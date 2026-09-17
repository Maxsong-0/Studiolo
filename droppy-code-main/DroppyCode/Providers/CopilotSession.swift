import Foundation

/// Drives the GitHub Copilot CLI (`copilot`) in headless server mode, the JSON-RPC
/// protocol its SDK speaks.
///
/// One `copilot --headless --stdio` process per thread, `Content-Length` framed.
/// Sessions are created under Droppy Code's own id and resumed by it, so a
/// thread's conversation survives relaunches in `~/.copilot/session-state`.
/// Turns arrive as `session.event` notifications. Permission prompts come as
/// `permission.requested` events and are answered over
/// `session.permissions.handlePendingPermissionRequest`; questions
/// (`userInput.request`) and plan exits (`exitPlanMode.request`) are server
/// requests answered in place.
///
/// With Hydra on, the session is created with the heads as custom agents on the pair's
/// model and effort, the lead's brief appended to the system message, and sub-agent
/// streaming switched on. A head's events carry its `agentId` and go out wrapped in
/// `agentEvent`; `subagent.started`, `subagent.completed` and `subagent.failed` bracket
/// its life. Copilot offers no way to stop one head on its own.
///
/// Protocol reference: https://github.com/github/copilot-sdk, `nodejs/src/client.ts`
/// and the generated `rpc.ts` and `session-events.ts`. Verified live against
/// `copilot 1.0.83` (protocol version 3): framing, ping, account auth, session
/// create, resume and delete, commands, agent and permission modes, model and
/// effort switching, rewind points, compaction and the event envelope. Model
/// turns need a GitHub account with Copilot access.
@MainActor
final class CopilotSession: ProviderSession {
    var onEvent: ((ProviderEvent) -> Void)?

    private enum PendingRequest {
        /// A `permission.requested` event, answered by request id.
        case permission(request: JSONValue, toolCallID: String?)
        /// A `userInput.request` server request from the `ask_user` tool.
        case question(RPCID, choices: [String])
        /// An `exitPlanMode.request` server request from the `exit_plan_mode` tool.
        case planExit(RPCID)
    }

    private let configuration: SessionConfiguration
    private var connection: JSONRPCConnection?
    private var sessionID: String?
    private var runtimeMode: RuntimeMode
    private var interactionMode: InteractionMode = .build
    /// The CLI's own permission mode: `manual`, `assisted` or `allow-all`.
    private var permissionMode = "manual"
    private var model: String?
    private var effort: String?
    private var pendingRequests: [String: PendingRequest] = [:]
    private var turnActive = false
    private var turnError: String?
    private var interruptRequested = false
    private var isStopping = false
    /// Tool names by call id, so a permission or a result knows what it belongs to.
    private var toolNames: [String: String] = [:]
    /// Tools the user declined, so their failed result reads as declined.
    private var declinedTools: Set<String> = []
    /// Messages that streamed deltas, so an empty final message still ends the row.
    private var streamedMessages: Set<String> = []
    /// Reasoning already shown from `assistant.reasoning` events (hashed, trimmed), so
    /// the copy a message carries in `reasoningText` is not shown a second time.
    private var shownReasoning: Set<Int> = []
    /// The lead's event sink, made once rather than once per event.
    private lazy var leadSink: (ProviderEvent) -> Void = { [weak self] event in self?.onEvent?(event) }
    /// The CLI's slash commands and their aliases, which `send` invokes instead of prompting with.
    private var commandNames: Set<String> = []
    /// The brief each `task` call gave its head, by call id, so a head that starts can be
    /// named by what it was sent to do.
    private var taskBriefs: [String: (description: String, prompt: String?)] = [:]

    /// Commands Droppy Code answers with its own controls: permission modes, the model
    /// picker, plan mode, the session list and the working directory.
    private static let ownCommands: Set<String> = ["allow-all", "yolo", "permissions", "model", "models", "plan", "session", "sessions", "cwd", "cd"]

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
        model = configuration.model
        effort = configuration.effort
    }

    var isRunning: Bool {
        guard let connection else { return false }
        return !connection.isClosed
    }

    private var workingDirectory: String { configuration.workingDirectory.path }

    // MARK: - Lifecycle

    func start() async throws -> String {
        guard let executable = configuration.executable else { throw ProviderError.notInstalled(configuration.provider) }
        let connection = try await Self.connect(
            executable: executable,
            directory: configuration.workingDirectory,
            environment: configuration.environment
        ) { [weak self] connection in
            connection.onNotification = { self?.handleNotification($0, $1) }
            connection.onRequest = { self?.handleRequest($0, $1, $2) }
            connection.onClose = { self?.handleClose($0) }
        }
        self.connection = connection

        let id = configuration.resumeID ?? UUID().uuidString.lowercased()
        var params = sessionParameters()
        params["sessionId"] = .string(id)
        // Resume fails for an id the CLI no longer has; the thread then starts a fresh session.
        _ = try await connection.request(configuration.resumeID == nil ? "session.create" : "session.resume", .object(params))
        sessionID = id
        // A resumed session comes back in interactive mode with manual permissions
        // (verified live), so the thread's modes are applied either way.
        await applyModes(runtime: configuration.runtimeMode, interaction: configuration.interactionMode)
        loadCommands()
        return id
    }

    /// What every session asks for: streaming deltas, permission prompts, questions
    /// and plan exits routed to the app. Auto mode needs the CLI's experimental
    /// flag for its assisted-approval judge, which is a session setting, so the
    /// thread restarts the session when it switches into or out of Auto.
    private func sessionParameters() -> [String: JSONValue] {
        var params: [String: JSONValue] = [
            "clientName": "droppy-code",
            "workingDirectory": .string(workingDirectory),
            "streaming": true,
            "requestPermission": true,
            "requestUserInput": true,
            "requestExitPlanMode": true,
            "includeSubAgentStreamingEvents": .bool(configuration.hydra?.runsNatively == true),
            "isExperimentalMode": .bool(configuration.runtimeMode == .auto),
        ]
        if let model, !model.isEmpty { params["model"] = .string(model) }
        if let effort, !effort.isEmpty { params["reasoningEffort"] = .string(effort) }
        if let hydra = configuration.hydra {
            if hydra.runsNatively {
                params["customAgents"] = .array(HydraPrompts.copilotAgents(hydra))
                params["systemMessage"] = ["mode": "append", "content": .string(HydraPrompts.policy(for: .copilot, maxHeads: hydra.maxHeads, autoMerges: hydra.autoMerges, reviewsHeads: hydra.reviewsHeads))]
            } else {
                // Heads on another provider are Droppy-run: the lead asks for them with the
                // delegation block, and no agents of the CLI's own are defined.
                params["systemMessage"] = ["mode": "append", "content": .string(HydraPrompts.fallbackPolicy(hydra))]
            }
        }
        return params
    }

    func send(_ input: TurnInput) async throws {
        guard let connection, !connection.isClosed else { throw ProviderError.notRunning }
        await applyModes(runtime: input.runtimeMode, interaction: input.interactionMode)
        await applySelection(model: input.model, effort: input.effort)
        beginTurn()
        do {
            // Slash commands are the terminal UI's, not the model's: the CLI runs them and
            // answers with text to show or a prompt to send.
            if let command = Self.slashCommand(in: input.text), commandNames.contains(command.name) {
                try await invokeCommand(command.name, input: command.input)
            } else {
                try await sendPrompt(input.text, images: input.images)
            }
        } catch {
            turnActive = false
            throw error
        }
    }

    private func beginTurn() {
        turnActive = true
        turnError = nil
        interruptRequested = false
        shownReasoning.removeAll()
        streamedMessages.removeAll()
        onEvent?(.turnStarted(providerTurnID: nil))
    }

    private func sendPrompt(_ text: String, images: [Attachment]) async throws {
        guard let connection, let sessionID else { throw ProviderError.notRunning }
        var params: [String: JSONValue] = [
            "sessionId": .string(sessionID),
            "prompt": .string(text),
        ]
        // The CLI reads image files itself and hands them to vision models.
        let attachments: [JSONValue] = images.filter(\.isImage).map { image in
            ["type": "file", "path": .string(image.path), "displayName": .string(image.name), "mimeType": .string(image.mimeType)]
        }
        if !attachments.isEmpty { params["attachments"] = .array(attachments) }
        _ = try await connection.request("session.send", .object(params))
    }

    private func invokeCommand(_ name: String, input: String) async throws {
        guard let connection, let sessionID else { throw ProviderError.notRunning }
        let result = try await connection.request("session.commands.invoke", [
            "sessionId": .string(sessionID),
            "name": .string(name),
            "input": .string(input),
        ])
        if let mode = result["mode"]?.string {
            interactionMode = mode == "plan" ? .plan : .build
            onEvent?(.modeChanged(interactionMode))
        }
        switch result["kind"]?.string {
        case "agent-prompt":
            try await sendPrompt(result["prompt"]?.string ?? input, images: [])
        case "text", "completed":
            let text = TextCleanup.stripANSI(result["text"]?.string ?? result["message"]?.string ?? "")
            if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let body = result["markdown"]?.bool == true ? text : "```\n\(text)\n```"
                onEvent?(.messageCompleted(id: "command-\(UUID().uuidString)", text: body))
            }
            finishTurn(aborted: false)
        default:
            onEvent?(.notice(Notice(level: .warning, message: "/\(name) opens a dialog that only Copilot's own terminal UI can show.")))
            finishTurn(aborted: false)
        }
    }

    /// `/name rest…` at the start of a prompt, or nil for ordinary text.
    private static let slashCommandPattern = #/^/([A-Za-z][\w-]*)(?:\s+([\s\S]*))?$/#

    static func slashCommand(in text: String) -> (name: String, input: String)? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let match = trimmed.firstMatch(of: slashCommandPattern) else { return nil }
        return (String(match.output.1), match.output.2.map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) } ?? "")
    }

    func interrupt() async {
        guard turnActive, let connection, let sessionID else { return }
        interruptRequested = true
        cancelPendingRequests()
        _ = try? await connection.request("session.abort", ["sessionId": .string(sessionID), "reason": "user_initiated"])
    }

    func compact() async throws {
        guard let connection, let sessionID else { throw ProviderError.notRunning }
        _ = try await connection.request("session.history.compact", ["sessionId": .string(sessionID), "trigger": "manual"])
    }

    /// Drops the most recent turns from Copilot's own history. Files stay as they
    /// are: Droppy Code restores those from its checkpoints.
    func rollback(turns: Int) async throws {
        guard let connection, let sessionID, turns > 0 else { return }
        let listed = try await connection.request("session.history.listRewindPoints", ["sessionId": .string(sessionID)])
        let points = listed["points"]?.array ?? []
        // One point per user message; rewinding to a point drops that message and everything after it.
        guard points.count >= turns, let eventID = points[points.count - turns]["eventId"]?.string else {
            throw ProviderError.failed("Copilot has no rewind point for that turn.")
        }
        let result = try await connection.request("session.history.rewind", [
            "sessionId": .string(sessionID),
            "eventId": .string(eventID),
            "mode": "conversation",
        ])
        guard result["outcome"]?.string == "success" else {
            throw ProviderError.failed(result["error"]?.string ?? "Copilot could not rewind the conversation.")
        }
    }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let pending = pendingRequests.removeValue(forKey: requestID) else { return }
        switch pending {
        case .permission(let request, let toolCallID):
            let decision: JSONValue
            switch optionID {
            case "allow":
                decision = ["kind": "approve-once", "approvedInteractively": true]
            case "always":
                decision = Self.sessionApproval(for: request) ?? ["kind": "approve-once", "approvedInteractively": true]
            default:
                decision = ["kind": "reject", "feedback": "The user declined this action."]
                if let toolCallID {
                    declinedTools.insert(toolCallID)
                    onEvent?(.toolUpdated(id: toolCallID, update: ToolUpdate(status: .declined)))
                }
            }
            respondPermission(requestID, decision)
        case .question(let id, _):
            connection?.respond(to: id, result: ["answer": "", "wasFreeform": true])
        case .planExit(let id):
            if optionID == "allow" {
                // The CLI leaves plan mode itself and says so in `session.mode_changed`.
                connection?.respond(to: id, result: ["approved": true, "selectedAction": "interactive"])
                onEvent?(.modeChanged(.build))
            } else {
                connection?.respond(to: id, result: [
                    "approved": false,
                    "feedback": "The user wants to keep planning. Wait for their feedback before changing anything.",
                ])
            }
        }
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {
        guard case .question(let id, let choices)? = pendingRequests.removeValue(forKey: requestID) else { return }
        let answer = answers.values.first?.joined(separator: ", ") ?? ""
        connection?.respond(to: id, result: ["answer": .string(answer), "wasFreeform": .bool(!choices.contains(answer))])
        onEvent?(.requestResolved(id: requestID))
    }

    func stop() {
        isStopping = true
        guard let connection, !connection.isClosed else { return }
        // A shutdown lets the CLI flush the session's event log; the close after it is the backstop.
        Task {
            _ = try? await connection.request("runtime.shutdown")
            connection.close()
        }
        Task {
            try? await Task.sleep(for: .seconds(3))
            connection.close()
        }
    }

    // MARK: - Modes and selection

    private func applyModes(runtime: RuntimeMode, interaction: InteractionMode) async {
        guard let connection, let sessionID else { return }
        runtimeMode = runtime
        let wanted = Self.permissionModeName(runtime)
        if wanted != permissionMode {
            // `assisted` is refused outside experimental mode; the reply says which mode holds,
            // and in `manual` every request reaches the app's own rules instead.
            let result = try? await connection.request("session.permissions.setMode", [
                "sessionId": .string(sessionID),
                "mode": .string(wanted),
                "source": "rpc",
            ])
            permissionMode = result?["mode"]?.string ?? permissionMode
        }
        if interaction != interactionMode {
            _ = try? await connection.request("session.mode.set", [
                "sessionId": .string(sessionID),
                "mode": .string(interaction == .plan ? "plan" : "interactive"),
            ])
            interactionMode = interaction
        }
    }

    private func applySelection(model: String?, effort: String?) async {
        guard let connection, let sessionID else { return }
        if let model, !model.isEmpty, model != self.model {
            var params: [String: JSONValue] = ["sessionId": .string(sessionID), "modelId": .string(model)]
            if let effort, !effort.isEmpty { params["reasoningEffort"] = .string(effort) }
            if (try? await connection.request("session.model.switchTo", .object(params))) != nil {
                self.model = model
                if let effort, !effort.isEmpty { self.effort = effort }
            }
        }
        if let effort, !effort.isEmpty, effort != self.effort {
            if (try? await connection.request("session.model.setReasoningEffort", [
                "sessionId": .string(sessionID),
                "reasoningEffort": .string(effort),
            ])) != nil {
                self.effort = effort
            }
        }
    }

    /// Full access lets the CLI approve everything itself. Auto asks its
    /// assisted-approval judge; the other modes keep every prompt in the app,
    /// where `automaticDecision` waves through what the mode allows.
    static func permissionModeName(_ mode: RuntimeMode) -> String {
        switch mode {
        case .supervised, .autoAcceptEdits: "manual"
        case .auto: "assisted"
        case .fullAccess: "allow-all"
        }
    }

    private func loadCommands() {
        guard let connection, let sessionID else { return }
        Task { [weak self] in
            guard let self, let result = try? await connection.request("session.commands.list", ["sessionId": .string(sessionID)]) else { return }
            var names: Set<String> = []
            var commands: [SlashCommand] = []
            for entry in result["commands"]?.array ?? [] {
                guard let name = entry["name"]?.string, !Self.ownCommands.contains(name) else { continue }
                names.insert(name)
                names.formUnion((entry["aliases"]?.array ?? []).compactMap(\.string))
                commands.append(SlashCommand(name: name, detail: entry["description"]?.string ?? ""))
            }
            commandNames = names
            if !commands.isEmpty { onEvent?(.commands(commands)) }
        }
    }

    // MARK: - Catalog and account

    /// `models.list`: every model the account may use, with its efforts and premium-request multiplier.
    static func listModels(executable: URL, environment: [String: String]) async throws -> [ModelOption] {
        let connection = try await connect(executable: executable, directory: FileManager.default.temporaryDirectory, environment: environment) { _ in }
        defer { connection.close() }
        let result = try await connection.request("models.list", [:])
        var models = [ProviderRegistry.copilotAuto]
        for model in result["models"]?.array ?? [] {
            guard let id = model["id"]?.string, model["policy"]?["state"]?.string != "disabled" else { continue }
            var details: [String] = []
            if let multiplier = model["billing"]?["multiplier"]?.double {
                details.append(multiplier == multiplier.rounded() ? "\(Int(multiplier))× premium requests" : "\(multiplier)× premium requests")
            }
            if model["policy"]?["state"]?.string == "unconfigured" {
                details.append("Enable it in your GitHub Copilot settings first")
            }
            models.append(ModelOption(
                id: id,
                name: model["name"]?.string ?? id,
                detail: details.isEmpty ? nil : details.joined(separator: " · "),
                efforts: (model["supportedReasoningEfforts"]?.array ?? []).compactMap(\.string),
                defaultEffort: model["defaultReasoningEffort"]?.string
            ))
        }
        return models
    }

    /// `account.getCurrentAuth`: the GitHub login behind the CLI, from `copilot login` or `gh auth`.
    static func authStatus(executable: URL, environment: [String: String]) async -> ProviderStatus.Auth {
        guard let connection = try? await connect(executable: executable, directory: FileManager.default.temporaryDirectory, environment: environment, configure: { _ in }) else {
            return .unknown
        }
        defer { connection.close() }
        guard let result = try? await connection.request("account.getCurrentAuth", [:]) else { return .unknown }
        guard let auth = result["authInfo"], !auth.isNull else { return .signedOut }
        var account = auth["login"]?.string
        // Signed in to GitHub without a Copilot plan: the turn would fail with HTTP 403.
        if auth["copilotUser"]?["access_type_sku"]?.string == "no_access" {
            account = (account ?? "GitHub") + " · no Copilot access"
        }
        return .signedIn(account)
    }

    /// `account.getQuota`: the monthly premium-request, chat and completion allowances.
    static func readPlanLimits(executable: URL, environment: [String: String]) async throws -> PlanLimits? {
        let connection = try await connect(executable: executable, directory: FileManager.default.temporaryDirectory, environment: environment) { _ in }
        defer { connection.close() }
        let quota = try await connection.request("account.getQuota", [:])
        let titles = [("premium_interactions", "Premium requests"), ("chat", "Chat messages"), ("completions", "Code completions")]
        var windows: [PlanLimits.Window] = []
        for (key, title) in titles {
            guard let snapshot = quota["quotaSnapshots"]?[key], !snapshot.isNull,
                  snapshot["isUnlimitedEntitlement"]?.bool != true,
                  let entitlement = snapshot["entitlementRequests"]?.double, entitlement > 0 else { continue }
            let remaining = snapshot["remainingPercentage"]?.double
            let used = snapshot["usedRequests"]?.double ?? 0
            let percent = remaining.map { max(0, min(100, 100 - $0)) } ?? min(100, used / entitlement * 100)
            windows.append(PlanLimits.Window(
                id: key,
                title: title,
                percent: percent,
                resetsAt: snapshot["resetDate"]?.string.flatMap(parseDate)
            ))
        }
        guard !windows.isEmpty else { return nil }
        let auth = try? await connection.request("account.getCurrentAuth", [:])
        let plan = auth?["authInfo"]?["copilotUser"]?["copilot_plan"]?.string
        return PlanLimits(planName: PlanLimitsReader.planName(plan).map { "Copilot \($0)" }, windows: windows)
    }

    private static func parseDate(_ text: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        if let date = formatter.date(from: text) { return date }
        formatter.formatOptions = [.withFullDate]
        return formatter.date(from: text)
    }

    private static func connect(
        executable: URL,
        directory: URL,
        environment: [String: String],
        configure: (JSONRPCConnection) -> Void
    ) async throws -> JSONRPCConnection {
        let process = StdioProcess(
            executable: executable,
            arguments: ["--headless", "--stdio", "--no-auto-update", "--log-level", "error"],
            directory: directory,
            environment: environment,
            framing: .contentLength
        )
        let connection = JSONRPCConnection(process: process, sendsVersion: true)
        configure(connection)
        try connection.start()
        let pong = try await connection.request("ping", ["message": "droppy-code"])
        guard pong["protocolVersion"]?.int != nil else {
            connection.close()
            throw ProviderError.failed("This `copilot` does not speak the SDK protocol. Update it with `copilot update`.")
        }
        return connection
    }

    // MARK: - Notifications

    private func handleNotification(_ method: String, _ params: JSONValue) {
        guard method == "session.event", let event = params["event"], let type = event["type"]?.string else { return }
        if let eventSession = params["sessionId"]?.string, let sessionID, eventSession != sessionID { return }
        // Sub-agents stream under their own agent id. With Hydra on they are heads and their
        // transcripts go to the head's own timeline; otherwise their parent `task` row stands
        // for them. Their permission prompts still need answering and their spend still
        // counts, so those pass either way.
        let agentID = event["agentId"]?.string
        if agentID != nil, configuration.hydra == nil, type != "assistant.usage", type.hasPrefix("assistant.") || type.hasPrefix("tool.") { return }
        let sink: (ProviderEvent) -> Void = if let agentID {
            { [weak self] event in self?.onEvent?(.agentEvent(agentID: agentID, event)) }
        } else {
            leadSink
        }
        let data = event["data"] ?? .null
        switch type {
        case "assistant.turn_start":
            if agentID != nil {
                sink(.turnStarted(providerTurnID: data["turnId"]?.string))
            } else if !turnActive {
                turnActive = true
                onEvent?(.turnStarted(providerTurnID: data["turnId"]?.string))
            }
        case "assistant.message_delta":
            if let id = data["messageId"]?.string, let delta = data["deltaContent"]?.string {
                streamedMessages.insert(id)
                sink(.messageDelta(id: id, text: delta))
            }
        case "assistant.message":
            handleMessage(data, sink: sink)
        case "assistant.reasoning_delta":
            if let id = data["reasoningId"]?.string, let delta = data["deltaContent"]?.string {
                sink(.reasoningDelta(id: id, text: delta))
            }
        case "assistant.reasoning":
            if let id = data["reasoningId"]?.string {
                let text = data["content"]?.string ?? ""
                shownReasoning.insert(Self.reasoningKey(text))
                sink(.reasoningCompleted(id: id, text: text))
            }
        case "assistant.usage":
            let spend = (data["inputTokens"]?.int ?? 0) + (data["outputTokens"]?.int ?? 0)
            if spend > 0 { TokenLedger.shared.record(spend: spend) }
        case "session.usage_info":
            if let used = data["currentTokens"]?.int {
                onEvent?(.usage(ContextUsage(usedTokens: used, windowTokens: data["tokenLimit"]?.int)))
            }
        case "tool.execution_start":
            if let id = data["toolCallId"]?.string, let name = data["toolName"]?.string {
                toolRequested(id: id, name: name, arguments: data["arguments"] ?? .null, server: data["mcpServerName"]?.string, mcpTool: data["mcpToolName"]?.string, sink: sink)
            }
        case "tool.execution_partial_result":
            if let id = data["toolCallId"]?.string, let output = data["partialOutput"]?.string {
                sink(.toolOutput(id: id, text: output))
            }
        case "tool.execution_progress":
            if let id = data["toolCallId"]?.string, let message = data["progressMessage"]?.string {
                sink(.toolUpdated(id: id, update: ToolUpdate(detail: message)))
            }
        case "tool.execution_complete":
            handleToolResult(data, sink: sink)
        case "subagent.started":
            guard let agentID, configuration.hydra != nil else { break }
            let brief = data["toolCallId"]?.string.flatMap { taskBriefs.removeValue(forKey: $0) }
            onEvent?(.agentStarted(AgentSpawn(
                id: agentID,
                taskID: nil,
                toolUseID: data["toolCallId"]?.string,
                description: brief?.description ?? data["agentDisplayName"]?.string ?? data["agentName"]?.string ?? "Subagent",
                prompt: brief?.prompt,
                model: data["model"]?.string
            )))
        case "subagent.completed":
            guard let agentID, configuration.hydra != nil else { break }
            onEvent?(.agentFinished(agentID: agentID, status: data["cancelled"]?.bool == true ? .interrupted : .completed, summary: nil))
        case "subagent.failed":
            guard let agentID, configuration.hydra != nil else { break }
            onEvent?(.agentFinished(agentID: agentID, status: .failed, summary: data["error"]?.string))
        case "permission.requested":
            handlePermission(data)
        case "permission.completed":
            if let requestID = data["requestId"]?.string, pendingRequests.removeValue(forKey: requestID) != nil {
                onEvent?(.requestResolved(id: requestID))
            }
        case "session.mode_changed":
            switch data["newMode"]?.string {
            case "plan":
                interactionMode = .plan
                onEvent?(.modeChanged(.plan))
            case "interactive", "autopilot":
                interactionMode = .build
                onEvent?(.modeChanged(.build))
            default:
                break
            }
        case "session.title_changed":
            if let title = data["title"]?.string, !title.isEmpty { onEvent?(.title(title)) }
        case "session.error":
            let message = Self.errorMessage(data)
            if turnActive {
                turnError = message
            } else {
                onEvent?(.notice(Notice(level: .error, message: message)))
            }
        case "session.warning":
            if let message = data["message"]?.string { onEvent?(.notice(Notice(level: .warning, message: message))) }
        case "session.compaction_complete":
            if data["success"]?.bool == true {
                onEvent?(.notice(Notice(level: .info, message: "Context compacted.")))
            } else if let error = data["error"]?.string {
                onEvent?(.notice(Notice(level: .warning, message: error)))
            }
        case "session.idle":
            finishTurn(aborted: data["aborted"]?.bool == true)
        case "session.shutdown":
            if let reason = data["errorReason"]?.string, data["shutdownType"]?.string == "error" {
                onEvent?(.notice(Notice(level: .error, message: reason)))
            }
        default:
            break
        }
    }

    private func handleMessage(_ data: JSONValue, sink: (ProviderEvent) -> Void) {
        guard let id = data["messageId"]?.string else { return }
        // Models that think without reasoning events carry the text on the message instead.
        if let reasoning = data["reasoningText"]?.string, reasoning.contains(where: { !$0.isWhitespace }),
           shownReasoning.insert(Self.reasoningKey(reasoning)).inserted {
            sink(.reasoningCompleted(id: "\(id)-reasoning", text: reasoning))
        }
        let content = data["content"]?.string ?? ""
        if !content.isEmpty || streamedMessages.contains(id) {
            sink(.messageCompleted(id: id, text: content))
        }
        // The calls this message asks for, so a permission prompt has its row to point at.
        for request in data["toolRequests"]?.array ?? [] {
            guard let callID = request["toolCallId"]?.string, let name = request["name"]?.string else { continue }
            toolRequested(id: callID, name: name, arguments: request["arguments"] ?? .null, server: request["mcpServerName"]?.string, mcpTool: request["mcpToolName"]?.string, sink: sink)
        }
    }

    private func toolRequested(id: String, name: String, arguments: JSONValue, server: String?, mcpTool: String?, sink: (ProviderEvent) -> Void) {
        guard toolNames[id] == nil else { return }
        toolNames[id] = name
        if name == "update_todo" {
            sink(.todos(Self.todos(from: arguments["todos"]?.string ?? "")))
            return
        }
        if name == "task" {
            taskBriefs[id] = (
                arguments["description"]?.string ?? arguments["name"]?.string ?? "Subagent",
                arguments["prompt"]?.string ?? arguments["task"]?.string
            )
        }
        guard let call = toolCall(name: name, arguments: arguments, server: server, mcpTool: mcpTool) else { return }
        sink(.toolStarted(id: id, call: call))
        if !call.edits.isEmpty {
            sink(.toolUpdated(id: id, update: ToolUpdate(edits: call.edits)))
        }
    }

    private func handleToolResult(_ data: JSONValue, sink: (ProviderEvent) -> Void) {
        guard let id = data["toolCallId"]?.string else { return }
        let name = toolNames[id] ?? ""
        guard !Self.hiddenTools.contains(name) else { return }
        let succeeded = data["success"]?.bool ?? true
        var update = ToolUpdate()
        if let error = data["error"]?["message"]?.string, !succeeded {
            update.output = error
        } else if let result = data["result"] {
            update.output = result["detailedContent"]?.string ?? result["content"]?.string
        }
        update.status = succeeded ? .completed : (declinedTools.contains(id) ? .declined : .failed)
        sink(.toolUpdated(id: id, update: update))
    }

    private func finishTurn(aborted: Bool) {
        guard turnActive else { return }
        turnActive = false
        // Every call of the turn has had its result; the names, declines, briefs and shown
        // reasoning were for those.
        toolNames.removeAll()
        declinedTools.removeAll()
        taskBriefs.removeAll()
        shownReasoning.removeAll()
        cancelPendingRequests()
        let error = turnError
        turnError = nil
        if aborted || interruptRequested {
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
        } else if let error {
            onEvent?(.turnCompleted(status: .failed, error: error))
        } else {
            onEvent?(.turnCompleted(status: .completed, error: nil))
        }
        interruptRequested = false
    }

    /// The CLI's message, with the way out when the account itself is the problem.
    static func errorMessage(_ data: JSONValue) -> String {
        var message = data["message"]?.string ?? "Copilot reported an error."
        if let raw = message.range(of: "GenericFailure, ") {
            // The model-list failure wraps the HTTP reply as JSON after this marker.
            let json = JSONValue.parse(String(message[raw.upperBound...]))
            if let status = json?["status"]?.int, let body = json?["body"]?.string {
                message = "HTTP \(status): \(body.trimmingCharacters(in: .whitespacesAndNewlines))"
            }
        }
        if data["statusCode"]?.int == 403 || data["remediation"]?.string == "sign_in"
            || message.localizedCaseInsensitiveContains("not authorized to use this Copilot feature") {
            message += " Check that your GitHub account has Copilot access, or sign in again with `copilot login`."
        }
        return message
    }

    // MARK: - Requests

    private func handleRequest(_ id: RPCID, _ method: String, _ params: JSONValue) {
        switch method {
        case "userInput.request":
            let choices = (params["choices"]?.array ?? []).compactMap(\.string)
            let key = "question-\(id.key)"
            pendingRequests[key] = .question(id, choices: choices)
            onEvent?(.question(QuestionRequest(id: key, questions: [
                QuestionRequest.Question(
                    id: "answer",
                    header: "",
                    prompt: params["question"]?.string ?? "",
                    choices: choices.map { QuestionRequest.Choice(label: $0, detail: nil) },
                    allowsMultiple: false,
                    allowsOther: params["allowFreeform"]?.bool ?? true,
                    isSecret: false
                ),
            ])))
        case "exitPlanMode.request":
            let key = "plan-\(id.key)"
            pendingRequests[key] = .planExit(id)
            let plan = params["planContent"]?.string.flatMap { $0.isEmpty ? nil : $0 } ?? params["summary"]?.string ?? ""
            onEvent?(.planCompleted(id: key, markdown: plan))
            onEvent?(.approval(ApprovalRequest(
                id: key,
                kind: .plan,
                title: "Implement this plan",
                options: [
                    .init(id: "allow", title: "Implement plan", role: .approve),
                    .init(id: "deny", title: "Keep planning", role: .decline),
                ],
                toolItemID: key
            )))
        case "autoModeSwitch.request":
            // A rate limit offered a switch to the Auto model tier: keep the model the user chose.
            connection?.respond(to: id, result: ["response": "no"])
        default:
            connection?.respond(to: id, errorCode: -32601, message: "Droppy Code does not support \(method).")
        }
    }

    private func handlePermission(_ data: JSONValue) {
        guard let requestID = data["requestId"]?.string, let request = data["permissionRequest"] else { return }
        if data["resolvedByHook"]?.bool == true { return }
        let toolCallID = request["toolCallId"]?.string
        if let decision = automaticDecision(request, prompt: data["promptRequest"]) {
            respondPermission(requestID, decision)
            return
        }
        pendingRequests[requestID] = .permission(request: request, toolCallID: toolCallID)
        // A write prompt carries the CLI's own patch, which beats the row's reconstruction.
        if request["kind"]?.string == "write", let toolCallID, let diff = request["diff"]?.string, !diff.isEmpty {
            let path = ToolTitles.relativePath(request["resolvedPath"]?.string ?? request["fileName"]?.string ?? "", to: workingDirectory)
            let stats = ToolTitles.diffStats(diff)
            onEvent?(.toolUpdated(id: toolCallID, update: ToolUpdate(edits: [FileEdit(path: path, diff: diff, additions: stats.additions, deletions: stats.deletions)])))
        }
        var options: [ApprovalRequest.Option] = [.init(id: "allow", title: "Approve", role: .approve)]
        if Self.sessionApproval(for: request) != nil {
            options.append(.init(id: "always", title: "Approve for session", role: .approveAlways))
        }
        options.append(.init(id: "deny", title: "Decline", role: .decline))
        var approval = describePermission(request)
        approval.id = requestID
        approval.options = options
        approval.toolItemID = toolCallID
        if approval.reason == nil, let judged = data["promptRequest"]?["assistedApproval"]?["reason"]?.string, !judged.isEmpty {
            approval.reason = judged
        }
        onEvent?(.approval(approval))
    }

    /// What the thread's mode approves without asking. Full access approves it all
    /// (the CLI normally does that itself in `allow-all`), accept-edits waves file
    /// reads and writes through, and Auto adds whatever the CLI's judge vouched for.
    private func automaticDecision(_ request: JSONValue, prompt: JSONValue?) -> JSONValue? {
        let kind = request["kind"]?.string ?? ""
        let approve: JSONValue = ["kind": "approve-once"]
        switch runtimeMode {
        case .fullAccess:
            return approve
        case .autoAcceptEdits:
            return kind == "write" || kind == "read" ? approve : nil
        case .auto:
            if kind == "write" || kind == "read" { return approve }
            let recommendation = prompt?["assistedApproval"]?["recommendation"]?.string ?? request["assistedApproval"]?["recommendation"]?.string
            return recommendation == "approve" ? approve : nil
        case .supervised:
            return nil
        }
    }

    private func describePermission(_ request: JSONValue) -> ApprovalRequest {
        let intention = request["intention"]?.string.flatMap { $0.isEmpty ? nil : $0 }
        switch request["kind"]?.string {
        case "shell":
            let command = ToolTitles.unwrapShell(request["fullCommandText"]?.string ?? "Command")
            var details: [String] = []
            if let directory = request["resolvedWorkingDirectory"]?.string, directory != workingDirectory {
                details.append(ToolTitles.relativePath(directory, to: workingDirectory))
            }
            if request["requestSandboxBypass"]?.bool == true {
                details.append(request["requestSandboxBypassReason"]?.string ?? "Runs outside the sandbox.")
            }
            return ApprovalRequest(id: "", kind: .command, title: command, detail: details.isEmpty ? nil : details.joined(separator: " · "), reason: intention, options: [])
        case "write":
            let path = request["resolvedPath"]?.string ?? request["fileName"]?.string ?? "file"
            return ApprovalRequest(id: "", kind: .fileChange, title: ToolTitles.relativePath(path, to: workingDirectory), reason: intention, options: [])
        case "read":
            let path = request["resolvedPath"]?.string ?? request["path"]?.string ?? "file"
            return ApprovalRequest(id: "", kind: .permissions, title: "Read \(ToolTitles.relativePath(path, to: workingDirectory))", reason: intention, options: [])
        case "url":
            let url = request["url"]?.string ?? "a URL"
            let detail = request["redirectedFrom"]?.string.map { "Redirected from \($0)" }
            return ApprovalRequest(id: "", kind: .permissions, title: "Fetch \(url)", detail: detail, reason: intention, options: [])
        case "mcp":
            let server = request["serverName"]?.string ?? "MCP"
            let tool = request["toolTitle"]?.string ?? request["toolName"]?.string ?? "tool"
            let arguments = request["args"].map(\.compactString)
            return ApprovalRequest(id: "", kind: .tool, title: "\(server) · \(tool)", detail: arguments == "null" ? nil : arguments, options: [])
        case "memory":
            let fact = request["fact"]?.string ?? "something about this project"
            return ApprovalRequest(id: "", kind: .tool, title: "Remember: \(fact)", reason: request["reason"]?.string, options: [])
        case "custom-tool":
            let tool = request["toolName"]?.string ?? "Tool"
            return ApprovalRequest(id: "", kind: .tool, title: tool, detail: request["toolDescription"]?.string, options: [])
        case "hook":
            let tool = request["toolName"]?.string ?? "Tool"
            return ApprovalRequest(id: "", kind: .tool, title: request["hookMessage"]?.string ?? tool, detail: request["hookMessage"] == nil ? nil : tool, options: [])
        case "extension-management":
            let operation = request["operation"]?.string ?? "manage"
            let extensionName = request["extensionName"]?.string ?? "an extension"
            return ApprovalRequest(id: "", kind: .tool, title: "\(operation.capitalized) \(extensionName)", options: [])
        default:
            let kind = request["kind"]?.string ?? "action"
            return ApprovalRequest(id: "", kind: .tool, title: Self.humanize(kind), reason: intention, options: [])
        }
    }

    /// The approve-for-session decision for a request, or nil when the CLI cannot remember this kind.
    static func sessionApproval(for request: JSONValue) -> JSONValue? {
        let approval: JSONValue
        switch request["kind"]?.string {
        case "shell":
            guard request["canOfferSessionApproval"]?.bool == true else { return nil }
            let identifiers = (request["commands"]?.array ?? []).compactMap { $0["identifier"]?.string }
            guard !identifiers.isEmpty else { return nil }
            approval = ["kind": "commands", "commandIdentifiers": .array(identifiers.map(JSONValue.string))]
        case "write":
            guard request["canOfferSessionApproval"]?.bool == true else { return nil }
            approval = ["kind": "write"]
        case "read":
            approval = ["kind": "read"]
        case "url":
            guard let url = request["url"]?.string, let host = URL(string: url)?.host, !host.isEmpty else { return nil }
            return ["kind": "approve-for-session", "domain": .string(host)]
        case "mcp":
            guard let server = request["serverName"]?.string else { return nil }
            approval = ["kind": "mcp", "serverName": .string(server), "toolName": .optional(request["toolName"]?.string)]
        case "custom-tool":
            guard let tool = request["toolName"]?.string else { return nil }
            approval = ["kind": "custom-tool", "toolName": .string(tool)]
        case "memory":
            approval = ["kind": "memory"]
        default:
            return nil
        }
        return ["kind": "approve-for-session", "approval": approval]
    }

    private func respondPermission(_ requestID: String, _ decision: JSONValue) {
        guard let connection, let sessionID else { return }
        Task {
            _ = try? await connection.request("session.permissions.handlePendingPermissionRequest", [
                "sessionId": .string(sessionID),
                "requestId": .string(requestID),
                "result": decision,
            ])
        }
    }

    private func cancelPendingRequests() {
        let waiting = pendingRequests
        pendingRequests.removeAll()
        for (key, pending) in waiting {
            switch pending {
            case .permission:
                respondPermission(key, ["kind": "cancelled", "reason": "The user stopped the turn."])
            case .question(let id, _):
                connection?.respond(to: id, errorCode: -32800, message: "The user stopped the turn.")
            case .planExit(let id):
                connection?.respond(to: id, result: ["approved": false, "feedback": "The user stopped the turn."])
            }
            onEvent?(.requestResolved(id: key))
        }
    }

    private func handleClose(_ tail: String) {
        let resolved = Array(pendingRequests.keys)
        pendingRequests.removeAll()
        for key in resolved { onEvent?(.requestResolved(id: key)) }
        turnActive = false
        guard !isStopping else { return }
        onEvent?(.exited(error: TextCleanup.lastLines(tail)))
    }

    // MARK: - Mapping

    /// Tools that never get a row: questions, plans and to-dos have their own surfaces.
    private static let hiddenTools: Set<String> = ["ask_user", "exit_plan_mode", "update_todo", "task_complete", "report_intent"]

    private func toolCall(name: String, arguments: JSONValue, server: String?, mcpTool: String?) -> ToolCall? {
        if let server {
            let tool = mcpTool ?? name
            return ToolCall(kind: .mcp, title: "\(server) · \(tool)", detail: arguments.isNull ? nil : arguments.compactString)
        }
        func path(_ key: String) -> String? {
            arguments[key]?.string.map { ToolTitles.relativePath($0, to: workingDirectory) }
        }
        switch name {
        case _ where Self.hiddenTools.contains(name):
            return nil
        case "bash", "powershell":
            return ToolCall(kind: .command, title: arguments["command"]?.string ?? "Shell command", detail: arguments["description"]?.string)
        case "read_bash", "write_bash", "stop_bash", "list_bash":
            return ToolCall(kind: .command, title: Self.humanize(name), detail: arguments["shellId"]?.string)
        case "view":
            return ToolCall(kind: .read, title: path("path") ?? "Read file", detail: Self.lineRange(arguments["view_range"]))
        case "create":
            var call = ToolCall(kind: .edit, title: path("path") ?? "Create file")
            if let file = path("path") {
                call.edits = [FileEdit(path: file, old: "", new: arguments["file_text"]?.string ?? "")]
            }
            return call
        case "edit":
            var call = ToolCall(kind: .edit, title: path("path") ?? "Edit file")
            if let file = path("path") {
                call.edits = [FileEdit(path: file, old: arguments["old_str"]?.string ?? "", new: arguments["new_str"]?.string ?? "")]
            }
            return call
        case "str_replace_editor":
            switch arguments["command"]?.string {
            case "view":
                return ToolCall(kind: .read, title: path("path") ?? "Read file", detail: Self.lineRange(arguments["view_range"]))
            case "create":
                var call = ToolCall(kind: .edit, title: path("path") ?? "Create file")
                if let file = path("path") {
                    call.edits = [FileEdit(path: file, old: "", new: arguments["file_text"]?.string ?? "")]
                }
                return call
            case "insert":
                var call = ToolCall(kind: .edit, title: path("path") ?? "Edit file")
                if let file = path("path") {
                    call.edits = [FileEdit(path: file, old: "", new: arguments["new_str"]?.string ?? "")]
                }
                return call
            default:
                var call = ToolCall(kind: .edit, title: path("path") ?? "Edit file")
                if let file = path("path") {
                    call.edits = [FileEdit(path: file, old: arguments["old_str"]?.string ?? "", new: arguments["new_str"]?.string ?? "")]
                }
                return call
            }
        case "apply_patch":
            let patch = arguments.string ?? arguments["input"]?.string ?? arguments["patch"]?.string ?? ""
            let paths = Self.patchPaths(patch).map { ToolTitles.relativePath($0, to: workingDirectory) }
            var call = ToolCall(kind: .edit, title: paths.count == 1 ? paths[0] : "Edit \(paths.count) files")
            call.edits = paths.map { FileEdit(path: $0) }
            return call
        case "grep":
            let paths = (arguments["paths"]?.array ?? []).compactMap(\.string).map { ToolTitles.relativePath($0, to: workingDirectory) }
            return ToolCall(kind: .search, title: arguments["pattern"]?.string ?? "Search", detail: paths.isEmpty ? nil : paths.joined(separator: ", "))
        case "glob":
            let paths = (arguments["paths"]?.array ?? []).compactMap(\.string).map { ToolTitles.relativePath($0, to: workingDirectory) }
            return ToolCall(kind: .search, title: arguments["pattern"]?.string ?? "Find files", detail: paths.isEmpty ? nil : paths.joined(separator: ", "))
        case "web_fetch":
            return ToolCall(kind: .web, title: arguments["url"]?.string ?? "Fetch page")
        case "web_search":
            return ToolCall(kind: .web, title: arguments["query"]?.string ?? "Search the web")
        case "task":
            let detail = [arguments["name"]?.string, arguments["agent_type"]?.string].compactMap { $0 }.joined(separator: " · ")
            return ToolCall(kind: .agent, title: arguments["description"]?.string ?? "Subagent", detail: detail.isEmpty ? nil : detail)
        case "read_agent", "write_agent", "list_agents":
            return ToolCall(kind: .agent, title: Self.humanize(name), detail: arguments["agent_id"]?.string ?? arguments["message"]?.string)
        case "skill":
            return ToolCall(kind: .other, title: "Skill · \(arguments["skill"]?.string ?? "")")
        case "sql":
            return ToolCall(kind: .other, title: arguments["description"]?.string ?? "SQL query", detail: arguments["query"]?.string)
        case "create_pull_request":
            return ToolCall(kind: .other, title: arguments["title"]?.string ?? "Create pull request")
        default:
            return ToolCall(kind: .other, title: Self.humanize(name), detail: arguments.isNull ? nil : TextCleanup.singleLine(arguments.compactString))
        }
    }

    private static func lineRange(_ value: JSONValue?) -> String? {
        guard let range = value?.array?.compactMap(\.int), range.count == 2 else { return nil }
        return range[1] < 0 ? "from line \(range[0])" : "lines \(range[0])–\(range[1])"
    }

    /// The files an `apply_patch` envelope touches: its `*** Update|Add|Delete File:` headers.
    static func patchPaths(_ patch: String) -> [String] {
        var paths: [String] = []
        var seen: Set<String> = []
        for line in patch.split(whereSeparator: \.isNewline) {
            for prefix in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] where line.hasPrefix(prefix) {
                let path = line.dropFirst(prefix.count).trimmingCharacters(in: .whitespaces)
                if !path.isEmpty, seen.insert(path).inserted { paths.append(path) }
            }
        }
        return paths
    }

    /// `update_todo` hands over a markdown checklist: `- [x]` done, `- [ ]` pending,
    /// anything else in the brackets in progress.
    private static let todoLinePattern = #/^(?:[-*+]|\d+[.)])\s*\[(.?)\]\s*(.+)$/#

    static func todos(from markdown: String) -> [TodoStep] {
        markdown.split(whereSeparator: \.isNewline).compactMap { line in
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard let match = trimmed.firstMatch(of: todoLinePattern) else { return nil }
            let status: TodoStep.Status = switch match.output.1.lowercased() {
            case "x": .done
            case " ", "": .pending
            default: .active
            }
            return TodoStep(text: String(match.output.2), status: status)
        }
    }

    private static func reasoningKey(_ text: String) -> Int {
        text.trimmingCharacters(in: .whitespacesAndNewlines).hashValue
    }

    private static func humanize(_ name: String) -> String {
        let words = name.replacingOccurrences(of: "-", with: " ").replacingOccurrences(of: "_", with: " ")
        return words.prefix(1).uppercased() + words.dropFirst()
    }
}
