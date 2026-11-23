// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Drives Claude Code in headless stream-json mode, routing permission prompts to the app.
///
/// With Crew on, the session defines the hands as session agents (`--agents`), steers the
/// lead towards them with an appended system prompt, and asks for the hands' own text and
/// thinking (`--forward-subagent-text`). A hand's messages carry the spawning tool call in
/// `parent_tool_use_id`; each such stream keeps its own block bookkeeping, and its events go
/// out wrapped in `agentEvent`. The `task_started`, `task_progress` and `task_notification`
/// system messages bracket a hand's life.
@MainActor
final class ClaudeSession: ProviderSession {
    var onEvent: ((ProviderEvent) -> Void)?

    private struct PendingTool {
        var name: String
        var input: JSONValue
        var suggestions: JSONValue?
    }

    /// The open blocks of one streamed transcript: the lead's, or one hand's.
    private struct StreamState {
        var currentMessageID: String?
        var blockIDs: [Int: String] = [:]
        var openText: [String: [String]] = [:]
        var openThinking: [String: [String]] = [:]
    }

    /// What the lead's API calls have spent in the running turn, call by call, as the
    /// stream shows it. The CLI states a turn's usage only on its result, so a turn that
    /// works for many minutes (a hosted hand's, above all) would show no spend until it
    /// ended; each call's own usage streams as the call runs instead, and the result then
    /// settles the turn against its total.
    ///
    /// A call is known by its message id and reports more than once: `message_start`
    /// brings its input and first output tokens, each assistant message cut from it (one
    /// per content block) repeats that snapshot, not final, and `message_delta` brings the
    /// final counts. Every count only grows within a call, so the largest seen per field is
    /// the call's, and a repeat adds nothing.
    private struct TurnSpend {
        private struct Call {
            var input = 0
            var cacheCreation = 0
            var cacheRead = 0
            var output = 0
            /// The call's iterations summed, when the API split the one request into several
            /// (an advisor's answer or a compaction inside it); zero otherwise.
            var iterations = 0

            var total: Int { max(iterations, input + cacheCreation + cacheRead + output) }

            mutating func merge(_ usage: JSONValue) {
                input = max(input, usage["input_tokens"]?.int ?? 0)
                cacheCreation = max(cacheCreation, usage["cache_creation_input_tokens"]?.int ?? 0)
                cacheRead = max(cacheRead, usage["cache_read_input_tokens"]?.int ?? 0)
                output = max(output, usage["output_tokens"]?.int ?? 0)
                if let parts = usage["iterations"]?.array, !parts.isEmpty {
                    iterations = max(iterations, parts.reduce(0) { $0 + TurnSpend.tokens(in: $1) })
                }
            }
        }

        private var calls: [String: Call] = [:]
        /// The call the stream opened last: the one a `message_delta`, which names none, closes.
        var openCall: String?
        /// What has gone out as spend this turn.
        private var reported = 0

        /// The tokens one usage object counts, cache reads included, which is what Claude
        /// Code itself reports.
        static func tokens(in usage: JSONValue?) -> Int {
            (usage?["input_tokens"]?.int ?? 0)
                + (usage?["cache_read_input_tokens"]?.int ?? 0)
                + (usage?["cache_creation_input_tokens"]?.int ?? 0)
                + (usage?["output_tokens"]?.int ?? 0)
        }

        /// Takes in one usage report for a call and returns the spend it adds beyond what the
        /// call was already counted for: nothing, for a repeat.
        mutating func note(call id: String, usage: JSONValue?) -> Int {
            guard let usage, usage.object != nil else { return 0 }
            var call = calls[id] ?? Call()
            let before = call.total
            call.merge(usage)
            calls[id] = call
            let added = call.total - before
            reported += added
            return added
        }

        /// Closes the turn on its result: what the result's total holds beyond what the calls
        /// already reported, and a fresh start for the next turn. A total short of what
        /// streamed adds nothing and takes nothing back: the result keeps the iterations of
        /// its last call only (see `turnSpend`), and a call a stop cut off can reach the
        /// stream without reaching the result.
        mutating func settle(total: Int) -> Int {
            defer { self = TurnSpend() }
            return max(0, total - reported)
        }
    }

    private let configuration: SessionConfiguration
    private var process: StdioProcess?
    private var readTask: Task<Void, Never>?
    private var sessionID: String?
    private var permissionMode: String
    private var runtimeMode: RuntimeMode
    private var model: String?
    private var controlCounter = 0
    private var pendingControl: [String: CheckedContinuation<JSONValue, Error>] = [:]
    private var pendingTools: [String: PendingTool] = [:]
    private var toolNames: [String: String] = [:]
    private var mainStream = StreamState()
    /// The lead's spend in the running turn, call by call (see `TurnSpend`).
    private var leadSpend = TurnSpend()
    /// Each hand's stream, by the tool call that spawned it.
    private var agentStreams: [String: StreamState] = [:]
    /// Hands by their task id, for progress and completion, and their task ids by hand, for stopping.
    private var agentsByTask: [String: String] = [:]
    private var tasksByAgent: [String: String] = [:]
    private var turnActive = false
    private var interruptRequested = false
    private var isStopping = false
    /// Set once the CLI has answered the initialize. Before that, a result or an exit is
    /// the launch's failure, reported by `start()` throwing, never a turn's end.
    private var isStarted = false
    private var hasClosed = false
    /// What the CLI said as it failed to launch: the `errors` of a result that came
    /// before the initialize was answered, or the last of its stderr.
    private var launchError: String?
    private static var knownOptions: [String: Set<String>] = [:]

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
        permissionMode = Self.modeName(configuration.runtimeMode, configuration.interactionMode)
        model = configuration.model
    }

    var isRunning: Bool { process?.isRunning ?? false }

    private var workingDirectory: String { configuration.workingDirectory.path }

    /// The newer CLI options are only passed when `--help` lists them, so an older
    /// Claude Code that does not know them yet still launches. Read once per executable;
    /// a probe that fails is not remembered, so the next launch asks again.
    private static func knownOptions(of executable: URL, environment: [String: String]) async -> Set<String> {
        if let cached = knownOptions[executable.path] { return cached }
        guard let result = try? await Shell.run(executable, ["--help"], environment: environment, timeout: 20) else { return [] }
        let help = TextCleanup.stripANSI(result.output + result.errorOutput)
        let found = ["--system-prompt-snapshot", "--forward-subagent-text"].filter { help.contains($0) }
        let options = Set(found)
        knownOptions[executable.path] = options
        return options
    }

    func start() async throws -> String {
        guard let executable = configuration.executable else { throw ProviderError.notInstalled(configuration.provider) }
        let id = configuration.resumeID ?? UUID().uuidString.lowercased()
        var arguments = [
            "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--include-partial-messages", "--permission-prompt-tool", "stdio",
            "--setting-sources", "user,project,local",
            "--allow-dangerously-skip-permissions", "--permission-mode", permissionMode,
        ]
        if let model = configuration.model, model != "default" { arguments += ["--model", model] }
        if let effort = configuration.effort, !effort.isEmpty { arguments += ["--effort", effort] }
        if configuration.fastMode { arguments += ["--settings", #"{"fastMode":true}"#] }
        // Connected servers from Settings are the only MCP servers the session sees.
        // --strict-mcp-config leaves the user's own Claude config files alone but out of this session; with none connected, the flag is not passed and Claude reads its own config as usual.
        if let mcpConfig = MCPProviderConfig.claudeConfigPath() { arguments += ["--mcp-config", mcpConfig, "--strict-mcp-config"] }
        var environment = configuration.environment
        let options = await Self.knownOptions(of: executable, environment: environment)
        // Resumed chats must use the current setting, even with Crew off.
        if options.contains("--system-prompt-snapshot") { arguments += ["--system-prompt-snapshot", "off"] }
        var crewPolicy: String?
        if let crew = configuration.crew {
            // The system prompt is rendered fresh rather than replayed from the
            // conversation's first request, so switching Crew on for an existing chat
            // reaches the model.
            if crew.runsNatively {
                // The hands and the lead's brief.
                arguments += ["--agents", CrewPrompts.claudeAgents(crew).compactString]
                crewPolicy = CrewPrompts.policy(for: .claude, crew)
                if options.contains("--forward-subagent-text") { arguments.append("--forward-subagent-text") }
                // Uncapped, the CLI keeps its own limit on hands at once.
                if let cap = crew.maxHands { environment["CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS"] = String(cap) }
            } else {
                // The hands run on another provider, as threads Bottega starts: the
                // lead asks for them with the delegation block, and its own agent tool goes,
                // since a hand it spawned itself would run on its own model.
                crewPolicy = CrewPrompts.fallbackPolicy(crew)
                arguments += ["--disallowedTools", "Agent", "Task"]
            }
        }
        // Exactly one combined system prompt segment for every session, Crew or not. The
        // off notice goes out too, so a resumed thread cannot keep earlier guidance alive.
        arguments += [
            "--append-system-prompt",
            [crewPolicy, AntiSlopPolicy.instructions(enabled: configuration.antiSlopEnabled)]
                .compactMap { $0 }
                .joined(separator: "\n\n"),
        ]
        if let resumeID = configuration.resumeID {
            arguments += ["--resume", resumeID]
            if let anchor = configuration.resumeAt { arguments += ["--resume-session-at", anchor] }
        } else {
            arguments += ["--session-id", id]
        }

        let process = StdioProcess(
            executable: executable,
            arguments: arguments,
            directory: configuration.workingDirectory,
            environment: environment
        )
        self.process = process
        try process.start()
        let messages = process.messages
        readTask = Task { [weak self] in
            for await message in messages {
                guard let self else { return }
                self.handle(message)
            }
            self?.didClose()
        }

        sessionID = id
        var initialize: [String: JSONValue] = ["subtype": "initialize", "hooks": .null]
        if configuration.crew?.runsNatively == true {
            // One-line progress notes for the hands, on task_progress.
            initialize["agentProgressSummaries"] = true
            initialize["forwardSubagentText"] = true
        }
        let initialization = try await control(.object(initialize))
        isStarted = true
        let models = Self.models(in: initialization)
        if !models.isEmpty { onEvent?(.models(models, current: configuration.model)) }
        let commands = SlashCommand.claudeCommands(initialization["commands"]?.array ?? [])
        if !commands.isEmpty { onEvent?(.commands(commands)) }
        return id
    }

    /// The CLI's model rows. From 2.1.280 each row also names the model id its value
    /// resolves to (`resolvedModel`: `opus` → `claude-opus-5-5`), which an earlier CLI
    /// leaves out.
    private static func models(in initialization: JSONValue) -> [ModelOption] {
        (initialization["models"]?.array ?? []).compactMap { entry -> ModelOption? in
            guard let value = entry["value"]?.string else { return nil }
            return ModelOption(
                id: value,
                name: entry["displayName"]?.string ?? value,
                detail: entry["description"]?.string,
                efforts: (entry["supportedEffortLevels"]?.array ?? []).compactMap(\.string),
                isDefault: value == "default",
                fastTier: entry["supportsFastMode"]?.bool == true ? "fast" : nil,
                resolvedModel: entry["resolvedModel"]?.string?.nilIfEmpty
            )
        }
    }

    /// What the CLI reports in its handshake: its models, and the slash commands it finds
    /// for `directory`. A bare process, not a session: it gets the initialize and nothing else.
    static func handshake(executable: URL, directory: URL, environment: [String: String]) async throws -> (models: [ModelOption], commands: [SlashCommand]) {
        let process = StdioProcess(
            executable: executable,
            arguments: ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence", "--setting-sources", "user,project,local"],
            directory: directory,
            environment: environment
        )
        try process.start()
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(20))
            if !Task.isCancelled { process.terminate() }
        }
        defer { watchdog.cancel(); process.terminate() }
        process.send(["type": "control_request", "request_id": "handshake", "request": ["subtype": "initialize", "hooks": .null]])
        return try await withTaskCancellationHandler {
            for await message in process.messages {
                guard message["type"]?.string == "control_response",
                      message["response"]?["request_id"]?.string == "handshake" else { continue }
                guard let response = message["response"]?["response"], let commands = response["commands"]?.array else {
                    throw ProviderError.failed("Claude could not load its commands.")
                }
                return (models(in: response), SlashCommand.claudeCommands(commands))
            }
            throw ProviderError.failed("Claude's command discovery did not finish. Try again.")
        } onCancel: {
            process.terminate()
        }
    }

    func send(_ input: TurnInput) async throws {
        guard let process, process.isRunning else { throw ProviderError.notRunning }
        runtimeMode = input.runtimeMode
        let mode = Self.modeName(input.runtimeMode, input.interactionMode)
        if mode != permissionMode {
            _ = try? await control(["subtype": "set_permission_mode", "mode": .string(mode)])
            permissionMode = mode
        }
        if input.model != model {
            let wire = input.model == "default" ? nil : input.model
            _ = try? await control(["subtype": "set_model", "model": .optional(wire)])
            model = input.model
        }
        var content: [JSONValue] = []
        for (image, base64) in await Attachment.base64Images(input.images) {
            content.append([
                "type": "image",
                "source": ["type": "base64", "media_type": .string(image.mimeType), "data": .string(base64)],
            ])
        }
        content.append(["type": "text", "text": .string(input.text)])
        writeUserMessage(.array(content))
    }

    func interrupt() async {
        guard turnActive else { return }
        interruptRequested = true
        for requestID in pendingTools.keys {
            sendToolResponse(requestID, ["behavior": "deny", "message": "The user stopped the turn.", "interrupt": true])
            onEvent?(.requestResolved(id: requestID))
        }
        pendingTools.removeAll()
        _ = try? await control(["subtype": "interrupt"])
    }

    func compact() async throws {
        guard let process, process.isRunning else { throw ProviderError.notRunning }
        writeUserMessage("/compact")
    }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let pending = pendingTools.removeValue(forKey: requestID) else { return }
        if optionID == "allow" || optionID == "always" {
            var response: [String: JSONValue] = ["behavior": "allow", "updatedInput": pending.input]
            if optionID == "always", let suggestions = pending.suggestions?.array {
                response["updatedPermissions"] = .array(suggestions.map(Self.sessionScoped))
            }
            sendToolResponse(requestID, .object(response))
            if pending.name == "ExitPlanMode" {
                onEvent?(.modeChanged(.build))
                let mode = Self.modeName(runtimeMode, .build)
                permissionMode = mode
                Task { _ = try? await self.control(["subtype": "set_permission_mode", "mode": .string(mode)]) }
            }
        } else {
            let message = pending.name == "ExitPlanMode"
                ? "The user wants to keep planning. Wait for their feedback before changing anything."
                : "The user declined this action."
            sendToolResponse(requestID, ["behavior": "deny", "message": .string(message)])
        }
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {
        guard let pending = pendingTools.removeValue(forKey: requestID) else { return }
        var input = pending.input.object ?? [:]
        var mapped: [String: JSONValue] = [:]
        for (question, values) in answers {
            mapped[question] = .string(values.joined(separator: ", "))
        }
        input["answers"] = .object(mapped)
        sendToolResponse(requestID, ["behavior": "allow", "updatedInput": .object(input)])
        onEvent?(.requestResolved(id: requestID))
    }

    func stop() {
        isStopping = true
        process?.terminate()
    }

    func stopAgent(_ id: String) async -> Bool {
        guard let taskID = tasksByAgent[id] else { return false }
        return (try? await control(["subtype": "stop_task", "task_id": .string(taskID)])) != nil
    }

    // MARK: - Wire

    private func control(_ request: JSONValue) async throws -> JSONValue {
        guard let process else { throw ProviderError.notRunning }
        if hasClosed { throw ProviderError.failed(launchError ?? "Claude exited.") }
        controlCounter += 1
        let requestID = "bottega-\(controlCounter)"
        return try await withCheckedThrowingContinuation { continuation in
            pendingControl[requestID] = continuation
            process.send(["type": "control_request", "request_id": .string(requestID), "request": request])
        }
    }

    private func writeUserMessage(_ content: JSONValue) {
        turnActive = true
        interruptRequested = false
        onEvent?(.turnStarted(providerTurnID: nil))
        process?.send([
            "type": "user",
            "session_id": "",
            "message": ["role": "user", "content": content],
            "parent_tool_use_id": .null,
        ])
    }

    private func sendToolResponse(_ requestID: String, _ response: JSONValue) {
        process?.send([
            "type": "control_response",
            "response": ["subtype": "success", "request_id": .string(requestID), "response": response],
        ])
    }

    private func handle(_ message: JSONValue) {
        // A hand's message names the tool call that spawned it; the lead's carries null.
        let parent = message["parent_tool_use_id"]?.string
        switch message["type"]?.string {
        case "control_response":
            let response = message["response"] ?? .null
            guard let requestID = response["request_id"]?.string,
                  let continuation = pendingControl.removeValue(forKey: requestID) else { return }
            if response["subtype"]?.string == "error" {
                continuation.resume(throwing: ProviderError.failed(response["error"]?.string ?? "Claude rejected the request."))
            } else {
                continuation.resume(returning: response["response"] ?? .null)
            }
        case "control_request":
            handleControlRequest(message)
        case "control_cancel_request":
            if let requestID = message["request_id"]?.string, pendingTools.removeValue(forKey: requestID) != nil {
                onEvent?(.requestResolved(id: requestID))
            }
        case "system":
            handleSystem(message)
        case "stream_event":
            if let parent {
                // The sink hands the event on at once, and what it reaches can come back
                // into this session and read the same hands. The state is taken out,
                // worked on and put back, so nothing is held open across the callbacks.
                var stream = agentStreams[parent] ?? StreamState()
                handleStreamEvent(message["event"] ?? .null, stream: &stream, sink: agentSink(parent))
                agentStreams[parent] = stream
            } else {
                let event = message["event"] ?? .null
                handleStreamEvent(event, stream: &mainStream, sink: leadSink)
                noteLeadSpend(streamEvent: event)
            }
        case "assistant":
            if let parent {
                var stream = agentStreams[parent] ?? StreamState()
                handleAssistant(message, stream: &stream, sink: agentSink(parent))
                agentStreams[parent] = stream
            } else {
                handleAssistant(message, stream: &mainStream, sink: leadSink)
                if let body = message["message"], let call = body["id"]?.string {
                    noteLeadSpend(call: call, usage: body["usage"])
                }
            }
        case "user":
            handleToolResults(message, sink: parent.map(agentSink) ?? leadSink)
        case "result":
            handleResult(message)
        case "rate_limit_event":
            let info = message["rate_limit_info"] ?? .null
            if info["status"]?.string == "rejected" {
                let resetsAt = (info["resetsAt"]?.double ?? info["resetsAtSeconds"]?.double).map { Date(timeIntervalSince1970: $0) }
                let when = resetsAt.map { " It resets at \(UsageLimitSignal.format($0))." } ?? ""
                onEvent?(.notice(Notice(level: .warning, message: "Claude hit a usage limit.\(when)")))
                onEvent?(.usageLimit(resetsAt: resetsAt))
            }
        default:
            break
        }
    }

    private func handleSystem(_ message: JSONValue) {
        switch message["subtype"]?.string {
        case "init":
            if let id = message["session_id"]?.string, id != sessionID {
                sessionID = id
                onEvent?(.sessionReady(sessionID: id))
            }
        case "compact_boundary":
            onEvent?(.notice(Notice(level: .info, message: "Context compacted.")))
        case "task_started":
            handleTaskStarted(message)
        case "task_progress":
            guard let taskID = message["task_id"]?.string, let agentID = agentsByTask[taskID] else { return }
            let usage = message["usage"]
            onEvent?(.agentProgress(
                agentID: agentID,
                summary: message["summary"]?.string,
                lastTool: message["last_tool_name"]?.string,
                tokens: usage?["total_tokens"]?.int,
                toolCalls: usage?["tool_uses"]?.int
            ))
        case "task_notification":
            guard let taskID = message["task_id"]?.string, let agentID = agentsByTask[taskID] else { return }
            let status: TurnStatus = switch message["status"]?.string {
            case "failed": .failed
            case "stopped": .interrupted
            default: .completed
            }
            agentStreams[agentID] = nil
            onEvent?(.agentFinished(agentID: agentID, status: status, summary: message["summary"]?.string))
        default:
            break
        }
    }

    /// A subagent task: the hand it stands for is keyed by the spawning tool call, which
    /// its transcript messages carry. Background shell commands are tasks too and pass.
    private func handleTaskStarted(_ message: JSONValue) {
        guard let taskID = message["task_id"]?.string, message["skip_transcript"]?.bool != true else { return }
        let isAgent = message["subagent_type"]?.string != nil || message["task_type"]?.string == "local_agent"
        guard isAgent else { return }
        let toolUseID = message["tool_use_id"]?.string
        let agentID = toolUseID ?? taskID
        agentsByTask[taskID] = agentID
        tasksByAgent[agentID] = taskID
        onEvent?(.agentStarted(AgentSpawn(
            id: agentID,
            taskID: taskID,
            toolUseID: toolUseID,
            description: message["description"]?.string ?? "Subagent",
            prompt: message["prompt"]?.string,
            model: nil,
            profile: CrewPrompts.profileName(forAgentName: message["subagent_type"]?.string),
            isBackground: message["is_backgrounded"]?.bool ?? false
        )))
    }

    /// Where a stream's events go: straight out for the lead, wrapped for a hand. The
    /// lead's is made once, not once per message.
    private lazy var leadSink: (ProviderEvent) -> Void = { [weak self] event in self?.onEvent?(event) }

    private func agentSink(_ agentID: String) -> (ProviderEvent) -> Void {
        { [weak self] event in self?.onEvent?(.agentEvent(agentID: agentID, event)) }
    }

    private func handleStreamEvent(_ event: JSONValue, stream: inout StreamState, sink: (ProviderEvent) -> Void) {
        switch event["type"]?.string {
        case "message_start":
            stream.currentMessageID = event["message"]?["id"]?.string ?? UUID().uuidString
            stream.blockIDs.removeAll()
        case "content_block_start":
            guard let index = event["index"]?.int, let block = event["content_block"] else { return }
            let messageID = stream.currentMessageID ?? "message"
            let id = "\(messageID)-\(index)"
            switch block["type"]?.string {
            case "text":
                stream.blockIDs[index] = id
                stream.openText[messageID, default: []].append(id)
                sink(.messageDelta(id: id, text: block["text"]?.string ?? ""))
            case "thinking":
                stream.blockIDs[index] = id
                stream.openThinking[messageID, default: []].append(id)
                sink(.reasoningDelta(id: id, text: block["thinking"]?.string ?? ""))
            case "tool_use":
                guard let toolID = block["id"]?.string, let name = block["name"]?.string else { return }
                toolNames[toolID] = name
                if let call = toolCall(name: name, input: [:]) {
                    sink(.toolStarted(id: toolID, call: call))
                }
            default:
                break
            }
        case "content_block_delta":
            guard let index = event["index"]?.int, let id = stream.blockIDs[index], let delta = event["delta"] else { return }
            switch delta["type"]?.string {
            case "text_delta":
                if let text = delta["text"]?.string { sink(.messageDelta(id: id, text: text)) }
            case "thinking_delta":
                if let text = delta["thinking"]?.string { sink(.reasoningDelta(id: id, text: text)) }
            default:
                break
            }
        default:
            break
        }
    }

    /// Text Claude Code puts in the transcript itself, under the model name `<synthetic>`,
    /// where the model said nothing: "No response requested." for a turn it found
    /// unanswered when a session was resumed, "(no content)" for an empty reply.
    private static let syntheticModel = "<synthetic>"
    private static let syntheticPlaceholders: Set<String> = ["No response requested.", "(no content)"]

    private func handleAssistant(_ message: JSONValue, stream: inout StreamState, sink: (ProviderEvent) -> Void) {
        guard let body = message["message"], let content = body["content"]?.array else { return }
        // Claude Code's own bookkeeping is never the model's words: a resumed session whose
        // transcript ended on a user message (a turn stopped or rewound here) gets a
        // synthetic "No response requested." after it, and the chat drew that as a reply
        // from the lead. An API error the CLI wraps the same way keeps its text, as a
        // notice rather than a message; the placeholders say nothing and are dropped.
        if body["model"]?.string == Self.syntheticModel {
            let text = content.compactMap { $0["type"]?.string == "text" ? $0["text"]?.string : nil }
                .joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if message["isApiErrorMessage"]?.bool == true, !text.isEmpty, !Self.syntheticPlaceholders.contains(text) {
                sink(.notice(Notice(level: .warning, message: text)))
            }
            return
        }
        let messageID = body["id"]?.string ?? UUID().uuidString
        if let uuid = message["uuid"]?.string { sink(.assistantMessageID(uuid)) }
        for block in content {
            switch block["type"]?.string {
            case "text":
                let text = block["text"]?.string ?? ""
                let id = stream.openText[messageID]?.isEmpty == false ? stream.openText[messageID]!.removeFirst() : "\(messageID)-text-\(UUID().uuidString)"
                sink(.messageCompleted(id: id, text: text))
            case "thinking":
                let text = block["thinking"]?.string ?? ""
                let id = stream.openThinking[messageID]?.isEmpty == false ? stream.openThinking[messageID]!.removeFirst() : "\(messageID)-thinking-\(UUID().uuidString)"
                sink(.reasoningCompleted(id: id, text: text))
            case "tool_use":
                guard let toolID = block["id"]?.string, let name = block["name"]?.string else { continue }
                let input = block["input"] ?? [:]
                toolNames[toolID] = name
                if name == "TodoWrite" {
                    sink(.todos(Self.todos(from: input)))
                } else if let call = toolCall(name: name, input: input) {
                    sink(.toolStarted(id: toolID, call: call))
                    sink(.toolUpdated(id: toolID, update: ToolUpdate(title: call.title, detail: call.detail, edits: call.edits, kind: call.kind)))
                }
            default:
                break
            }
        }
    }

    private func handleToolResults(_ message: JSONValue, sink: (ProviderEvent) -> Void) {
        for block in message["message"]?["content"]?.array ?? [] where block["type"]?.string == "tool_result" {
            guard let toolID = block["tool_use_id"]?.string else { continue }
            // The result closes the call, so its name is not needed again.
            let name = toolNames.removeValue(forKey: toolID) ?? ""
            if Self.hiddenResultTools.contains(name) { continue }
            let isError = block["is_error"]?.bool ?? false
            let output = Self.resultText(block["content"])
            var status: ToolCall.Status = isError ? .failed : .completed
            if isError, output.localizedCaseInsensitiveContains("declined") || output.localizedCaseInsensitiveContains("doesn't want") {
                status = .declined
            }
            sink(.toolUpdated(id: toolID, update: ToolUpdate(output: output, status: status)))
        }
    }

    /// The lead's calls as the stream opens and closes them, for the turn's spend to count
    /// while the turn runs (see `TurnSpend`). A hand's calls stay out: the result's total is
    /// the lead's alone, and a hand's spend reaches its card from the hand's own reports.
    private func noteLeadSpend(streamEvent event: JSONValue) {
        switch event["type"]?.string {
        case "message_start":
            leadSpend.openCall = event["message"]?["id"]?.string
            if let call = leadSpend.openCall { noteLeadSpend(call: call, usage: event["message"]?["usage"]) }
        case "message_delta":
            if let call = leadSpend.openCall { noteLeadSpend(call: call, usage: event["usage"]) }
        default:
            break
        }
    }

    private func noteLeadSpend(call: String, usage: JSONValue?) {
        recordSpend(leadSpend.note(call: call, usage: usage))
    }

    /// Spend for the ledger, and for the turn, which keeps its own record of it as the other
    /// sessions' turns do, so what a lead or a hand on Claude spent can be read back
    /// afterwards, and read while it works.
    private func recordSpend(_ spend: Int) {
        guard spend > 0 else { return }
        TokenLedger.shared.record(spend: spend)
        onEvent?(.tokenSpend(TokenSpend(provider: configuration.provider, totalTokens: spend, model: model == "default" ? nil : model)))
    }

    private func handleResult(_ message: JSONValue) {
        // Before the initialize is answered, a result is the launch's failure, not a turn's:
        // a `--resume` of a conversation the CLI no longer has answers with one
        // (`error_during_execution`, no turns, its `errors` naming the session) and exits.
        // `start()` throws it from `didClose`; nothing was sent that could end here.
        guard isStarted else {
            let errors = (message["errors"]?.array ?? []).compactMap(\.string).joined(separator: "\n")
            if !errors.isEmpty { launchError = errors }
            return
        }
        turnActive = false
        if let usage = Self.contextUsage(message) { onEvent?(.usage(usage)) }
        // The result states the turn's total. What its calls already reported as they
        // streamed is not counted again: only what the stream left out is added.
        recordSpend(leadSpend.settle(total: Self.turnSpend(message)))
        for requestID in pendingTools.keys { onEvent?(.requestResolved(id: requestID)) }
        pendingTools.removeAll()
        // Every call of the turn has had its result; an interrupted one never will.
        toolNames.removeAll()
        mainStream = StreamState()
        // The hands' streams stay: a hand running in the background outlives the lead's
        // turn, and clearing its stream here made its next delta orphaned and dropped.
        // Each hand's stream goes with its own task_notification.
        let isError = message["is_error"]?.bool ?? (message["subtype"]?.string != "success")
        if interruptRequested {
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
        } else if isError {
            let errors = (message["errors"]?.array ?? []).compactMap(\.string).joined(separator: "\n")
            let text = message["result"]?.string ?? errors
            // The limit refusal may arrive as the result alone, without a rate limit event before it.
            if UsageLimitSignal.matches(text) { onEvent?(.usageLimit(resetsAt: UsageLimitSignal.resetTime(in: text))) }
            onEvent?(.turnCompleted(status: .failed, error: text.isEmpty ? "Claude stopped before finishing." : text))
        } else {
            onEvent?(.turnCompleted(status: .completed, error: nil))
        }
        interruptRequested = false
    }

    private func handleControlRequest(_ message: JSONValue) {
        guard let requestID = message["request_id"]?.string, let request = message["request"] else { return }
        guard request["subtype"]?.string == "can_use_tool" else {
            process?.send([
                "type": "control_response",
                "response": ["subtype": "error", "request_id": .string(requestID), "error": "Bottega does not support this request."],
            ])
            return
        }
        let name = request["tool_name"]?.string ?? "Tool"
        let input = request["input"] ?? [:]
        let toolUseID = request["tool_use_id"]?.string
        pendingTools[requestID] = PendingTool(name: name, input: input, suggestions: request["permission_suggestions"])

        switch name {
        case "AskUserQuestion":
            let questions = (input["questions"]?.array ?? []).compactMap { question -> QuestionRequest.Question? in
                guard let prompt = question["question"]?.string else { return nil }
                return QuestionRequest.Question(
                    id: prompt,
                    header: question["header"]?.string ?? "",
                    prompt: prompt,
                    choices: (question["options"]?.array ?? []).compactMap { option in
                        option["label"]?.string.map { QuestionRequest.Choice(label: $0, detail: option["description"]?.string) }
                    },
                    allowsMultiple: question["multiSelect"]?.bool ?? false,
                    allowsOther: true,
                    isSecret: false
                )
            }
            onEvent?(.question(QuestionRequest(id: requestID, questions: questions)))
        case "ExitPlanMode":
            onEvent?(.planCompleted(id: toolUseID ?? requestID, markdown: input["plan"]?.string ?? ""))
            onEvent?(.approval(ApprovalRequest(
                id: requestID,
                kind: .plan,
                title: "Implement this plan",
                options: [
                    .init(id: "allow", title: "Implement plan", role: .approve),
                    .init(id: "deny", title: "Keep planning", role: .decline),
                ],
                toolItemID: toolUseID
            )))
        default:
            // The card shows the kind, title and detail; the edits' diffs are built once,
            // when the assistant message lands.
            let call = toolCall(name: name, input: input, withEdits: false) ?? ToolCall(kind: .other, title: name)
            let kind: ApprovalRequest.Kind = switch call.kind {
            case .command: .command
            case .edit: .fileChange
            default: .tool
            }
            var options: [ApprovalRequest.Option] = [.init(id: "allow", title: "Approve", role: .approve)]
            if let suggestions = request["permission_suggestions"]?.array, !suggestions.isEmpty {
                options.append(.init(id: "always", title: "Approve for session", role: .approveAlways))
            }
            options.append(.init(id: "deny", title: "Decline", role: .decline))
            onEvent?(.approval(ApprovalRequest(
                id: requestID,
                kind: kind,
                title: call.title,
                detail: call.kind == .command ? request["description"]?.string : call.detail,
                reason: request["decision_reason"]?.string,
                options: options,
                toolItemID: toolUseID
            )))
        }
    }

    private func didClose() {
        hasClosed = true
        let tail = process?.errorTail ?? ""
        // The result's own error names the failure better than the stderr tail can.
        let reason = launchError ?? TextCleanup.lastLines(tail)
        launchError = reason
        let waiting = Array(pendingControl.values)
        pendingControl.removeAll()
        for continuation in waiting {
            continuation.resume(throwing: ProviderError.failed(reason ?? "Claude exited."))
        }
        for requestID in pendingTools.keys { onEvent?(.requestResolved(id: requestID)) }
        pendingTools.removeAll()
        turnActive = false
        // No result is coming to settle the turn: what its calls streamed stays counted.
        leadSpend = TurnSpend()
        // A launch that failed is reported by `start()` throwing; only a session that
        // ran says it exited.
        guard !isStopping, isStarted else { return }
        onEvent?(.exited(error: TextCleanup.lastLines(tail)))
    }

    // MARK: - Mapping

    /// Tools whose results are shown by the app itself, never as a tool row.
    private static let hiddenResultTools: Set<String> = ["AskUserQuestion", "ExitPlanMode", "EnterPlanMode", "TodoWrite", "ToolSearch"]

    private func toolCall(name: String, input: JSONValue, withEdits: Bool = true) -> ToolCall? {
        func path(_ key: String) -> String? {
            input[key]?.string.map { ToolTitles.relativePath($0, to: workingDirectory) }
        }
        switch name {
        case "TodoWrite", "AskUserQuestion", "ExitPlanMode", "EnterPlanMode", "ToolSearch":
            return nil
        case "Bash":
            return ToolCall(kind: .command, title: input["command"]?.string ?? "Shell command", detail: input["description"]?.string)
        case "BashOutput", "KillShell", "KillBash", "Monitor", "TaskOutput", "TaskStop":
            return ToolCall(kind: .command, title: name)
        case "Read":
            return ToolCall(kind: .read, title: path("file_path") ?? "Read file")
        case "Write":
            var call = ToolCall(kind: .edit, title: path("file_path") ?? "Write file")
            if withEdits, let file = path("file_path"), let content = input["content"]?.string {
                call.edits = [FileEdit(path: file, old: "", new: content)]
            }
            return call
        case "Edit":
            var call = ToolCall(kind: .edit, title: path("file_path") ?? "Edit file")
            if withEdits, let file = path("file_path") {
                call.edits = [FileEdit(path: file, old: input["old_string"]?.string ?? "", new: input["new_string"]?.string ?? "")]
            }
            return call
        case "MultiEdit":
            var call = ToolCall(kind: .edit, title: path("file_path") ?? "Edit file")
            if withEdits, let file = path("file_path") {
                call.edits = (input["edits"]?.array ?? []).map {
                    FileEdit(path: file, old: $0["old_string"]?.string ?? "", new: $0["new_string"]?.string ?? "")
                }
            }
            return call
        case "NotebookEdit":
            return ToolCall(kind: .edit, title: path("notebook_path") ?? "Edit notebook")
        case "Glob":
            return ToolCall(kind: .search, title: input["pattern"]?.string ?? "Find files", detail: path("path"))
        case "Grep":
            return ToolCall(kind: .search, title: input["pattern"]?.string ?? "Search", detail: path("path"))
        case "LS":
            return ToolCall(kind: .search, title: path("path") ?? "List files")
        case "WebFetch":
            return ToolCall(kind: .web, title: input["url"]?.string ?? "Fetch page")
        case "WebSearch":
            return ToolCall(kind: .web, title: input["query"]?.string ?? "Search the web")
        case "Task", "Agent":
            return ToolCall(kind: .agent, title: input["description"]?.string ?? "Subagent", detail: input["subagent_type"]?.string)
        case "Skill":
            return ToolCall(kind: .other, title: "Skill · \(input["skill"]?.string ?? input["command"]?.string ?? "")")
        default:
            if name.hasPrefix("mcp__") {
                let parts = name.split(separator: "__", omittingEmptySubsequences: true)
                let server = parts.count > 1 ? String(parts[1]) : "MCP"
                let tool = parts.count > 2 ? parts[2...].joined(separator: "__") : name
                return ToolCall(kind: .mcp, title: "\(server) · \(tool)", detail: input.isNull ? nil : input.compactString)
            }
            return ToolCall(kind: .other, title: name)
        }
    }

    private static func todos(from input: JSONValue) -> [TodoStep] {
        (input["todos"]?.array ?? []).compactMap { todo in
            guard let text = todo["content"]?.string else { return nil }
            let status: TodoStep.Status = switch todo["status"]?.string {
            case "completed": .done
            case "in_progress": .active
            default: .pending
            }
            return TodoStep(text: text, status: status)
        }
    }

    private static func resultText(_ content: JSONValue?) -> String {
        guard let content else { return "" }
        if let text = content.string { return text }
        return (content.array ?? []).compactMap { $0["text"]?.string }.joined(separator: "\n")
    }

    /// The tokens the turn sent and received across its API calls, cache reads
    /// included, which is what Claude Code itself reports. The CLI adds its calls' counts
    /// up but keeps the iterations of the last call alone (the parts the API split one
    /// request into), so their sum is the turn's whole only when the turn made one call;
    /// the larger of the two stands for the turn either way.
    private static func turnSpend(_ message: JSONValue) -> Int {
        guard let usage = message["usage"] else { return 0 }
        let iterations = (usage["iterations"]?.array ?? []).reduce(0) { $0 + TurnSpend.tokens(in: $1) }
        return max(iterations, TurnSpend.tokens(in: usage))
    }

    private static func contextUsage(_ message: JSONValue) -> ContextUsage? {
        guard let usage = message["usage"] else { return nil }
        let last = usage["iterations"]?.array?.last ?? usage
        let used = (last["input_tokens"]?.int ?? 0)
            + (last["cache_read_input_tokens"]?.int ?? 0)
            + (last["cache_creation_input_tokens"]?.int ?? 0)
            + (last["output_tokens"]?.int ?? 0)
        guard used > 0 else { return nil }
        let window = message["modelUsage"]?.object?.values.compactMap { $0["contextWindow"]?.int }.max()
        return ContextUsage(usedTokens: used, windowTokens: window)
    }

    private static func sessionScoped(_ suggestion: JSONValue) -> JSONValue {
        guard var object = suggestion.object else { return suggestion }
        object["destination"] = "session"
        return .object(object)
    }

    static func modeName(_ runtime: RuntimeMode, _ interaction: InteractionMode) -> String {
        if interaction == .plan { return "plan" }
        switch runtime {
        case .supervised: return "default"
        case .autoAcceptEdits: return "acceptEdits"
        case .auto: return "auto"
        case .fullAccess: return "bypassPermissions"
        }
    }
}
