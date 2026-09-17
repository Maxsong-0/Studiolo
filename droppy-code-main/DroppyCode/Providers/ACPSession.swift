import Foundation

/// Spaces out the starts of ACP agent processes. An agent keeps a store of its own on this
/// Mac (OpenCode's is SQLite), and several starting in the same instant, as a team of heads
/// does, trip over its lock: "database is locked", and a head's first turn is over before
/// it began. A lone start never waits; a batch starts one every `spacing` seconds.
private actor ACPLaunchGate {
    private let spacing: TimeInterval
    private var nextSlot = Date.distantPast

    init(spacing: TimeInterval) {
        self.spacing = spacing
    }

    func wait() async {
        let now = Date.now
        let slot = max(now, nextSlot)
        nextSlot = slot.addingTimeInterval(spacing)
        if slot > now { try? await Task.sleep(for: .seconds(slot.timeIntervalSince(now))) }
    }
}

/// Drives Agent Client Protocol agents: Cursor, OpenCode, Grok and Devin.
@MainActor
final class ACPSession: ProviderSession {
    private static let launchGate = ACPLaunchGate(spacing: 1.5)

    var onEvent: ((ProviderEvent) -> Void)?

    private let configuration: SessionConfiguration
    private var connection: JSONRPCConnection?
    private var sessionID: String?
    private var authMethods: [String] = []
    private var canLoadSessions = false
    private var isReplaying = false
    private var isStopping = false
    private var runtimeMode: RuntimeMode
    private var messageID: String?
    private var thoughtID: String?
    private var counter = 0
    private var pendingPermissions: [String: RPCID] = [:]
    private var promptActive = false
    private var cancelRequested = false
    /// Tool calls already routed to the checklist (opencode `todowrite` arrives as a
    /// generic tool_call, not a plan update). Their later updates keep feeding the
    /// same checklist instead of opening new tool rows.
    private var todoToolCallIDs: Set<String> = []
    /// Resolves the session usage counter into per-event spend.
    private var spendTracker = TokenSpendTracker()

    private var models: [ModelOption] = []
    private var modelConfigID: String?
    private var modeConfigID: String?
    private var effortConfigID: String?
    /// Cursor's Fast toggle (`model_config` / `fast`), when the parameterized picker is on.
    private var fastConfigID: String?
    private var hasModeAPI = false
    private var planMode: String?
    private var buildMode: String?
    private var currentMode: String?
    private var currentModel: String?
    private var currentEffort: String?
    private var currentFast: String?
    /// Cursor only exposes effort/fast selectors when the client opts into its parameterized picker.
    private var usesParameterizedModels: Bool { configuration.provider == .cursor }

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
    }

    var isRunning: Bool {
        guard let connection else { return false }
        return !connection.isClosed
    }

    private var workingDirectory: String { configuration.workingDirectory.path }

    private var launchArguments: [String] {
        switch configuration.provider {
        case .cursor:
            switch configuration.runtimeMode {
            case .auto: ["--auto-review", "acp"]
            case .fullAccess: ["--force", "acp"]
            default: ["acp"]
            }
        case .grok:
            switch configuration.runtimeMode {
            case .supervised: ["--permission-mode", "default", "agent", "stdio"]
            case .autoAcceptEdits: ["--permission-mode", "acceptEdits", "agent", "stdio"]
            case .auto: ["--permission-mode", "auto", "agent", "stdio"]
            case .fullAccess: ["agent", "--always-approve", "stdio"]
            }
        case .devin:
            // Devin's modes at a glance: auto approves read-only tools, accept-edits
            // adds workspace edits, smart adds model-judged-safe actions, dangerous
            // approves everything. Passed before the subcommand, like its CLI expects.
            switch configuration.runtimeMode {
            case .supervised: ["--permission-mode", "auto", "acp"]
            case .autoAcceptEdits: ["--permission-mode", "accept-edits", "acp"]
            case .auto: ["--permission-mode", "smart", "acp"]
            case .fullAccess: ["--permission-mode", "dangerous", "acp"]
            }
        default:
            ["acp"]
        }
    }

    func start() async throws -> String {
        guard let executable = configuration.executable else { throw ProviderError.notInstalled(configuration.provider) }
        await Self.launchGate.wait()
        let process = StdioProcess(
            executable: executable,
            arguments: launchArguments,
            directory: configuration.workingDirectory,
            environment: configuration.environment
        )
        let connection = JSONRPCConnection(process: process, sendsVersion: true)
        connection.onNotification = { [weak self] method, params in self?.handleNotification(method, params) }
        connection.onRequest = { [weak self] id, method, params in self?.handleRequest(id, method, params) }
        connection.onClose = { [weak self] tail in self?.handleClose(tail) }
        self.connection = connection
        try connection.start()

        let initialized = try await connection.request("initialize", [
            "protocolVersion": 1,
            "clientCapabilities": clientCapabilities,
            "clientInfo": ["name": "droppy-code", "title": "Droppy Code", "version": .string(AppInfo.version)],
        ])
        canLoadSessions = initialized["agentCapabilities"]?["loadSession"]?.bool ?? false
        authMethods = (initialized["authMethods"]?.array ?? []).compactMap { $0["id"]?.string }
        if let state = initialized["_meta"]?["modelState"] { applyModels(state) }

        let session: JSONValue
        do {
            session = try await openSession(connection)
        } catch let error as RPCError where error.message.localizedCaseInsensitiveContains("auth") && !authMethods.isEmpty {
            _ = try await connection.request("authenticate", ["methodId": .string(authMethods[0])])
            session = try await openSession(connection)
        }
        applySessionState(session)
        await applySelection(
            model: configuration.model,
            effort: configuration.effort,
            fastMode: configuration.fastMode,
            interaction: configuration.interactionMode
        )
        guard let sessionID else {
            throw ProviderError.failed("\(configuration.provider.displayName) did not start a session.")
        }
        return sessionID
    }

    /// Cursor hides effort and Fast behind an undocumented `_meta` flag; without it the
    /// agent only returns one baked-in variant per model and rejects any other effort.
    private var clientCapabilities: JSONValue {
        var capabilities: [String: JSONValue] = [
            "fs": ["readTextFile": false, "writeTextFile": false],
            "terminal": false,
        ]
        if usesParameterizedModels {
            capabilities["_meta"] = ["parameterizedModelPicker": true]
        }
        return .object(capabilities)
    }

    func send(_ input: TurnInput) async throws {
        guard let connection, !connection.isClosed, let sessionID else { throw ProviderError.notRunning }
        runtimeMode = input.runtimeMode
        await applySelection(
            model: input.model,
            effort: input.effort,
            fastMode: input.fastMode,
            interaction: input.interactionMode
        )
        var prompt: [JSONValue] = [["type": "text", "text": .string(input.text)]]
        for (image, base64) in await Attachment.base64Images(input.images) {
            prompt.append(["type": "image", "mimeType": .string(image.mimeType), "data": .string(base64)])
        }
        messageID = nil
        thoughtID = nil
        promptActive = true
        cancelRequested = false
        onEvent?(.turnStarted(providerTurnID: nil))
        Task { [weak self] in
            do {
                let result = try await connection.request("session/prompt", [
                    "sessionId": .string(sessionID),
                    "prompt": .array(prompt),
                ])
                self?.finishPrompt(result["stopReason"]?.string, error: nil)
            } catch {
                self?.finishPrompt(nil, error: error)
            }
        }
    }

    func interrupt() async {
        guard promptActive, let connection, let sessionID else { return }
        cancelRequested = true
        for (key, id) in pendingPermissions {
            connection.respond(to: id, result: ["outcome": ["outcome": "cancelled"]])
            onEvent?(.requestResolved(id: key))
        }
        pendingPermissions.removeAll()
        connection.notify("session/cancel", ["sessionId": .string(sessionID)])
    }

    func compact() async throws {
        throw ProviderError.failed("\(configuration.provider.displayName) compacts its own context.")
    }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let id = pendingPermissions.removeValue(forKey: requestID) else { return }
        connection?.respond(to: id, result: ["outcome": ["outcome": "selected", "optionId": .string(optionID)]])
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {}

    func stop() {
        isStopping = true
        connection?.close()
    }

    // MARK: - Catalog

    static func probeModels(provider: ProviderKind, executable: URL, environment: [String: String]) async throws -> [ModelOption] {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-code-catalog", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let session = ACPSession(configuration: SessionConfiguration(
            provider: provider,
            executable: executable,
            workingDirectory: directory,
            environment: environment,
            runtimeMode: .supervised,
            interactionMode: .build
        ))
        var models: [ModelOption] = []
        session.onEvent = { event in
            if case .models(let list, _) = event { models = list }
        }
        defer { session.stop() }
        _ = try await session.start()
        await session.probeModelEfforts()
        return models
    }

    // MARK: - Session state

    private func openSession(_ connection: JSONRPCConnection) async throws -> JSONValue {
        let base: [String: JSONValue] = ["cwd": .string(workingDirectory), "mcpServers": []]
        if let resumeID = configuration.resumeID, canLoadSessions {
            var params = base
            params["sessionId"] = .string(resumeID)
            isReplaying = true
            let result = try? await connection.request("session/load", .object(params))
            isReplaying = false
            if let result {
                sessionID = resumeID
                return result
            }
        }
        let result = try await connection.request("session/new", .object(base))
        sessionID = result["sessionId"]?.string
        return result
    }

    private func applySessionState(_ state: JSONValue) {
        if let modes = state["modes"], !modes.isNull {
            hasModeAPI = true
            currentMode = modes["currentModeId"]?.string
            resolveModes((modes["availableModes"]?.array ?? []).compactMap { $0["id"]?.string })
        }
        if let modelState = state["models"], !modelState.isNull { applyModels(modelState) }
        if let options = state["configOptions"]?.array { applyConfigOptions(options) }
    }

    /// Cursor reports the effort scale of the current model only, so the catalog probe
    /// walks the list: each model without a scale yet is made current in turn and its
    /// scale read off the reply, and the model the session started on is put back.
    private func probeModelEfforts() async {
        guard usesParameterizedModels, let modelConfigID else { return }
        let original = currentModel
        for model in models where model.efforts.isEmpty {
            await setConfigOption(modelConfigID, value: model.id)
            currentModel = model.id
        }
        if let original, !Self.sameModel(original, currentModel) {
            await setConfigOption(modelConfigID, value: original)
            currentModel = original
        }
    }

    private func resolveModes(_ ids: [String]) {
        planMode = ids.first { $0 == "plan" }
        buildMode = ids.first { ["agent", "build", "code", "default"].contains($0) }
            ?? ids.first { $0 != "plan" && $0 != "ask" }
    }

    private func applyModels(_ state: JSONValue) {
        let current = state["currentModelId"]?.string.map(Self.baseModelID)
        currentModel = current ?? currentModel
        let list = (state["availableModels"]?.array ?? []).compactMap { entry -> ModelOption? in
            guard let rawID = entry["modelId"]?.string else { return nil }
            let id = usesParameterizedModels ? Self.baseModelID(rawID) : rawID
            let efforts = entry["_meta"]?["reasoningEfforts"]?.array ?? []
            return ModelOption(
                id: id,
                name: entry["name"]?.string ?? id,
                detail: entry["description"]?.string,
                efforts: Self.orderedEfforts(efforts.compactMap { $0["value"]?.string }),
                defaultEffort: efforts.first { $0["default"]?.bool == true }?["value"]?.string,
                isDefault: id == current
            )
        }
        guard !list.isEmpty else { return }
        models = list
        onEvent?(.models(list, current: currentModel))
    }

    private func applyConfigOptions(_ options: [JSONValue]) {
        var efforts: [String] = []
        var hasFast = false
        effortConfigID = nil
        fastConfigID = nil
        for option in options {
            guard let id = option["id"]?.string else { continue }
            let values = Self.flatten(option["options"]?.array ?? [])
            let current = option["currentValue"]?.string
            switch option["category"]?.string ?? id {
            case "model":
                modelConfigID = id
                currentModel = current.map { usesParameterizedModels ? Self.baseModelID($0) : $0 } ?? currentModel
                let list = values.compactMap { value -> ModelOption? in
                    guard let rawID = value["value"]?.string else { return nil }
                    let valueID = usesParameterizedModels ? Self.baseModelID(rawID) : rawID
                    // A rebuilt list keeps what earlier passes learned about each model (its
                    // scale, its Fast tier): Cursor reports them for the current model only,
                    // one pass at a time.
                    let known = models.first { Self.sameModel($0.id, valueID) }
                    var option = ModelOption(
                        id: valueID,
                        name: value["name"]?.string ?? valueID,
                        detail: value["description"]?.string,
                        efforts: known?.efforts ?? [],
                        defaultEffort: known?.defaultEffort,
                        isDefault: valueID == currentModel
                    )
                    option.fastTier = known?.fastTier
                    return option
                }
                if !list.isEmpty { models = list }
            case "mode":
                modeConfigID = id
                currentMode = current ?? currentMode
                resolveModes(values.compactMap { $0["value"]?.string })
            case "thought_level", "effort":
                // Cursor also advertises a boolean `thinking` thought_level; only the
                // scale (low/medium/high/…) drives Droppy's effort slider.
                let scale = values.compactMap { $0["value"]?.string }
                guard Self.isEffortScale(scale) else { break }
                effortConfigID = id
                currentEffort = current
                efforts = Self.orderedEfforts(scale)
            case "model_config":
                if id == "fast" {
                    fastConfigID = id
                    currentFast = current
                    hasFast = true
                }
            default:
                break
            }
        }
        models = models.map { model in
            var model = model
            let isCurrent = Self.sameModel(model.id, currentModel)
            if isCurrent, !efforts.isEmpty {
                model.efforts = efforts
                model.defaultEffort = currentEffort ?? model.defaultEffort
            }
            if isCurrent {
                model.fastTier = hasFast ? "true" : nil
            }
            return model
        }
        if !models.isEmpty { onEvent?(.models(models, current: currentModel)) }
    }

    private static func flatten(_ values: [JSONValue]) -> [JSONValue] {
        values.flatMap { value -> [JSONValue] in
            if let nested = value["options"]?.array { return nested }
            return [value]
        }
    }

    /// Values that belong on the effort slider, as opposed to a boolean thinking toggle.
    private static func isEffortScale(_ values: [String]) -> Bool {
        let known: Set<String> = ["none", "low", "medium", "high", "xhigh", "max", "extra-high"]
        return values.contains { known.contains($0.lowercased()) }
    }

    /// The scale low to high, whatever order the agent lists it in: Grok's ACP advertises
    /// its levels from extra high down to low, and the slider (see `EffortSlider`) reads
    /// the list left to right as low to high, so its knob sat at Low for the highest. A
    /// level the ranks do not know keeps its place among its neighbours.
    static func orderedEfforts(_ values: [String]) -> [String] {
        let ranks: [String: Int] = [
            "none": 0, "minimal": 1, "low": 2, "medium": 3, "high": 4,
            "xhigh": 5, "extra-high": 5, "extra_high": 5, "extra high": 5, "max": 6, "ultra": 6,
        ]
        let ranked = values.enumerated().map { (offset: $0.offset, value: $0.element, rank: ranks[$0.element.lowercased()]) }
        guard ranked.count { $0.rank != nil } > 1 else { return values }
        return ranked.sorted { lhs, rhs in
            switch (lhs.rank, rhs.rank) {
            case (let l?, let r?) where l != r: return l < r
            default: return lhs.offset < rhs.offset
            }
        }.map(\.value)
    }

    /// Cursor's parameterized IDs are `model[effort=high,…]`; the picker stores the base name.
    private static func baseModelID(_ id: String) -> String {
        guard let bracket = id.firstIndex(of: "[") else { return id }
        return String(id[..<bracket])
    }

    private static func sameModel(_ lhs: String?, _ rhs: String?) -> Bool {
        guard let lhs, let rhs else { return false }
        return baseModelID(lhs) == baseModelID(rhs)
    }

    private func applySelection(model: String?, effort: String?, fastMode: Bool, interaction: InteractionMode) async {
        guard let connection, let sessionID else { return }
        if let model, !model.isEmpty {
            let wireID = usesParameterizedModels ? Self.baseModelID(model) : model
            if !Self.sameModel(wireID, currentModel) {
                if let modelConfigID {
                    await setConfigOption(modelConfigID, value: wireID)
                } else {
                    _ = try? await connection.request("session/set_model", ["sessionId": .string(sessionID), "modelId": .string(wireID)])
                }
                currentModel = wireID
            }
        }
        if let effort, !effort.isEmpty, let effortConfigID, effort != currentEffort {
            await setConfigOption(effortConfigID, value: effort)
            currentEffort = effort
        }
        if let fastConfigID {
            let wanted = fastMode ? "true" : "false"
            if wanted != currentFast {
                await setConfigOption(fastConfigID, value: wanted)
                currentFast = wanted
            }
        }
        if let mode = interaction == .plan ? planMode : buildMode, mode != currentMode {
            if let modeConfigID {
                await setConfigOption(modeConfigID, value: mode)
            } else if hasModeAPI {
                _ = try? await connection.request("session/set_mode", ["sessionId": .string(sessionID), "modeId": .string(mode)])
            }
            currentMode = mode
        }
    }

    private func setConfigOption(_ configID: String, value: String) async {
        guard let connection, let sessionID else { return }
        let result = try? await connection.request("session/set_config_option", [
            "sessionId": .string(sessionID),
            "configId": .string(configID),
            "value": .string(value),
        ])
        if let options = result?["configOptions"]?.array { applyConfigOptions(options) }
    }

    // MARK: - Updates

    private func handleNotification(_ method: String, _ params: JSONValue) {
        guard method == "session/update", !isReplaying, let update = params["update"] else { return }
        switch update["sessionUpdate"]?.string {
        case "agent_message_chunk":
            guard let text = update["content"]?["text"]?.string else { return }
            thoughtID = nil
            let id = update["messageId"]?.string.map { "message-\($0)" } ?? messageID ?? nextID("message")
            messageID = id
            onEvent?(.messageDelta(id: id, text: text))
        case "agent_thought_chunk":
            guard let text = update["content"]?["text"]?.string else { return }
            let id = thoughtID ?? nextID("thought")
            thoughtID = id
            onEvent?(.reasoningDelta(id: id, text: text))
        case "tool_call":
            guard let id = update["toolCallId"]?.string else { return }
            messageID = nil
            thoughtID = nil
            if Self.isTodoCall(update) {
                todoToolCallIDs.insert(id)
                let steps = Self.todoSteps(update)
                if !steps.isEmpty { onEvent?(.todos(steps)) }
                return
            }
            onEvent?(.toolStarted(id: id, call: makeToolCall(update)))
            let initial = makeToolUpdate(update)
            if initial.output != nil || initial.status != nil { onEvent?(.toolUpdated(id: id, update: initial)) }
        case "tool_call_update":
            guard let id = update["toolCallId"]?.string else { return }
            if todoToolCallIDs.contains(id) || Self.isTodoCall(update) {
                todoToolCallIDs.insert(id)
                let steps = Self.todoSteps(update)
                if !steps.isEmpty { onEvent?(.todos(steps)) }
                return
            }
            onEvent?(.toolUpdated(id: id, update: makeToolUpdate(update)))
        case "plan":
            let steps = Self.parseTodoItems(update["entries"])
            onEvent?(.todos(steps))
        case "available_commands_update":
            let commands = (update["availableCommands"]?.array ?? []).compactMap { command -> SlashCommand? in
                guard let name = command["name"]?.string else { return nil }
                return SlashCommand(name: name, detail: command["description"]?.string ?? "")
            }
            onEvent?(.commands(commands))
        case "current_mode_update":
            currentMode = update["currentModeId"]?.string
            if let planMode { onEvent?(.modeChanged(currentMode == planMode ? .plan : .build)) }
        case "config_option_update":
            if let options = update["configOptions"]?.array { applyConfigOptions(options) }
        case "session_info_update":
            if let title = update["title"]?.string, !title.isEmpty { onEvent?(.title(title)) }
        case "usage_update":
            if let used = update["used"]?.int {
                onEvent?(.usage(ContextUsage(usedTokens: used, windowTokens: update["size"]?.int)))
                let spend = spendTracker.spend(total: used)
                if spend > 0 { TokenLedger.shared.record(spend: spend) }
            }
        default:
            break
        }
    }

    private func handleRequest(_ id: RPCID, _ method: String, _ params: JSONValue) {
        guard method == "session/request_permission" else {
            connection?.respond(to: id, errorCode: -32601, message: "Method not found")
            return
        }
        let options = params["options"]?.array ?? []
        let toolCall = params["toolCall"] ?? .null
        let call = makeToolCall(toolCall)
        if let optionID = automaticOption(for: call.kind, options: options) {
            connection?.respond(to: id, result: ["outcome": ["outcome": "selected", "optionId": .string(optionID)]])
            return
        }
        let mapped = options.compactMap { option -> ApprovalRequest.Option? in
            guard let optionID = option["optionId"]?.string else { return nil }
            let role: ApprovalRequest.Option.Role = switch option["kind"]?.string {
            case "allow_always": .approveAlways
            case "reject_once", "reject_always": .decline
            default: .approve
            }
            return .init(id: optionID, title: option["name"]?.string ?? optionID, role: role)
        }
        pendingPermissions[id.key] = id
        let kind: ApprovalRequest.Kind = switch call.kind {
        case .command: .command
        case .edit: .fileChange
        default: .tool
        }
        onEvent?(.approval(ApprovalRequest(
            id: id.key,
            kind: kind,
            title: call.title,
            detail: call.detail,
            options: mapped,
            toolItemID: toolCall["toolCallId"]?.string
        )))
    }

    private func automaticOption(for kind: ToolCall.Kind, options: [JSONValue]) -> String? {
        let automatic = switch runtimeMode {
        case .fullAccess: true
        case .autoAcceptEdits: kind == .edit
        default: false
        }
        guard automatic else { return nil }
        let allowOnce = options.first { $0["kind"]?.string == "allow_once" }
        let allowAlways = options.first { $0["kind"]?.string == "allow_always" }
        return (allowOnce ?? allowAlways)?["optionId"]?.string
    }

    private func finishPrompt(_ stopReason: String?, error: Error?) {
        guard promptActive else { return }
        promptActive = false
        todoToolCallIDs.removeAll()
        for (key, id) in pendingPermissions {
            connection?.respond(to: id, result: ["outcome": ["outcome": "cancelled"]])
            onEvent?(.requestResolved(id: key))
        }
        pendingPermissions.removeAll()
        if cancelRequested || stopReason == "cancelled" {
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
        } else if let error {
            onEvent?(.turnCompleted(status: .failed, error: error.localizedDescription))
        } else if stopReason == "refusal" {
            onEvent?(.turnCompleted(status: .failed, error: "The agent declined this request."))
        } else {
            onEvent?(.turnCompleted(status: .completed, error: nil))
        }
        cancelRequested = false
    }

    private func handleClose(_ tail: String) {
        if promptActive {
            finishPrompt(nil, error: ProviderError.failed(TextCleanup.lastLines(tail) ?? "The agent exited."))
        }
        guard !isStopping else { return }
        onEvent?(.exited(error: TextCleanup.lastLines(tail)))
    }

    // MARK: - Mapping

    /// True when a tool_call update belongs to a todo-list tool. opencode's `todowrite`
    /// reports itself with a count title ("3 todos") and the JSON list as its output,
    /// never as a plan update, so it is detected here instead of in `makeToolCall`.
    private static func isTodoCall(_ update: JSONValue) -> Bool {
        if update["rawInput"]?["todos"] != nil { return true }
        if let title = update["title"]?.string, isTodoTitle(title) { return true }
        return false
    }

    private static func isTodoTitle(_ title: String) -> Bool {
        let text = title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch text {
        case "todo", "todos", "todowrite", "todo write", "write todos":
            return true
        default:
            break
        }
        // opencode titles the call "<n> todos", e.g. "3 todos".
        for suffix in [" todos", " todo"] {
            guard text.hasSuffix(suffix) else { continue }
            let count = text.dropLast(suffix.count).trimmingCharacters(in: .whitespaces)
            if !count.isEmpty, count.allSatisfy(\.isNumber) { return true }
        }
        return false
    }

    /// The checklist carried by a todo tool call: structured input first, then the
    /// JSON list the tool reports back as its result text.
    private static func todoSteps(_ update: JSONValue) -> [TodoStep] {
        var steps = parseTodoItems(update["rawInput"]?["todos"])
        if !steps.isEmpty { return steps }
        steps = parseTodoItems(update["rawOutput"]?["todos"])
        if !steps.isEmpty { return steps }
        for block in update["content"]?.array ?? [] {
            guard block["type"]?.string == "content",
                  let text = block["content"]?["text"]?.string,
                  let json = JSONValue.parse(text) else { continue }
            steps = parseTodoItems(json["todos"] ?? json)
            if !steps.isEmpty { return steps }
        }
        if let raw = update["rawOutput"], !raw.isNull {
            for key in ["output", "stdout", "text"] {
                guard let text = raw[key]?.string, let json = JSONValue.parse(text) else { continue }
                steps = parseTodoItems(json["todos"] ?? json)
                if !steps.isEmpty { return steps }
            }
        }
        return []
    }

    /// Todo items across provider shapes: ACP plan entries (`content`), Claude's
    /// TodoWrite (`content`) and Codex-style steps (`step`). Unknown statuses stay
    /// pending so a new provider state never drops an item.
    static func parseTodoItems(_ value: JSONValue?) -> [TodoStep] {
        (value?.array ?? []).compactMap { item -> TodoStep? in
            guard let text = item["content"]?.string ?? item["text"]?.string ?? item["step"]?.string ?? item["title"]?.string,
                  !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            let status: TodoStep.Status = switch item["status"]?.string {
            case "completed", "complete", "done": .done
            case "in_progress", "inProgress", "in-progress", "active": .active
            default: .pending
            }
            return TodoStep(text: text, status: status)
        }
    }

    private func nextID(_ prefix: String) -> String {
        counter += 1
        return "\(prefix)-\(counter)-\(UUID().uuidString.prefix(8))"
    }

    /// Same row shapes as Claude's: the subject is the command, the path, the
    /// pattern or the URL, and the detail only qualifies it. ACP agents title
    /// calls freely ("Fetched https://…", "Search for 'x'", a bare "read"), so
    /// the subject comes from `rawInput` and `locations` first and the title
    /// is the fallback, with its leading verb dropped.
    private func makeToolCall(_ update: JSONValue) -> ToolCall {
        let kind = Self.kind(update["kind"]?.string)
        let raw = update["rawInput"] ?? .null
        let location = update["locations"]?.array?.first?["path"]?.string.map { ToolTitles.relativePath($0, to: workingDirectory) }
        let path = Self.string(in: raw, keys: Self.pathKeys).map { ToolTitles.relativePath($0, to: workingDirectory) } ?? location
        let title = Self.subject(from: update["title"]?.string, kind: kind)
        var call = ToolCall(kind: kind, title: "")
        switch kind {
        case .command:
            call.title = raw["command"]?.string.map(ToolTitles.unwrapShell) ?? title
        case .read, .edit:
            call.title = path ?? title
        case .search:
            let pattern = Self.string(in: raw, keys: ["pattern", "query", "glob", "regex", "search"])
            call.title = pattern ?? title.nilIfEmpty ?? path ?? ""
            if pattern != nil || !title.isEmpty, let path, path != workingDirectory, path != call.title { call.detail = path }
        case .web:
            call.title = Self.string(in: raw, keys: ["url", "query", "prompt"]) ?? title
        case .mcp, .agent, .other:
            call.title = title
            if let path, path != workingDirectory, path != call.title { call.detail = path }
        }
        if call.title.isEmpty { call.title = path ?? Self.fallbackTitle(kind) }
        call.edits = diffs(update["content"])
        if let status = update["status"]?.string { call.status = Self.status(status) }
        return call
    }

    private func makeToolUpdate(_ update: JSONValue) -> ToolUpdate {
        var result = ToolUpdate()
        if let kind = update["kind"]?.string { result.kind = Self.kind(kind) }
        let kind = result.kind ?? .other
        let title = Self.subject(from: update["title"]?.string, kind: kind)
        // A bare tool-name title ("Edit file") must not overwrite a real path.
        if !title.isEmpty, !Self.isFallbackTitle(title) { result.title = title }
        if let command = update["rawInput"]?["command"]?.string { result.title = ToolTitles.unwrapShell(command) }
        // A path that only arrives with the update still names the row.
        if kind == .read || kind == .edit, result.title == nil,
           let path = update["locations"]?.array?.first?["path"]?.string {
            let relative = ToolTitles.relativePath(path, to: workingDirectory)
            if relative != workingDirectory { result.title = relative }
        }
        if let status = update["status"]?.string { result.status = Self.status(status) }
        let edits = diffs(update["content"])
        if !edits.isEmpty { result.edits = edits }
        var output = (update["content"]?.array ?? [])
            .compactMap { $0["type"]?.string == "content" ? $0["content"]?["text"]?.string : nil }
            .joined(separator: "\n")
        if let raw = update["rawOutput"], !raw.isNull {
            if output.isEmpty {
                output = [raw["stdout"]?.string, raw["stderr"]?.string, raw["output"]?.string]
                    .compactMap { $0 }
                    .filter { !$0.isEmpty }
                    .joined(separator: "\n")
            }
            result.exitCode = raw["exitCode"]?.int ?? raw["metadata"]?["exit"]?.int
        }
        if !output.isEmpty, output != "(no output)" { result.output = output }
        if let exitCode = result.exitCode, exitCode != 0, result.status == .completed { result.status = .failed }
        return result
    }

    private static let pathKeys = ["filePath", "file_path", "path", "file", "notebook_path", "target_file", "targetFile"]

    private static func string(in value: JSONValue, keys: [String]) -> String? {
        for key in keys {
            if let text = value[key]?.string?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty { return text }
        }
        return nil
    }

    /// The title without the verb an agent prefixed it with, so the row reads
    /// "Browsed https://…" rather than "Browsed Fetched https://…". Verbs are
    /// only dropped when something remains; a bare tool name ("read", "glob")
    /// becomes empty so a path or pattern takes its place.
    /// Longest phrases first, so "searched the web for" wins over "searched for". Each
    /// carries its trailing space, matched as is against the lowercased title.
    private static let subjectPhrases = [
        "searched web for ", "search web for ", "searching web for ", "searched the web for ", "search the web for ",
        "searched for ", "searching for ", "search for ", "reading file ", "read file ", "listed directory ",
        "listing directory ", "list directory ", "listed files in ", "list files in ", "listing files in ",
    ]
    private static let subjectVerbs: Set<String> = [
        "read", "reading", "viewed", "viewing", "view", "fetched", "fetching", "fetch", "browsed", "browsing",
        "searched", "searching", "search", "grep", "glob", "listed", "listing", "list", "wrote", "writing",
        "write", "edited", "editing", "edit", "modified", "modifying", "modify", "updated", "updating", "update",
        "created", "creating", "create", "deleted", "deleting", "delete", "ran", "running", "run", "executed",
        "executing", "execute", "called", "calling", "call", "opened", "opening", "open",
    ]

    private static func subject(from title: String?, kind: ToolCall.Kind) -> String {
        var text = cleanTitle(title)
        guard !text.isEmpty else { return "" }
        if isBareToolName(text) { return "" }
        let phrases = subjectPhrases
        let verbs = subjectVerbs
        let lowered = text.lowercased()
        var stripped = false
        for phrase in phrases where [.read, .edit, .search, .web].contains(kind) && lowered.hasPrefix(phrase) {
            text = String(text.dropFirst(phrase.count))
            stripped = true
            break
        }
        let rowHasVerb = [.read, .edit, .search, .web].contains(kind)
        if !stripped, rowHasVerb, let head = lowered.split(separator: " ", maxSplits: 1).first, verbs.contains(String(head)) {
            let rest = text.dropFirst(head.count).trimmingCharacters(in: .whitespaces)
            if !rest.isEmpty { text = rest }
        }
        text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        // "'devin'" / "“x”" → x
        for (open, close) in [("'", "'"), ("\"", "\""), ("“", "”"), ("`", "`")]
        where text.count >= 2 && text.hasPrefix(open) && text.hasSuffix(close) {
            text = String(text.dropFirst().dropLast())
        }
        return text
    }

    /// A title that is just the tool's identifier: one lowercase token like
    /// "read", "webfetch" or "todowrite".
    private static func isBareToolName(_ title: String) -> Bool {
        let bare: Set<String> = [
            "read", "write", "edit", "multiedit", "patch", "bash", "shell", "list", "ls", "glob", "grep", "search",
            "webfetch", "websearch", "fetch", "todowrite", "todoread", "task", "tool", "apply_patch", "read_file",
            "write_file", "edit_file", "list_files", "search_files", "run_command", "run_terminal_cmd", "web_search",
            "web_fetch", "codebase_search", "file_search", "grep_search", "list_dir", "view_file", "replace_file_content",
        ]
        return bare.contains(title.lowercased())
    }

    private func diffs(_ content: JSONValue?) -> [FileEdit] {
        (content?.array ?? []).compactMap { block -> FileEdit? in
            guard block["type"]?.string == "diff", let path = block["path"]?.string else { return nil }
            return FileEdit(
                path: ToolTitles.relativePath(path, to: workingDirectory),
                old: block["oldText"]?.string ?? "",
                new: block["newText"]?.string ?? ""
            )
        }
    }

    private static func cleanTitle(_ title: String?) -> String {
        (title ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "` \n"))
    }

    private static func kind(_ raw: String?) -> ToolCall.Kind {
        switch raw {
        case "read": .read
        case "edit", "delete", "move": .edit
        case "search": .search
        case "execute": .command
        case "fetch": .web
        default: .other
        }
    }

    private static func fallbackTitle(_ kind: ToolCall.Kind) -> String {
        switch kind {
        case .command: "Run command"
        case .read: "Read file"
        case .edit: "Edit file"
        case .search: "Search"
        case .web: "Fetch"
        case .mcp: "Tool"
        case .agent: "Agent"
        case .other: "Tool"
        }
    }

    /// True when a call's title is just the tool's display name — no subject.
    private static func isFallbackTitle(_ title: String) -> Bool {
        switch title.lowercased() {
        case "run command", "read file", "edit file", "write file", "search", "fetch", "tool", "agent", "file", "files":
            true
        default:
            isBareToolName(title)
        }
    }

    private static func status(_ raw: String) -> ToolCall.Status {
        switch raw {
        case "completed": .completed
        case "failed": .failed
        default: .running
        }
    }
}

extension SessionConfiguration {
    init(
        provider: ProviderKind,
        executable: URL,
        workingDirectory: URL,
        environment: [String: String],
        runtimeMode: RuntimeMode,
        interactionMode: InteractionMode
    ) {
        self.init(
            provider: provider,
            executable: executable,
            workingDirectory: workingDirectory,
            environment: environment,
            resumeID: nil,
            resumeAt: nil,
            model: nil,
            effort: nil,
            runtimeMode: runtimeMode,
            interactionMode: interactionMode
        )
    }
}
