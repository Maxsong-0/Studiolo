import Foundation

/// Drives the Pi coding harness (`pi`) in RPC mode, its long-lived JSON protocol.
///
/// One `pi --mode rpc` process per thread, strict JSONL over stdin/stdout
/// (LF-delimited, one JSON object per line). Prompts arrive as `prompt` requests
/// and stream back as `message_start` / `message_update` / `message_end` events;
/// tool approvals come as `extension_ui_request` server requests and are answered
/// with `extension_ui_response`. The session file pi reports in `get_state` is the
/// resume id, so a thread's conversation survives relaunches in `~/.pi/agent`.
///
/// Pi itself keeps approving everything (`--approve`) or nothing (`--no-approve`),
/// so Droppy Code gates the tools itself through a pi extension (`PiCLI`) loaded
/// with `-e`: every tool that could change something lands as a `droppy-gate`
/// select request, which the app answers or asks the user. Supervised asks for all
/// of them, Auto-accept edits waves file changes through, Auto also passes shell
/// commands that only read, and Full access returns before asking. Plan mode is
/// read-only from the extension's own prompt, with nothing extra loaded.
@MainActor
final class PiSession: ProviderSession {
    var onEvent: ((ProviderEvent) -> Void)?

    private let configuration: SessionConfiguration
    private var process: StdioProcess?
    private var readTask: Task<Void, Never>?
    /// The pi session file path, taken from `configuration.resumeID`.
    private var sessionID: String?
    private var started = false
    private var stopped = false
    private var runtimeMode: RuntimeMode
    private var interactionMode: InteractionMode
    private var model: String?
    private var effort: String?

    private var turnActive = false
    private var interruptRequested = false
    /// Namespaces ids per launch, so a resumed thread never streams into an old entry.
    private let launchID = String(UUID().uuidString.prefix(8))
    private var messageCounter = 0
    private var thinkingCounter = 0
    private var currentMessageID: String?
    private var currentThinkingID: String?
    private var messageText = ""
    private var thinkingText = ""
    /// The model's context ceiling, from the session state's model.
    private var contextWindow: Int?
    private var gateURL: URL?
    private var pendingApprovals: [String: ApprovalRequest] = [:]
    /// `extension_ui_request` ids by method, for the app's questions.
    private var pendingQuestions: [String: String] = [:]
    /// Gate kinds approved for the rest of the session ("Allow for session").
    private var sessionGrants: Set<String> = []
    private var toolNames: [String: String] = [:]
    private var toolInputs: [String: JSONValue] = [:]
    /// Files a `write` is about to replace, read as the call starts, so the row's
    /// diff is the file before against the content written.
    private var editSnapshots: [String: String] = [:]
    private var turnError: String?
    /// The turn's last reply, which plan mode hands over as the plan.
    private var lastReplyText = ""
    private var turnCounter = 0
    private var pendingResponses: [String: CheckedContinuation<JSONValue, Never>] = [:]
    private var requestCounter = 0

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
        interactionMode = configuration.interactionMode
        model = configuration.model
        effort = configuration.effort
        sessionID = configuration.resumeID.flatMap { $0.isEmpty ? nil : $0 }
    }

    var isRunning: Bool { started && !stopped && process?.isRunning == true }

    private var workingDirectory: String { configuration.workingDirectory.path }

    // MARK: - Lifecycle

    func start() async throws -> String {
        guard let executable = configuration.executable else { throw ProviderError.notInstalled(configuration.provider) }
        if let sessionID, !FileManager.default.fileExists(atPath: sessionID) {
            throw ProviderError.failed("Pi no longer has this conversation; starting a new one.")
        }
        let extensionURL = try PiCLI.installExtension()
        let gateDirectory = PiCLI.supportDirectory.appendingPathComponent("gates", isDirectory: true)
        try FileManager.default.createDirectory(at: gateDirectory, withIntermediateDirectories: true)
        let gateURL = gateDirectory.appendingPathComponent("\(UUID().uuidString).json")
        self.gateURL = gateURL
        writeGate(finalReport: false)

        var arguments = ["--mode", "rpc", "-e", extensionURL.path]
        arguments.append(runtimeMode == .auto || runtimeMode == .fullAccess ? "--approve" : "--no-approve")
        if let model, !model.isEmpty, model != "default", let slash = model.firstIndex(of: "/") {
            arguments += ["--provider", String(model[..<slash]), "--model", String(model[model.index(after: slash)...])]
        }
        if let effort, !effort.isEmpty, PiCLI.thinkingLevels.contains(effort) {
            arguments += ["--thinking", effort]
        }
        if let sessionID { arguments += ["--session", sessionID] }

        var environment = configuration.environment
        environment["DROPPY_CODE_PI_GATE"] = gateURL.path
        environment["PI_SKIP_VERSION_CHECK"] = "1"

        let process = StdioProcess(
            executable: executable,
            arguments: arguments,
            directory: configuration.workingDirectory,
            environment: environment,
            framing: .lines
        )
        do {
            try process.start()
        } catch {
            throw ProviderError.failed("Pi could not start: \(error.localizedDescription)")
        }
        self.process = process
        let messages = process.messages
        readTask = Task { [weak self] in
            for await message in messages {
                guard let self else { return }
                self.handle(message)
            }
            let status = await process.waitForExit()
            self?.didClose(status: status)
        }

        let state = await request(["type": .string("get_state")])
        let data = state["data"] ?? state
        if let file = data["sessionFile"]?.string, !file.isEmpty {
            self.sessionID = file
        } else if let id = data["sessionId"]?.string, !id.isEmpty {
            self.sessionID = id
        }
        contextWindow = data["model"]?["contextWindow"]?.int
        onEvent?(.sessionReady(sessionID: self.sessionID ?? ""))

        let commandsResponse = await request(["type": .string("get_commands")])
        let commandsData = commandsResponse["data"] ?? commandsResponse
        if let entries = commandsData["commands"]?.array {
            let commands = entries.compactMap { entry -> SlashCommand? in
                guard let name = entry["name"]?.string else { return nil }
                return SlashCommand(name: name, detail: entry["description"]?.string ?? "")
            }
            if !commands.isEmpty { onEvent?(.commands(commands)) }
        }
        started = true
        stopped = false
        return self.sessionID ?? ""
    }

    func send(_ input: TurnInput) async throws {
        guard isRunning, let process else { throw ProviderError.notRunning }
        guard !turnActive else { throw ProviderError.failed("Pi is still on the last turn.") }
        runtimeMode = input.runtimeMode
        interactionMode = input.interactionMode
        writeGate(finalReport: input.isFinalReport)
        if let wanted = input.model, !wanted.isEmpty, wanted != model, let slash = wanted.firstIndex(of: "/") {
            process.send(["type": .string("set_model"), "provider": .string(String(wanted[..<slash])), "modelId": .string(String(wanted[wanted.index(after: slash)...]))])
            model = wanted
        }
        if let wanted = input.effort, !wanted.isEmpty, wanted != effort, PiCLI.thinkingLevels.contains(wanted) {
            process.send(["type": .string("set_thinking_level"), "level": .string(wanted)])
            effort = wanted
        }
        turnActive = true
        interruptRequested = false
        turnError = nil
        turnCounter += 1
        lastReplyText = ""
        toolNames.removeAll()
        toolInputs.removeAll()
        editSnapshots.removeAll()
        var prompt: JSONValue = ["type": .string("prompt"), "message": .string(input.text)]
        let images: [JSONValue] = input.images.compactMap { image in
            guard image.isImage, let data = try? Data(contentsOf: image.url) else { return nil }
            return ["type": "image", "data": .string(data.base64EncodedString()), "mimeType": .string(image.mimeType)]
        }
        if !images.isEmpty {
            if case .object(var fields) = prompt {
                fields["images"] = .array(images)
                prompt = .object(fields)
            }
        }
        process.send(prompt)
        onEvent?(.turnStarted(providerTurnID: nil))
    }

    func interrupt() async {
        interruptRequested = true
        process?.send(["type": .string("abort")])
    }

    func compact() async throws {
        guard isRunning, let process else { throw ProviderError.notRunning }
        process.send(["type": .string("compact")])
    }

    func stop() {
        stopped = true
        process?.terminate()
        process = nil
        readTask?.cancel()
        readTask = nil
        if let gateURL {
            try? FileManager.default.removeItem(at: gateURL)
            self.gateURL = nil
        }
    }

    func stopAgent(_ id: String) async -> Bool { false }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let request = pendingApprovals.removeValue(forKey: requestID) else { return }
        let value: String
        switch request.options.first(where: { $0.id == optionID })?.role {
        case .approveAlways:
            value = "Allow for session"
            sessionGrants.insert(Self.grantKey(request.kind))
        case .decline, .cancel, nil:
            value = "Deny"
        case .approve:
            value = "Allow"
        }
        process?.send(["type": .string("extension_ui_response"), "id": .string(requestID), "value": .string(value)])
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {
        guard let method = pendingQuestions.removeValue(forKey: requestID) else { return }
        let texts = answers.values.flatMap { $0 }
        if texts.isEmpty {
            process?.send(["type": .string("extension_ui_response"), "id": .string(requestID), "cancelled": true])
        } else if method == "confirm" {
            process?.send(["type": .string("extension_ui_response"), "id": .string(requestID), "confirmed": .bool(texts.first == "Yes")])
        } else {
            process?.send(["type": .string("extension_ui_response"), "id": .string(requestID), "value": .string(texts.first ?? "")])
        }
        onEvent?(.requestResolved(id: requestID))
    }

    // MARK: - Requests

    /// Writes a request line with a fresh id and resumes with the `response` that
    /// carries it, or `.null` after 30 seconds.
    private func request(_ fields: [String: JSONValue]) async -> JSONValue {
        guard let process else { return .null }
        requestCounter += 1
        let id = "droppy-\(requestCounter)"
        var line = fields
        line["id"] = .string(id)
        let value = await withCheckedContinuation { (continuation: CheckedContinuation<JSONValue, Never>) in
            pendingResponses[id] = continuation
            process.send(.object(line))
            Task {
                try? await Task.sleep(for: .seconds(30))
                if let continuation = self.pendingResponses.removeValue(forKey: id) {
                    continuation.resume(returning: .null)
                }
            }
        }
        return value
    }

    /// The gate the extension reads before every tool call.
    private func writeGate(finalReport: Bool) {
        guard let gateURL else { return }
        let gate: JSONValue = [
            "runtimeMode": .string(runtimeMode.rawValue),
            "interactionMode": .string(interactionMode.rawValue),
            "finalReport": .bool(finalReport),
            "systemPrompt": "",
        ]
        try? gate.data().write(to: gateURL, options: .atomic)
    }

    private static func grantKey(_ kind: ApprovalRequest.Kind) -> String {
        switch kind {
        case .command: "command"
        case .fileChange: "fileChange"
        case .tool, .permissions, .plan: "tool"
        }
    }

    // MARK: - Stream

    private func handle(_ message: JSONValue) {
        switch message["type"]?.string {
        case "response":
            guard let id = message["id"]?.string else { return }
            if let continuation = pendingResponses.removeValue(forKey: id) {
                continuation.resume(returning: message)
            } else if message["command"]?.string == "prompt", message["success"]?.bool == false {
                turnError = message["error"]?.string ?? message["message"]?.string ?? "Pi reported an error."
                finishTurn()
            }
        case "message_start":
            guard message["message"]?["role"]?.string == "assistant" else { return }
            messageCounter += 1
            currentMessageID = "\(launchID)-m-\(messageCounter)"
            messageText = ""
            thinkingText = ""
            currentThinkingID = nil
        case "message_update":
            handleUpdate(message)
        case "message_end":
            handleMessageEnd(message["message"] ?? message)
        case "tool_execution_start":
            handleToolStart(message)
        case "tool_execution_update":
            guard let id = message["toolCallId"]?.string else { return }
            let text = Self.text(in: message["partialResult"]?["content"])
            if !text.isEmpty { onEvent?(.toolOutput(id: id, text: text)) }
        case "tool_execution_end":
            handleToolEnd(message)
        case "agent_end":
            finishTurn()
        case "compaction_start":
            onEvent?(.notice(Notice(level: .info, message: "Compacting the conversation…")))
        case "compaction_end":
            onEvent?(.notice(Notice(level: .info, message: "Conversation compacted")))
        case "auto_retry_start":
            onEvent?(.notice(Notice(level: .warning, message: message["error"]?.string ?? "Pi is retrying.")))
        case "extension_error":
            onEvent?(.notice(Notice(level: .error, message: message["error"]?.string ?? "Pi reported an extension error.")))
        case "extension_ui_request":
            handleExtensionUI(message)
        default:
            break
        }
    }

    private func handleUpdate(_ message: JSONValue) {
        let event = message["assistantMessageEvent"] ?? message["event"] ?? .null
        switch event["type"]?.string {
        case "text_delta":
            guard let delta = event["delta"]?.string, !delta.isEmpty else { return }
            if currentMessageID == nil {
                messageCounter += 1
                currentMessageID = "\(launchID)-m-\(messageCounter)"
            }
            messageText += delta
            onEvent?(.messageDelta(id: currentMessageID!, text: delta))
        case "thinking_delta":
            guard let delta = event["delta"]?.string, !delta.isEmpty else { return }
            if currentThinkingID == nil {
                thinkingCounter += 1
                currentThinkingID = "\(launchID)-t-\(thinkingCounter)"
                thinkingText = ""
            }
            thinkingText += delta
            onEvent?(.reasoningDelta(id: currentThinkingID!, text: delta))
        default:
            break
        }
        let usage = message["usage"] ?? event["usage"] ?? .null
        let total = usage["totalTokens"]?.int
        let summed = (usage["input"]?.int ?? usage["inputTokens"]?.int ?? 0)
            + (usage["output"]?.int ?? usage["outputTokens"]?.int ?? 0)
            + (usage["cacheRead"]?.int ?? usage["cacheReadTokens"]?.int ?? 0)
            + (usage["cacheWrite"]?.int ?? usage["cacheWriteTokens"]?.int ?? 0)
        let used = total ?? summed
        if used > 0 { onEvent?(.usage(ContextUsage(usedTokens: used, windowTokens: contextWindow))) }
    }

    private func handleMessageEnd(_ message: JSONValue) {
        guard message["role"]?.string == "assistant" else { return }
        var finalText = messageText
        if let blocks = message["content"]?.array {
            let joined = blocks.compactMap { block -> String? in
                guard block["type"]?.string == "text" else { return nil }
                return block["text"]?.string
            }.joined()
            if !joined.isEmpty { finalText = joined }
        }
        if let id = currentThinkingID, !thinkingText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            onEvent?(.reasoningCompleted(id: id, text: thinkingText))
        }
        currentThinkingID = nil
        if let id = currentMessageID {
            onEvent?(.messageCompleted(id: id, text: finalText))
            lastReplyText = finalText
        }
        currentMessageID = nil
        if message["stopReason"]?.string == "error" {
            turnError = message["errorMessage"]?.string ?? "Pi reported an error."
        }
    }

    private func handleToolStart(_ message: JSONValue) {
        guard let id = message["toolCallId"]?.string, let name = message["toolName"]?.string else { return }
        let args = message["args"] ?? message["input"] ?? .null
        toolNames[id] = name
        toolInputs[id] = args
        if name == "write", let path = args["path"]?.string {
            editSnapshots[id] = Self.snapshot(path) ?? ""
        }
        let kind: ToolCall.Kind = switch name {
        case "bash": .command
        case "read": .read
        case "edit", "write": .edit
        case "grep", "find", "ls": .search
        default: .other
        }
        let title: String
        let detail: String?
        switch name {
        case "bash":
            title = ToolTitles.unwrapShell(args["command"]?.string ?? "bash")
            detail = nil
        case "read", "edit", "write":
            title = ToolTitles.relativePath(args["path"]?.string ?? name, to: workingDirectory)
            detail = nil
        case "grep", "find":
            title = args["pattern"]?.string ?? name
            detail = nil
        case "ls":
            title = args["path"]?.string ?? "."
            detail = nil
        default:
            title = name
            detail = args.isNull || args.compactString == "{}" ? nil : args.prettyString
        }
        onEvent?(.toolStarted(id: id, call: ToolCall(kind: kind, title: title, detail: detail)))
    }

    private func handleToolEnd(_ message: JSONValue) {
        guard let id = message["toolCallId"]?.string else { return }
        let result = message["result"] ?? .null
        let output = Self.text(in: result["content"])
        let isError = message["isError"]?.bool == true || result["isError"]?.bool == true
        // A call the gate blocked on the user's word comes back as an error whose text is
        // the gate's reason; the row reads as declined rather than failed.
        let status: ToolCall.Status = !isError ? .completed
            : output.hasPrefix("The user declined this") ? .declined : .failed
        let exitCode = result["details"]?["exitCode"]?.int
        var edits: [FileEdit]? = nil
        if let name = toolNames[id], let args = toolInputs[id] {
            edits = editDiff(name: name, args: args, id: id)
        }
        onEvent?(.toolUpdated(id: id, update: ToolUpdate(output: output.isEmpty ? nil : output, status: status, exitCode: exitCode, edits: edits)))
    }

    /// A minimal unified diff of the old text against the new: a `---`/`+++` header,
    /// then the old lines prefixed with `-` and the new lines prefixed with `+`.
    private func editDiff(name: String, args: JSONValue, id: String) -> [FileEdit]? {
        switch name {
        case "edit":
            guard let path = args["path"]?.string else { return nil }
            let old = args["oldText"]?.string ?? ""
            let new = args["newText"]?.string ?? ""
            return [Self.unifiedDiff(path: path, old: old, new: new)]
        case "write":
            guard let path = args["path"]?.string, let content = args["content"]?.string else { return nil }
            let before = editSnapshots.removeValue(forKey: id) ?? ""
            return [Self.unifiedDiff(path: path, old: before, new: content)]
        default:
            return nil
        }
    }

    private static func unifiedDiff(path: String, old: String, new: String) -> FileEdit {
        let relative = path.hasPrefix("/") ? path : path
        let header = "--- a/\(relative)\n+++ b/\(relative)"
        let body = old.components(separatedBy: "\n").filter { !$0.isEmpty }.map { "-\($0)" }
            + new.components(separatedBy: "\n").filter { !$0.isEmpty }.map { "+\($0)" }
        let diff = ([header] + body).joined(separator: "\n")
        let stats = ToolTitles.diffStats(diff)
        return FileEdit(path: relative, diff: diff, additions: stats.additions, deletions: stats.deletions)
    }

    private static func snapshot(_ path: String) -> String? {
        let url = URL(fileURLWithPath: path)
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 1_000_000 else { return nil }
        return try? String(contentsOf: url, encoding: .utf8)
    }

    /// The text of a content-block array, as tool results carry it.
    private static func text(in value: JSONValue?) -> String {
        guard let value, !value.isNull else { return "" }
        if let text = value.string { return text }
        guard let blocks = value.array else { return "" }
        return blocks.compactMap { block -> String? in
            if let text = block["text"]?.string { return text }
            return nil
        }.joined(separator: "\n")
    }

    private func finishTurn() {
        guard turnActive else { return }
        turnActive = false
        let error = turnError
        turnError = nil
        // Plan mode is the gate's read-only turn: its final reply is the plan.
        if interactionMode == .plan, error == nil, !interruptRequested,
           !lastReplyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            onEvent?(.planCompleted(id: "\(launchID)-plan-\(turnCounter)", markdown: lastReplyText))
        }
        if interruptRequested {
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
        } else if let error {
            onEvent?(.turnCompleted(status: .failed, error: error))
        } else {
            onEvent?(.turnCompleted(status: .completed, error: nil))
        }
        interruptRequested = false
    }

    // MARK: - Extension UI

    private func handleExtensionUI(_ message: JSONValue) {
        guard let id = message["id"]?.string ?? message["requestId"]?.string,
              let method = message["method"]?.string else { return }
        let params = message["params"] ?? message
        switch method {
        case "select":
            handleSelect(id: id, params: params)
        case "confirm":
            let title = params["title"]?.string ?? "Pi"
            let prompt = [title, params["message"]?.string].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "\n")
            pendingQuestions[id] = "confirm"
            onEvent?(.question(QuestionRequest(id: id, questions: [
                QuestionRequest.Question(
                    id: id,
                    header: "Pi",
                    prompt: prompt,
                    choices: [QuestionRequest.Choice(label: "Yes", detail: nil), QuestionRequest.Choice(label: "No", detail: nil)],
                    allowsMultiple: false,
                    allowsOther: false,
                    isSecret: false
                ),
            ])))
        case "input", "editor":
            pendingQuestions[id] = method
            onEvent?(.question(QuestionRequest(id: id, questions: [
                QuestionRequest.Question(
                    id: id,
                    header: "Pi",
                    prompt: params["title"]?.string ?? "Pi",
                    choices: [],
                    allowsMultiple: false,
                    allowsOther: true,
                    isSecret: false
                ),
            ])))
        case "notify":
            let level: Notice.Level = switch params["notifyType"]?.string {
            case "warning": .warning
            case "error": .error
            default: .info
            }
            onEvent?(.notice(Notice(level: level, message: params["message"]?.string ?? "Pi sent a notification.")))
        case "setStatus", "setWidget", "setTitle", "set_editor_text":
            break
        default:
            break
        }
    }

    private func handleSelect(id: String, params: JSONValue) {
        let title = params["title"]?.string ?? ""
        let options = (params["options"]?.array ?? []).compactMap(\.string)
        guard title.hasPrefix("droppy-gate "), let payload = JSONValue.parse(String(title.dropFirst("droppy-gate ".count))) else {
            pendingQuestions[id] = "select"
            onEvent?(.question(QuestionRequest(id: id, questions: [
                QuestionRequest.Question(
                    id: id,
                    header: "Pi",
                    prompt: title,
                    choices: options.map { QuestionRequest.Choice(label: $0, detail: nil) },
                    allowsMultiple: false,
                    allowsOther: false,
                    isSecret: false
                ),
            ])))
            return
        }
        let kindString = payload["kind"]?.string ?? "tool"
        if sessionGrants.contains(kindString) {
            process?.send(["type": .string("extension_ui_response"), "id": .string(id), "value": .string("Allow")])
            return
        }
        let toolName = payload["toolName"]?.string ?? "tool"
        let input = payload["input"] ?? .null
        let kind: ApprovalRequest.Kind = kindString == "command" ? .command : kindString == "fileChange" ? .fileChange : .tool
        let callTitle: String
        let callDetail: String?
        switch kind {
        case .command:
            callTitle = ToolTitles.unwrapShell(input["command"]?.string ?? toolName)
            callDetail = input["command"]?.string
        case .fileChange:
            callTitle = ToolTitles.relativePath(input["path"]?.string ?? toolName, to: workingDirectory)
            callDetail = input["path"]?.string
        case .tool, .permissions, .plan:
            callTitle = toolName
            callDetail = input.isNull || input.compactString == "{}" ? nil : input.prettyString
        }
        pendingApprovals[id] = ApprovalRequest(
            id: id,
            kind: kind,
            title: callTitle,
            detail: callDetail,
            options: [
                .init(id: "allow", title: "Approve", role: .approve),
                .init(id: "always", title: "Approve for session", role: .approveAlways),
                .init(id: "deny", title: "Decline", role: .decline),
            ],
            toolItemID: payload["toolCallId"]?.string
        )
        onEvent?(.approval(pendingApprovals[id]!))
    }

    private func didClose(status: Int32) {
        let tail = TextCleanup.lastLines(process?.errorTail ?? "")
        process = nil
        if turnActive {
            turnActive = false
            onEvent?(.turnCompleted(status: interruptRequested ? .interrupted : .failed, error: tail.flatMap { $0.isEmpty ? nil : $0 }))
        }
        guard !stopped else { return }
        started = false
        if status == 0 {
            onEvent?(.exited(error: nil))
        } else {
            let suffix = (tail ?? "").isEmpty ? "" : "\n" + (tail ?? "")
            onEvent?(.exited(error: "Pi exited with status \(status)\(suffix)"))
        }
    }
}
