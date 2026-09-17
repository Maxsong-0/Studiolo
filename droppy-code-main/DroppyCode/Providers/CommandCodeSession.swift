import Foundation

/// Drives Command Code (`cmd`) in its headless print mode.
///
/// The CLI has no long-lived server protocol: each turn is one `cmd -p` process with
/// `--output-format json`, the prompt piped on stdin, that streams newline-delimited
/// event frames and ends with one result line. The first turn's `run_start` names the
/// session, and every later turn resumes it with `--resume`, so a thread's conversation
/// survives relaunches in `~/.commandcode/projects`.
///
/// Headless runs cannot prompt, so the CLI is given `--yolo` and Droppy Code gates the
/// tools itself through a session mod (`CommandCodeAPI.approvalMod`, loaded with `--mod`):
/// every tool that could change something lands as a request file in the run's approval
/// directory, the app answers with a reply file, and the mod blocks the call or lets it
/// through. Supervised asks for all of them, Auto-accept edits waves file writes through,
/// Auto also passes shell commands that only read, and Full access loads no gate at all.
/// Plan mode is the CLI's own read-only mode, with the run's final text as the plan.
///
/// Protocol reference: https://commandcode.ai/docs/headless and /docs/mods. Verified
/// live against `command-code 1.53.1`: the event frames, stdin prompts, the result line,
/// the mod gate under `--yolo`, `--list-models`, `cmd status`, exit code 130 on a
/// terminated run, and the `/alpha/billing` routes behind the usage panel.
@MainActor
final class CommandCodeSession: ProviderSession {
    var onEvent: ((ProviderEvent) -> Void)?

    private let configuration: SessionConfiguration
    private var process: StdioProcess?
    private var readTask: Task<Void, Never>?
    private var sessionID: String?
    private var started = false
    private var stopped = false
    private var runtimeMode: RuntimeMode
    private var interactionMode: InteractionMode
    private var model: String?
    private var effort: String?

    private var turnActive = false
    private var interruptRequested = false
    private var isFinalReport = false
    /// Namespaces ids per launch, so a resumed thread never streams into an old entry.
    private let launchID = String(UUID().uuidString.prefix(8))
    private var turnCounter = 0
    private var messageCounter = 0
    private var thinkingCounter = 0
    private var currentMessageID: String?
    private var currentThinkingID: String?
    private var messageText = ""
    private var thinkingText = ""
    private var streamedText = false
    /// The model the CLI reported for the run, for the context meter's ceiling.
    private var activeModel: String?
    /// Files a `write_file` is about to replace, read as the call is queued, so the
    /// row's diff is the file before against the content written.
    private var editSnapshots: [String: String] = [:]
    private var toolNames: [String: String] = [:]
    private var toolInputs: [String: JSONValue] = [:]

    /// The result line, kept until the process is gone so the turn closes in order.
    private struct RunResult {
        var subtype: String
        var stopReason: String?
        var error: String?
    }

    private var runResult: RunResult?
    private var runError: String?

    /// The gate: where the mod and the run's request files live.
    private var modURL: URL?
    private var approvalDirectory: URL?
    private var approvalWatcher: DispatchSourceFileSystemObject?
    private var pendingApprovals: [String: ApprovalRequest] = [:]
    /// Kinds approved for the rest of the session ("Approve for session").
    private var sessionGrants: Set<String> = []

    /// Round trips one headless run may make before the CLI stops it. The CLI's own
    /// default is 100, short for a long agentic job.
    private static let maxTurns = 400

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
        interactionMode = configuration.interactionMode
        model = configuration.model
        effort = configuration.effort
        sessionID = configuration.resumeID.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// A session is a conversation, not a process: it stays up between turns and each
    /// turn spawns the CLI anew.
    var isRunning: Bool { started && !stopped }

    private var workingDirectory: String { configuration.workingDirectory.path }

    // MARK: - Lifecycle

    func start() async throws -> String {
        guard configuration.executable != nil else { throw ProviderError.notInstalled(configuration.provider) }
        if let sessionID, !Self.transcriptExists(sessionID, environment: configuration.environment) {
            throw ProviderError.failed("Command Code no longer has this conversation; starting a new one.")
        }
        modURL = try CommandCodeAPI.installMod()
        let directory = CommandCodeAPI.supportDirectory
            .appendingPathComponent("approvals", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        approvalDirectory = directory
        startWatcher(directory)
        started = true
        stopped = false
        return sessionID ?? ""
    }

    /// `~/.commandcode/projects/<project>/<session>.jsonl`, the transcript a resume reads.
    /// A session the CLI no longer has (cleared, or another Mac's) starts over instead
    /// of failing every turn.
    private static func transcriptExists(_ sessionID: String, environment: [String: String]) -> Bool {
        let home = environment["HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? LoginEnvironment.homeDirectory
        let projects = URL(fileURLWithPath: home).appendingPathComponent(".commandcode/projects", isDirectory: true)
        let fileManager = FileManager.default
        guard let folders = try? fileManager.contentsOfDirectory(at: projects, includingPropertiesForKeys: nil) else { return false }
        return folders.contains { fileManager.fileExists(atPath: $0.appendingPathComponent("\(sessionID).jsonl").path) }
    }

    func send(_ input: TurnInput) async throws {
        guard isRunning, let executable = configuration.executable else { throw ProviderError.notRunning }
        guard !turnActive else { throw ProviderError.failed("Command Code is still on the last turn.") }
        runtimeMode = input.runtimeMode
        interactionMode = input.interactionMode
        if let model = input.model, !model.isEmpty { self.model = model }
        effort = input.effort
        isFinalReport = input.isFinalReport
        turnCounter += 1
        turnActive = true
        interruptRequested = false
        runResult = nil
        runError = nil
        streamedText = false
        editSnapshots.removeAll()
        toolNames.removeAll()
        toolInputs.removeAll()

        var arguments = [
            "-p",
            "--output-format", "json",
            "--skip-onboarding",
            "--no-auto-update",
            "--trust",
            "--tools-enable", "todo_write",
            "--max-turns", String(isFinalReport ? 1 : Self.maxTurns),
        ]
        if let model, !model.isEmpty { arguments += ["--model", model] }
        if let effort, !effort.isEmpty, Self.takesEffort(effort, model: model) { arguments += ["--effort", effort] }
        if interactionMode == .plan {
            arguments += ["--permission-mode", "plan"]
        } else {
            // Print mode withholds writes and commands unless told otherwise; the mod
            // below is what asks the user.
            arguments += ["--yolo"]
        }
        if let modURL, runtimeMode != .fullAccess || interactionMode == .plan {
            arguments += ["--mod", modURL.path]
        }
        if let sessionID, !sessionID.isEmpty { arguments += ["--resume", sessionID] }

        var environment = configuration.environment
        if runtimeMode != .fullAccess, let approvalDirectory {
            environment["DROPPY_CODE_APPROVALS"] = approvalDirectory.path
        } else {
            environment["DROPPY_CODE_APPROVALS"] = nil
        }

        let process = StdioProcess(
            executable: executable,
            arguments: arguments,
            directory: configuration.workingDirectory,
            environment: environment
        )
        self.process = process
        do {
            try process.start()
        } catch {
            turnActive = false
            throw ProviderError.failed("Command Code could not start: \(error.localizedDescription)")
        }
        process.write(Data(Self.prompt(for: input).utf8))
        process.closeInput()
        onEvent?(.turnStarted(providerTurnID: nil))

        let messages = process.messages
        readTask = Task { [weak self] in
            for await message in messages {
                guard let self else { return }
                self.handle(message)
            }
            let status = await process.waitForExit()
            self?.didClose(status: status, process: process)
        }
    }

    /// The user's words, with any attachments named for `read_file`, which reads images.
    private static func prompt(for input: TurnInput) -> String {
        var text = input.text
        let files = input.images.filter { FileManager.default.fileExists(atPath: $0.path) }
        if !files.isEmpty {
            text += "\n\nAttached files (read them with read_file):\n"
            text += files.map { "- \($0.path)" }.joined(separator: "\n")
        }
        return text
    }

    /// Whether the effort is one the model takes, per the CLI's table. The CLI refuses a
    /// run outright for a model with no adjustable effort ("LongCat 2.0 has no adjustable
    /// reasoning effort", exit 1), so an effort the table does not list for the model
    /// stays off the command line.
    private static func takesEffort(_ effort: String, model: String?) -> Bool {
        guard let model else { return false }
        return CommandCodeAPI.efforts(for: model).contains(effort)
    }

    func interrupt() async {
        guard turnActive else { return }
        interruptRequested = true
        // SIGTERM ends the run at once (exit 130); the CLI has already persisted every
        // turn it finished, so the next `--resume` picks up from there.
        process?.terminate()
    }

    func compact() async throws {
        throw ProviderError.failed("Command Code compacts its context on its own.")
    }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let request = pendingApprovals.removeValue(forKey: requestID) else { return }
        switch optionID {
        case "allow", "always":
            if optionID == "always" { sessionGrants.insert(Self.grantKey(request.kind)) }
            reply(to: requestID, allow: true)
        default:
            reply(to: requestID, allow: false)
        }
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {}

    func stop() {
        stopped = true
        turnActive = false
        process?.terminate()
        stopWatcher()
        if let approvalDirectory {
            try? FileManager.default.removeItem(at: approvalDirectory)
        }
    }

    // MARK: - Stream

    private func handle(_ message: JSONValue) {
        switch message["type"]?.string {
        case "event":
            handleEvent(message["event"] ?? .null)
        case "result":
            runResult = RunResult(
                subtype: message["subtype"]?.string ?? "success",
                stopReason: message["stopReason"]?.string,
                error: Self.errorText(message["error"])
            )
            if let id = message["sessionId"]?.string, !id.isEmpty, id != sessionID {
                sessionID = id
                onEvent?(.sessionReady(sessionID: id))
            }
        default:
            break
        }
    }

    private func handleEvent(_ event: JSONValue) {
        switch event["type"]?.string {
        case "run_start":
            if let id = event["sessionId"]?.string, !id.isEmpty, id != sessionID {
                sessionID = id
                onEvent?(.sessionReady(sessionID: id))
            }
        case "message_start":
            messageCounter += 1
            currentMessageID = "cc-\(launchID)-m\(messageCounter)"
            messageText = ""
            streamedText = false
        case "text_delta":
            guard let delta = event["delta"]?.string, !delta.isEmpty else { return }
            if currentMessageID == nil {
                messageCounter += 1
                currentMessageID = "cc-\(launchID)-m\(messageCounter)"
            }
            streamedText = true
            messageText += delta
            onEvent?(.messageDelta(id: currentMessageID!, text: delta))
        case "thinking_start":
            thinkingCounter += 1
            currentThinkingID = "cc-\(launchID)-r\(thinkingCounter)"
            thinkingText = ""
        case "thinking_delta":
            guard let delta = event["delta"]?.string, !delta.isEmpty else { return }
            if currentThinkingID == nil {
                thinkingCounter += 1
                currentThinkingID = "cc-\(launchID)-r\(thinkingCounter)"
            }
            thinkingText += delta
            onEvent?(.reasoningDelta(id: currentThinkingID!, text: delta))
        case "thinking_end":
            guard let id = currentThinkingID else { return }
            let text = event["text"]?.string ?? thinkingText
            if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                onEvent?(.reasoningCompleted(id: id, text: text))
            }
            currentThinkingID = nil
        case "message_end":
            let text = Self.text(in: event["content"])
            if let id = currentMessageID {
                if streamedText {
                    onEvent?(.messageCompleted(id: id, text: messageText.isEmpty ? text : messageText))
                } else if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    // A reply that arrived whole rather than as deltas.
                    onEvent?(.messageCompleted(id: id, text: text))
                }
            }
            currentMessageID = nil
            currentThinkingID = nil
        case "model_request_start":
            if let model = event["model"]?.string { activeModel = model }
        case "model_request_end":
            if let model = event["model"]?.string { activeModel = model }
            let usage = event["usage"] ?? .null
            let input = usage["inputTokens"]?.int ?? 0
            let output = usage["outputTokens"]?.int ?? 0
            let used = input + output
            if used > 0 {
                onEvent?(.usage(ContextUsage(usedTokens: used, windowTokens: CommandCodeAPI.contextWindow(for: activeModel ?? model))))
                TokenLedger.shared.record(spend: used)
            }
        case "tool_queued":
            guard let id = event["toolCallId"]?.string, let name = event["toolName"]?.string else { return }
            let input = event["input"] ?? .null
            toolNames[id] = name
            toolInputs[id] = input
            if name == "todo_write" {
                onEvent?(.todos(Self.todos(from: input)))
                return
            }
            if name == "write_file", let path = input["file_path"]?.string {
                editSnapshots[id] = Self.snapshot(path) ?? ""
            }
            onEvent?(.toolStarted(id: id, call: makeToolCall(name: name, input: input, snapshot: editSnapshots[id])))
            // The gate's request may already be on disk by the time the row exists.
            scanApprovals()
        case "tool_update":
            guard let id = event["toolCallId"]?.string, toolNames[id] != "todo_write" else { return }
            let text = Self.text(in: event["partial"])
            if !text.isEmpty { onEvent?(.toolUpdated(id: id, update: ToolUpdate(output: text))) }
        case "tool_completed":
            guard let id = event["toolCallId"]?.string, toolNames[id] != "todo_write" else { return }
            var update = ToolUpdate(status: .completed)
            let text = Self.text(in: event["result"])
            if !text.isEmpty { update.output = text }
            update.edits = finishEdit(id)
            onEvent?(.toolUpdated(id: id, update: update))
        case "tool_errored":
            guard let id = event["toolCallId"]?.string, toolNames[id] != "todo_write" else { return }
            let error = Self.errorText(event["error"]) ?? "The tool failed."
            onEvent?(.toolUpdated(id: id, update: ToolUpdate(output: error, status: .failed, edits: finishEdit(id))))
        case "tool_hook_blocked", "tool_denied":
            guard let id = event["toolCallId"]?.string, toolNames[id] != "todo_write" else { return }
            let reason = event["hookOutput"]?.string ?? event["denyMessage"]?.string ?? "Declined."
            editSnapshots[id] = nil
            onEvent?(.toolUpdated(id: id, update: ToolUpdate(output: reason, status: .declined)))
            if pendingApprovals.removeValue(forKey: id) != nil { onEvent?(.requestResolved(id: id)) }
        case "subagent_progress":
            guard let id = event["toolCallId"]?.string, let tool = event["toolName"]?.string else { return }
            let input = event["toolInput"] ?? .null
            let subject = input["command"]?.string ?? input["file_path"]?.string ?? input["pattern"]?.string ?? input["query"]?.string ?? input["url"]?.string
            let line = subject.map { "\(tool)  \(TextCleanup.singleLine($0, limit: 100))" } ?? tool
            onEvent?(.toolOutput(id: id, text: line + "\n"))
        case "session_titled":
            if let title = event["title"]?.string, !title.isEmpty { onEvent?(.title(title)) }
        case "compaction_start":
            onEvent?(.notice(Notice(level: .info, message: "Compacting context.")))
        case "compaction_done":
            let saved = event["tokensSaved"]?.int ?? 0
            let message = saved > 0 ? "Compacted context, freeing \(saved.formatted()) tokens." : "Compacted context."
            onEvent?(.notice(Notice(level: .info, message: message)))
        case "notice":
            guard let message = event["message"]?.string, !message.isEmpty else { return }
            let level: Notice.Level = switch event["level"]?.string {
            case "error": .error
            case "warning", "warn": .warning
            default: .info
            }
            onEvent?(.notice(Notice(level: level, message: message)))
        case "run_error":
            runError = Self.errorText(event["error"])
        case "interrupted":
            interruptRequested = true
        case "run_end":
            let result = event["result"] ?? .null
            let finalText = result["finalText"]?.string ?? ""
            if interactionMode == .plan, !finalText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                onEvent?(.planCompleted(id: "cc-\(launchID)-plan\(turnCounter)", markdown: finalText))
            }
        default:
            // turn_start, turn_end, message_update, model_trace, tool_running, tool_hooks,
            // tool_input_repaired, subagent_start and subagent_stop carry nothing more
            // than what the frames above already said.
            break
        }
    }

    private func didClose(status: Int32, process: StdioProcess) {
        guard self.process === process else { return }
        self.process = nil
        turnActive = false
        currentMessageID = nil
        currentThinkingID = nil
        for id in pendingApprovals.keys { onEvent?(.requestResolved(id: id)) }
        pendingApprovals.removeAll()
        clearApprovalFiles()
        guard !stopped else { return }

        if interruptRequested || status == 130 {
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
            return
        }
        if let runResult {
            switch runResult.subtype {
            case "success":
                if runResult.stopReason == "max_turns" { noteTurnCap() }
                onEvent?(.turnCompleted(status: .completed, error: nil))
            case "max_turns":
                noteTurnCap()
                onEvent?(.turnCompleted(status: .completed, error: nil))
            default:
                let error = runResult.error ?? runError ?? TextCleanup.lastLines(process.errorTail) ?? "Command Code stopped before finishing."
                if Self.isRateLimit(error) { onEvent?(.usageLimit(resetsAt: nil)) }
                onEvent?(.turnCompleted(status: .failed, error: Self.friendlyError(error)))
            }
            return
        }
        // No result line: the CLI died before it could write one. Its exit codes say why.
        let tail = TextCleanup.lastLines(process.errorTail)
        switch status {
        case 0:
            onEvent?(.turnCompleted(status: .completed, error: nil))
        case 3:
            onEvent?(.turnCompleted(status: .failed, error: "Command Code is not signed in. Run `cmd login` in Terminal, or add a Studio API key in Settings → Providers → Command Code."))
        case 4:
            onEvent?(.turnCompleted(status: .failed, error: tail ?? "Command Code was denied permission."))
        case 5:
            onEvent?(.usageLimit(resetsAt: nil))
            onEvent?(.turnCompleted(status: .failed, error: tail ?? "Command Code is rate limited. Wait for the window to reset, or buy on-demand credits, which are never throttled."))
        case 6:
            onEvent?(.turnCompleted(status: .failed, error: "Could not reach Command Code. Check the connection and try again."))
        case 7:
            onEvent?(.turnCompleted(status: .failed, error: tail ?? "Command Code's API returned a server error. Try again in a moment."))
        case 8:
            noteTurnCap()
            onEvent?(.turnCompleted(status: .completed, error: nil))
        case 9:
            onEvent?(.turnCompleted(status: .failed, error: "The model produced no response."))
        case 10:
            onEvent?(.turnCompleted(status: .failed, error: "Command Code is out of credits. Top up at commandcode.ai/billing, or wait for the monthly credits to reset."))
        default:
            onEvent?(.turnCompleted(status: .failed, error: runError ?? tail ?? "Command Code exited with code \(status)."))
        }
    }

    private func noteTurnCap() {
        guard !isFinalReport else { return }
        onEvent?(.notice(Notice(level: .warning, message: "Command Code stopped at its cap of \(Self.maxTurns) round trips for one message. Send another to carry on.")))
    }

    private static func isRateLimit(_ text: String) -> Bool {
        let lowered = text.lowercased()
        return lowered.contains("rate limit") || lowered.contains("usage exceeded") || lowered.contains("window limit")
    }

    /// The CLI's wording for the account errors, made actionable.
    private static func friendlyError(_ text: String) -> String {
        let lowered = text.lowercased()
        if lowered.contains("not authenticated") || lowered.contains("unauthorized") || lowered.contains("session expired") {
            return "Command Code is not signed in. Run `cmd login` in Terminal, or add a Studio API key in Settings → Providers → Command Code."
        }
        if lowered.contains("insufficient credits") || lowered.contains("out of credits") {
            return "Command Code is out of credits. Top up at commandcode.ai/billing, or wait for the monthly credits to reset."
        }
        return text
    }

    private static func errorText(_ value: JSONValue?) -> String? {
        guard let value, !value.isNull else { return nil }
        if let text = value.string { return text.isEmpty ? nil : text }
        if let message = value["message"]?.string, !message.isEmpty { return message }
        let compact = value.compactString
        return compact == "{}" ? nil : compact
    }

    /// The text of a content-block array, as tool results and messages carry it.
    private static func text(in value: JSONValue?) -> String {
        guard let value, !value.isNull else { return "" }
        if let text = value.string { return text }
        guard let blocks = value.array else { return "" }
        return blocks.compactMap { block -> String? in
            if let text = block["text"]?.string { return text }
            if block["type"]?.string == "image" { return "[image]" }
            return nil
        }.joined(separator: "\n")
    }

    private static func todos(from input: JSONValue) -> [TodoStep] {
        (input["todos"]?.array ?? []).compactMap { todo in
            guard let text = todo["content"]?.string ?? todo["text"]?.string ?? todo["title"]?.string else { return nil }
            let status: TodoStep.Status = switch todo["status"]?.string {
            case "completed", "done": .done
            case "in_progress", "active": .active
            default: .pending
            }
            return TodoStep(text: text, status: status)
        }
    }

    // MARK: - Tools

    private func makeToolCall(name: String, input: JSONValue, snapshot: String?) -> ToolCall {
        func path(_ key: String) -> String? {
            input[key]?.string.map { ToolTitles.relativePath($0, to: workingDirectory) }
        }
        switch name {
        case "shell_command", "powershell":
            let command = input["command"]?.string.map(ToolTitles.unwrapShell) ?? "Shell command"
            var call = ToolCall(kind: .command, title: command, detail: input["description"]?.string)
            if input["run_in_background"]?.bool == true { call.detail = [call.detail, "in the background"].compactMap { $0 }.joined(separator: " · ") }
            return call
        case "monitor_command":
            return ToolCall(kind: .command, title: input["command"]?.string.map(ToolTitles.unwrapShell) ?? "Monitor command")
        case "shell_output":
            return ToolCall(kind: .command, title: "Shell output", detail: input["id"]?.string ?? input["task_id"]?.string)
        case "shell_tasks":
            return ToolCall(kind: .command, title: "Shell tasks")
        case "kill_shell":
            return ToolCall(kind: .command, title: "Stop shell", detail: input["id"]?.string ?? input["task_id"]?.string ?? input["pid"]?.int.map(String.init))
        case "read_file":
            var call = ToolCall(kind: .read, title: path("file_path") ?? path("path") ?? "Read file")
            if let offset = input["offset"]?.int, let limit = input["limit"]?.int {
                call.detail = "lines \(offset)–\(offset + limit)"
            }
            return call
        case "read_directory":
            return ToolCall(kind: .search, title: path("path") ?? "List files")
        case "write_file":
            var call = ToolCall(kind: .edit, title: path("file_path") ?? "Write file")
            if let file = path("file_path"), let content = input["content"]?.string {
                call.edits = [FileEdit(path: file, old: snapshot ?? "", new: content)]
            }
            return call
        case "edit_file":
            var call = ToolCall(kind: .edit, title: path("file_path") ?? path("path") ?? "Edit file")
            if let file = path("file_path") ?? path("path") {
                let old = input["old_string"]?.string ?? input["old_text"]?.string ?? input["old"]?.string ?? ""
                let new = input["new_string"]?.string ?? input["new_text"]?.string ?? input["new"]?.string ?? ""
                call.edits = [FileEdit(path: file, old: old, new: new)]
            }
            return call
        case "glob":
            return ToolCall(kind: .search, title: input["pattern"]?.string ?? "Find files", detail: path("path"))
        case "grep":
            let scope = [path("path"), input["glob"]?.string].compactMap { $0 }.joined(separator: " · ")
            return ToolCall(kind: .search, title: input["pattern"]?.string ?? "Search", detail: scope.isEmpty ? nil : scope)
        case "web_fetch":
            return ToolCall(kind: .web, title: input["url"]?.string ?? "Fetch page")
        case "web_search":
            return ToolCall(kind: .web, title: input["query"]?.string ?? "Search the web")
        case "agent":
            let description = input["description"]?.string ?? input["prompt"]?.string.map { TextCleanup.singleLine($0, limit: 96) } ?? "Sub-agent"
            return ToolCall(kind: .agent, title: description, detail: input["subagent_type"]?.string)
        case "agent_output":
            return ToolCall(kind: .agent, title: "Agent output", detail: input["agent_id"]?.string)
        case "activate_skill":
            return ToolCall(kind: .other, title: "Skill · \(input["name"]?.string ?? "")")
        case "enter_worktree", "exit_worktree":
            return ToolCall(kind: .other, title: Self.humanize(name), detail: input["name"]?.string)
        case "run_command":
            return ToolCall(kind: .other, title: "Command · \(input["command"]?.string ?? input["name"]?.string ?? "")")
        default:
            if name.hasPrefix("mcp__") {
                let parts = name.split(separator: "__", omittingEmptySubsequences: true)
                let server = parts.count > 1 ? String(parts[1]) : "MCP"
                let tool = parts.count > 2 ? parts[2...].joined(separator: "__") : name
                return ToolCall(kind: .mcp, title: "\(server) · \(tool)", detail: input.isNull ? nil : input.compactString)
            }
            var call = ToolCall(kind: .other, title: Self.humanize(name))
            let detail = input.isNull ? nil : input.compactString
            if let detail, !detail.isEmpty, detail != "{}", detail.count <= 300 { call.detail = detail }
            return call
        }
    }

    /// "Task create" from `task_create`.
    private static func humanize(_ name: String) -> String {
        let words = name.split(separator: "_").map(String.init)
        guard let first = words.first else { return name }
        return ([first.prefix(1).uppercased() + first.dropFirst()] + words.dropFirst()).joined(separator: " ")
    }

    /// A finished write as the file before against after, from the snapshot taken as the
    /// call was queued; nil for anything else, so the row's edits stay as queued.
    private func finishEdit(_ id: String) -> [FileEdit]? {
        guard let before = editSnapshots.removeValue(forKey: id),
              let path = toolInputs[id]?["file_path"]?.string else { return nil }
        let relative = ToolTitles.relativePath(path, to: workingDirectory)
        let after = Self.snapshot(path) ?? ""
        guard before != after else { return [FileEdit(path: relative)] }
        return [FileEdit(path: relative, old: before, new: after)]
    }

    /// Text files up to 1 MB; anything else (binary, huge, missing) reads as empty so a
    /// write shows as a whole-file addition.
    private static func snapshot(_ path: String) -> String? {
        let url = URL(fileURLWithPath: path)
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 1_000_000 else { return nil }
        return try? String(contentsOf: url, encoding: .utf8)
    }

    // MARK: - The gate

    private func startWatcher(_ directory: URL) {
        stopWatcher()
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: [.write], queue: .main)
        source.setEventHandler { [weak self] in
            MainActor.assumeIsolated { self?.scanApprovals() }
        }
        source.setCancelHandler { close(descriptor) }
        source.resume()
        approvalWatcher = source
    }

    private func stopWatcher() {
        approvalWatcher?.cancel()
        approvalWatcher = nil
    }

    /// Picks up every request the mod has written and either answers it from the mode
    /// and the session's grants or asks the user.
    private func scanApprovals() {
        guard let approvalDirectory, turnActive else { return }
        let fileManager = FileManager.default
        guard let files = try? fileManager.contentsOfDirectory(at: approvalDirectory, includingPropertiesForKeys: nil) else { return }
        for file in files.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) where file.lastPathComponent.hasSuffix(".request.json") {
            guard let data = try? Data(contentsOf: file), let request = JSONValue.parse(data) else { continue }
            try? fileManager.removeItem(at: file)
            let id = request["toolCallId"]?.string ?? String(file.lastPathComponent.dropLast(".request.json".count))
            guard pendingApprovals[id] == nil else { continue }
            decide(id: id, request: request)
        }
    }

    private func decide(id: String, request: JSONValue) {
        let name = request["toolName"]?.string ?? ""
        let input = request["input"] ?? .null
        let approval = makeApproval(id: id, name: name, input: input)
        if autoApproves(name: name, input: input) || sessionGrants.contains(Self.grantKey(approval.kind)) {
            reply(to: id, allow: true)
            return
        }
        pendingApprovals[id] = approval
        onEvent?(.approval(approval))
    }

    /// What the mode lets through without asking. Full access never reaches here: it
    /// loads no gate.
    private func autoApproves(name: String, input: JSONValue) -> Bool {
        let isEdit = name == "write_file" || name == "edit_file"
        switch runtimeMode {
        case .fullAccess:
            return true
        case .supervised:
            return false
        case .autoAcceptEdits:
            return isEdit
        case .auto:
            if isEdit { return true }
            switch name {
            case "shell_command", "powershell", "monitor_command":
                return !ShellCommandKind.mayWriteFiles(input["command"]?.string ?? "")
            case "kill_shell", "cron_create", "cron_delete", "task_stop":
                return true
            default:
                return false
            }
        }
    }

    private func makeApproval(id: String, name: String, input: JSONValue) -> ApprovalRequest {
        let call = makeToolCall(name: name, input: input, snapshot: editSnapshots[id])
        let kind: ApprovalRequest.Kind = switch call.kind {
        case .command: .command
        case .edit: .fileChange
        default: .tool
        }
        let options: [ApprovalRequest.Option] = [
            .init(id: "allow", title: "Approve", role: .approve),
            .init(id: "always", title: "Approve for session", role: .approveAlways),
            .init(id: "deny", title: "Decline", role: .decline),
        ]
        return ApprovalRequest(
            id: id,
            kind: kind,
            title: call.title,
            detail: call.detail,
            options: options,
            toolItemID: toolNames[id] != nil ? id : nil
        )
    }

    private static func grantKey(_ kind: ApprovalRequest.Kind) -> String {
        switch kind {
        case .command: "command"
        case .fileChange: "fileChange"
        case .tool, .permissions, .plan: "tool"
        }
    }

    private func reply(to id: String, allow: Bool) {
        guard let approvalDirectory else { return }
        let reply: JSONValue = allow
            ? ["decision": "allow"]
            : ["decision": "deny", "reason": "The user declined this in Droppy Code. Do not retry it; take another approach or explain what you would need."]
        let file = approvalDirectory.appendingPathComponent("\(id).reply.json")
        let temporary = approvalDirectory.appendingPathComponent("\(id).reply.json.tmp")
        do {
            try reply.data().write(to: temporary, options: .atomic)
            _ = try FileManager.default.replaceItemAt(file, withItemAt: temporary)
        } catch {
            try? reply.data().write(to: file, options: .atomic)
        }
    }

    /// Requests and replies a finished run left behind, so a stale one never answers a
    /// later call.
    private func clearApprovalFiles() {
        guard let approvalDirectory,
              let files = try? FileManager.default.contentsOfDirectory(at: approvalDirectory, includingPropertiesForKeys: nil) else { return }
        for file in files { try? FileManager.default.removeItem(at: file) }
    }
}
