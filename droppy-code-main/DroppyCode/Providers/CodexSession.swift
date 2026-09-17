import Foundation
import OSLog

/// Drives `codex app-server` over JSON-RPC.
///
/// With Hydra on, the thread starts with config overrides that put the heads on the pair's
/// model and effort and cap how many run at once, and with developer instructions that
/// steer the lead towards `spawn_agent`. A head is a child thread of the app-server:
/// `thread/started` announces it with the lead as its parent, its own notifications carry
/// its thread id and go out wrapped in `agentEvent`, and its `turn/completed` ends it.
@MainActor
final class CodexSession: ProviderSession {
    private static let log = Logger(subsystem: "iordv.droppycode", category: "codex")
    var onEvent: ((ProviderEvent) -> Void)?

    /// Whose transcript a notification belongs to.
    private enum Target {
        case lead
        case head(String)
    }

    private enum PendingRequest {
        case decision(RPCID, payloads: [String: JSONValue])
        case permissions(RPCID, requested: JSONValue)
        case question(RPCID)
    }

    private struct PolicySettings {
        var approvalPolicy: JSONValue
        var sandbox: JSONValue
        var sandboxPolicy: JSONValue
        var reviewer: JSONValue
    }

    private let configuration: SessionConfiguration
    private var connection: JSONRPCConnection?
    private var threadID: String?
    private var turnID: String?
    private var activeModel: String?
    private var pendingRequests: [String: PendingRequest] = [:]
    private var isStopping = false
    /// Resolves the cumulative thread total into per-event spend for the ledger.
    private var spendTracker = TokenSpendTracker()
    /// The heads' threads, spawned under this one, with each one's running turn (for
    /// stopping it) and its own spend tracker.
    private var headThreads: Set<String> = []
    private var headTurns: [String: String] = [:]
    private var headSpend: [String: TokenSpendTracker] = [:]
    /// The lead's event sink, made once rather than once per notification.
    private lazy var leadSink: (ProviderEvent) -> Void = { [weak self] event in self?.onEvent?(event) }

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
    }

    var isRunning: Bool {
        guard let connection else { return false }
        return !connection.isClosed
    }

    private var workingDirectory: String { configuration.workingDirectory.path }

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
        var started = false
        defer {
            if !started {
                connection.close()
                self.connection = nil
            }
        }
        let params = threadParameters

        if let resumeID = configuration.resumeID {
            var resume = params
            resume["threadId"] = .string(resumeID)
            resume["excludeTurns"] = true
            let result = try await connection.request("thread/resume", .object(resume))
            guard let id = result["thread"]?["id"]?.string, id == resumeID else {
                throw ProviderError.failed("Codex could not resume the original conversation.")
            }
            threadID = id
            activeModel = result["model"]?.string
            started = true
            return id
        }
        let result = try await connection.request("thread/start", .object(params))
        guard let id = result["thread"]?["id"]?.string else {
            throw ProviderError.failed("Codex did not start a thread.")
        }
        threadID = id
        activeModel = result["model"]?.string
        started = true
        return id
    }

    func send(_ input: TurnInput) async throws {
        guard let connection, !connection.isClosed, let threadID else { throw ProviderError.notRunning }
        var content: [JSONValue] = input.command?.codexInput(for: input.text) ?? [["type": "text", "text": .string(input.text)]]
        for image in input.images where image.isImage {
            content.append(["type": "localImage", "path": .string(image.path)])
        }
        let policy = policySettings(input.runtimeMode)
        var params: [String: JSONValue] = [
            "threadId": .string(threadID),
            "input": .array(content),
            "approvalPolicy": policy.approvalPolicy,
            "approvalsReviewer": policy.reviewer,
            "sandboxPolicy": policy.sandboxPolicy,
            "summary": "auto",
        ]
        let effort = input.effort.flatMap { $0.isEmpty ? nil : $0 }
        if let effort { params["effort"] = .string(effort) }
        if let tier = input.serviceTier { params["serviceTierForTurn"] = .string(tier) }
        if let model = input.model ?? activeModel {
            params["model"] = .string(model)
            params["collaborationMode"] = [
                "mode": input.interactionMode == .plan ? "plan" : "default",
                "settings": [
                    "model": .string(model),
                    "reasoning_effort": .optional(effort),
                    "developer_instructions": .null,
                ],
            ]
        }
        let result = try await connection.request("turn/start", .object(params))
        if let id = result["turn"]?["id"]?.string { turnID = id }
    }

    func interrupt() async {
        guard let connection, let threadID, let turnID else { return }
        _ = try? await connection.request("turn/interrupt", [
            "threadId": .string(threadID),
            "turnId": .string(turnID),
        ])
    }

    func compact() async throws {
        guard let connection, let threadID else { throw ProviderError.notRunning }
        _ = try await connection.request("thread/compact/start", ["threadId": .string(threadID)])
    }

    func rollback(from targetID: String) async throws -> String? {
        guard let connection, let threadID else { throw ProviderError.notRunning }
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(30))
            if !Task.isCancelled { connection.close() }
        }
        defer { watchdog.cancel() }
        var cursor: String?
        var count = 0
        var found = false
        var previousID: String?
        repeat {
            let page = try await connection.request("thread/turns/list", [
                "threadId": .string(threadID), "cursor": .optional(cursor),
                "limit": 100, "sortDirection": "desc", "itemsView": "notLoaded",
            ])
            guard let turns = page["data"]?.array else {
                throw ProviderError.failed("Codex did not return its conversation history.")
            }
            for turn in turns {
                guard let id = turn["id"]?.string else { throw ProviderError.failed("Codex returned a turn without an ID.") }
                if found { previousID = id; break }
                count += 1
                if id == targetID { found = true }
            }
            let next = page["nextCursor"]?.string
            guard next == nil || next != cursor else { throw ProviderError.failed("Codex could not page its conversation history.") }
            cursor = next
        } while cursor != nil && previousID == nil
        guard found else { throw ProviderError.failed("Codex could not find the selected turn. The conversation was kept.") }
        let result: JSONValue
        do {
            result = try await connection.request("thread/rollback", [
                "threadId": .string(threadID),
                "numTurns": .int(count),
            ])
        } catch let error as RPCError where error.message.contains("paginated threads do not support thread/rollback") {
            return try await forkHistory(before: targetID, keeping: previousID, connection: connection, originalID: threadID)
        }
        guard result["thread"]?["id"]?.string == threadID,
              let remaining = result["thread"]?["turns"]?.array,
              !remaining.contains(where: { $0["id"]?.string == targetID }),
              remaining.last?["id"]?.string == previousID else {
            throw ProviderError.failed("Codex did not confirm the requested conversation rollback.")
        }
        turnID = nil
        return threadID
    }

    private var threadParameters: [String: JSONValue] {
        let policy = policySettings(configuration.runtimeMode)
        var params: [String: JSONValue] = [
            "cwd": .string(workingDirectory),
            "approvalPolicy": policy.approvalPolicy,
            "sandbox": policy.sandbox,
            "approvalsReviewer": policy.reviewer,
        ]
        if let model = configuration.model { params["model"] = .string(model) }
        if let hydra = configuration.hydra {
            if hydra.runsNatively {
                params["config"] = .object(HydraPrompts.codexConfig(hydra))
                params["developerInstructions"] = .string(HydraPrompts.policy(for: .codex, maxHeads: hydra.maxHeads, autoMerges: hydra.autoMerges, reviewsHeads: hydra.reviewsHeads))
            } else {
                // Heads on another provider are Droppy-run: the lead asks for them with the
                // delegation block, and Codex's own agents are switched off for the thread,
                // whatever the user's config enables. Told only in words, a lead still
                // reached for spawn_agent, ran its heads on itself and the pair's model never
                // saw them.
                params["config"] = ["features": ["multi_agent": false]]
                params["developerInstructions"] = .string(HydraPrompts.fallbackPolicy(hydra))
            }
        }
        return params
    }

    private func forkHistory(before targetID: String, keeping previousID: String?, connection: JSONRPCConnection, originalID: String) async throws -> String? {
        guard let previousID else {
            _ = try await connection.request("thread/archive", ["threadId": .string(originalID)])
            threadID = nil
            turnID = nil
            return nil
        }
        var params = threadParameters
        params["threadId"] = .string(originalID)
        params["lastTurnId"] = .string(previousID)
        params["excludeTurns"] = true
        let result = try await connection.request("thread/fork", .object(params))
        guard let id = result["thread"]?["id"]?.string, id != originalID else {
            throw ProviderError.failed("Codex did not create a conversation at the revert point.")
        }
        do {
            let page = try await connection.request("thread/turns/list", [
                "threadId": .string(id), "limit": 1, "sortDirection": "desc", "itemsView": "notLoaded",
            ])
            guard let turns = page["data"]?.array,
                  turns.first?["id"]?.string == previousID,
                  turns.first?["id"]?.string != targetID else {
                throw ProviderError.failed("Codex did not confirm the reverted conversation history.")
            }
            _ = try await connection.request("thread/archive", ["threadId": .string(originalID)])
        } catch {
            _ = try? await connection.request("thread/archive", ["threadId": .string(id)])
            throw error
        }
        threadID = id
        turnID = nil
        activeModel = result["model"]?.string ?? activeModel
        return id
    }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let pending = pendingRequests.removeValue(forKey: requestID) else { return }
        switch pending {
        case .decision(let id, let payloads):
            connection?.respond(to: id, result: ["decision": payloads[optionID] ?? "cancel"])
        case .permissions(let id, let requested):
            if optionID == "turn" || optionID == "session" {
                connection?.respond(to: id, result: ["permissions": requested, "scope": .string(optionID)])
            } else {
                connection?.respond(to: id, result: ["permissions": [:], "scope": "turn"])
            }
        case .question(let id):
            connection?.respond(to: id, result: ["answers": [:]])
        }
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {
        guard case .question(let id)? = pendingRequests.removeValue(forKey: requestID) else { return }
        var payload: [String: JSONValue] = [:]
        for (questionID, values) in answers {
            payload[questionID] = ["answers": .array(values.map(JSONValue.string))]
        }
        connection?.respond(to: id, result: ["answers": .object(payload)])
        onEvent?(.requestResolved(id: requestID))
    }

    func stop() {
        isStopping = true
        connection?.close()
    }

    func stopAgent(_ id: String) async -> Bool {
        guard let connection, headThreads.contains(id), let turnID = headTurns[id] else { return false }
        return (try? await connection.request("turn/interrupt", [
            "threadId": .string(id),
            "turnId": .string(turnID),
        ])) != nil
    }

    // MARK: - Catalog

    static func listCommands(executable: URL, directory: URL, environment: [String: String]) async throws -> [SlashCommand] {
        let connection = try await connect(executable: executable, directory: directory, environment: environment, configure: { _ in })
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(20))
            if !Task.isCancelled { connection.close() }
        }
        defer { watchdog.cancel(); connection.close() }
        return SlashCommand.codexSkills(try await connection.request("skills/list", ["cwds": [.string(directory.path)], "forceReload": true]))
    }

    static func listModels(executable: URL, environment: [String: String]) async throws -> [ModelOption] {
        let connection = try await connect(
            executable: executable,
            directory: FileManager.default.temporaryDirectory,
            environment: environment,
            configure: { _ in }
        )
        defer { connection.close() }
        var models: [ModelOption] = []
        var cursor: JSONValue = .null
        for _ in 0..<10 {
            var params: [String: JSONValue] = [:]
            if !cursor.isNull { params["cursor"] = cursor }
            let result = try await connection.request("model/list", .object(params))
            for model in result["data"]?.array ?? [] where model["hidden"]?.bool != true {
                guard let id = model["id"]?.string else { continue }
                models.append(ModelOption(
                    id: id,
                    name: model["displayName"]?.string ?? id,
                    detail: model["description"]?.string,
                    efforts: (model["supportedReasoningEfforts"]?.array ?? []).compactMap { $0["reasoningEffort"]?.string },
                    defaultEffort: model["defaultReasoningEffort"]?.string,
                    isDefault: model["isDefault"]?.bool ?? false,
                    fastTier: fastTier(in: model["serviceTiers"]?.array ?? [])
                ))
            }
            cursor = result["nextCursor"] ?? .null
            if cursor.isNull { break }
        }
        return models
    }

    /// The account's rate-limit windows, such as the 5-hour and weekly limits, or a monthly one on some plans.
    static func readPlanLimits(executable: URL, environment: [String: String]) async throws -> PlanLimits? {
        let connection = try await connect(
            executable: executable,
            directory: FileManager.default.temporaryDirectory,
            environment: environment,
            configure: { _ in }
        )
        defer { connection.close() }
        let result = try await connection.request("account/rateLimits/read")
        var snapshots: [JSONValue] = []
        if let byID = result["rateLimitsByLimitId"]?.object, !byID.isEmpty {
            snapshots = byID.keys.sorted().compactMap { byID[$0] }
        } else if let snapshot = result["rateLimits"], !snapshot.isNull {
            snapshots = [snapshot]
        }
        var plan: String?
        var windows: [PlanLimits.Window] = []
        for snapshot in snapshots {
            plan = plan ?? snapshot["planType"]?.string
            let name = snapshots.count > 1 ? snapshot["limitName"]?.string : nil
            for key in ["primary", "secondary"] {
                guard let window = snapshot[key], let percent = window["usedPercent"]?.double else { continue }
                var title = PlanLimitsReader.windowTitle(minutes: window["windowDurationMins"]?.int)
                if let name { title += " · \(name)" }
                windows.append(PlanLimits.Window(
                    id: "\(snapshot["limitId"]?.string ?? "codex")-\(key)",
                    title: title,
                    percent: percent,
                    resetsAt: window["resetsAt"]?.double.map { Date(timeIntervalSince1970: $0) }
                ))
            }
        }
        guard !windows.isEmpty else { return nil }
        return PlanLimits(planName: PlanLimitsReader.planName(plan), windows: windows, resetCredits: parseResetCredits(result["rateLimitResetCredits"] ?? result["rate_limit_reset_credits"]))
    }

    /// The banked resets in a rate-limits answer: a count, plus the credits it described that are still spendable.
    private static func parseResetCredits(_ raw: JSONValue?) -> PlanLimits.ResetCredits? {
        guard let raw, let count = raw["availableCount"]?.int ?? raw["available_count"]?.int else { return nil }
        let credits = (raw["credits"]?.array ?? []).compactMap { credit -> PlanLimits.ResetCredit? in
            guard let id = credit["id"]?.string, !id.isEmpty else { return nil }
            let status = credit["status"]?.string
            guard status == nil || status == "available" || status == "unknown" else { return nil }
            return PlanLimits.ResetCredit(
                id: id,
                title: credit["title"]?.string,
                detail: credit["description"]?.string,
                expiresAt: resetDate(credit["expiresAt"] ?? credit["expires_at"])
            )
        }
        return PlanLimits.ResetCredits(availableCount: max(0, count), credits: credits)
    }

    /// Epoch seconds or milliseconds (anything above 1e10 is milliseconds), or an ISO 8601 string.
    private static func resetDate(_ value: JSONValue?) -> Date? {
        guard let value else { return nil }
        if let number = value.double ?? value.string.flatMap(Double.init) {
            guard number > 0 else { return nil }
            return Date(timeIntervalSince1970: number > 10_000_000_000 ? number / 1_000 : number)
        }
        guard let text = value.string else { return nil }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: text) ?? ISO8601DateFormatter().date(from: text)
    }

    /// Spends one of the account's banked resets. Without a credit id the app-server picks one.
    static func consumeResetCredit(executable: URL, environment: [String: String], creditID: String?) async throws -> PlanLimits.ResetOutcome {
        let connection = try await connect(
            executable: executable,
            directory: FileManager.default.temporaryDirectory,
            environment: environment,
            configure: { _ in }
        )
        defer { connection.close() }
        var params: [String: JSONValue] = ["idempotencyKey": .string(UUID().uuidString.lowercased())]
        if let creditID { params["creditId"] = .string(creditID) }
        let result = try await connection.request("account/rateLimitResetCredit/consume", .object(params))
        guard let outcome = result["outcome"]?.string.flatMap(PlanLimits.ResetOutcome.init(rawValue:)) else {
            throw ResetCreditError.unknownOutcome
        }
        return outcome
    }

    /// The tier Codex offers for faster responses on a model, if any.
    private static func fastTier(in tiers: [JSONValue]) -> String? {
        tiers.lazy.compactMap { tier -> String? in
            guard let id = tier["id"]?.string else { return nil }
            let label = "\(id) \(tier["name"]?.string ?? "")".lowercased()
            return label.contains("fast") || label.contains("priority") ? id : nil
        }.first
    }

    private static func connect(
        executable: URL,
        directory: URL,
        environment: [String: String],
        configure: (JSONRPCConnection) -> Void
    ) async throws -> JSONRPCConnection {
        let process = StdioProcess(executable: executable, arguments: ["app-server"], directory: directory, environment: environment)
        let connection = JSONRPCConnection(process: process, sendsVersion: false)
        configure(connection)
        try connection.start()
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(20))
            if !Task.isCancelled { connection.close() }
        }
        defer { watchdog.cancel() }
        do {
            _ = try await connection.request("initialize", [
                "clientInfo": ["name": "droppy-code", "title": "Droppy Code", "version": .string(AppInfo.version)],
                "capabilities": ["experimentalApi": true],
            ])
        } catch {
            connection.close()
            throw error
        }
        connection.notify("initialized")
        return connection
    }

    // MARK: - Notifications

    private func handleNotification(_ method: String, _ params: JSONValue) {
        if method == "thread/started" {
            headStarted(params["thread"] ?? .null)
            return
        }
        // Whose notification: the lead's, a head's, or some other thread's on the same server.
        var target = Target.lead
        if let eventThread = params["threadId"]?.string, let threadID, eventThread != threadID {
            guard headThreads.contains(eventThread) else { return }
            target = .head(eventThread)
        }
        let sink: (ProviderEvent) -> Void = switch target {
        case .lead: leadSink
        case .head(let id): { [weak self] event in self?.onEvent?(.agentEvent(agentID: id, event)) }
        }
        switch method {
        case "turn/started":
            let id = params["turn"]?["id"]?.string
            switch target {
            case .lead:
                turnID = id
            case .head(let head):
                headTurns[head] = id
            }
            sink(.turnStarted(providerTurnID: id))
        case "item/started":
            if let item = params["item"] { itemStarted(item, target: target, sink: sink) }
        case "item/completed":
            if let item = params["item"] { itemCompleted(item, target: target, sink: sink) }
        case "item/agentMessage/delta":
            if let id = params["itemId"]?.string, let delta = params["delta"]?.string {
                sink(.messageDelta(id: id, text: delta))
            }
        case "item/reasoning/summaryTextDelta", "item/reasoning/textDelta":
            if let id = params["itemId"]?.string, let delta = params["delta"]?.string {
                sink(.reasoningDelta(id: id, text: delta))
            }
        case "item/reasoning/summaryPartAdded":
            if let id = params["itemId"]?.string, let index = params["summaryIndex"]?.int, index > 0 {
                sink(.reasoningDelta(id: id, text: "\n\n"))
            }
        case "item/commandExecution/outputDelta":
            if let id = params["itemId"]?.string, let delta = params["delta"]?.string {
                sink(.toolOutput(id: id, text: delta))
            }
        case "item/plan/delta":
            if let id = params["itemId"]?.string, let delta = params["delta"]?.string {
                sink(.planDelta(id: id, text: delta))
            }
        case "turn/plan/updated":
            let steps = (params["plan"]?.array ?? []).compactMap { step -> TodoStep? in
                guard let text = step["step"]?.string else { return nil }
                let status: TodoStep.Status = switch step["status"]?.string {
                case "completed": .done
                case "inProgress": .active
                default: .pending
                }
                return TodoStep(text: text, status: status)
            }
            sink(.todos(steps))
        case "turn/diff/updated":
            if let diff = params["diff"]?.string { sink(.diff(diff)) }
        case "thread/tokenUsage/updated":
            guard let usage = params["tokenUsage"], let last = usage["last"] else { break }
            let used = last["totalTokens"]?.int ?? 0
            switch target {
            case .lead:
                onEvent?(.usage(ContextUsage(usedTokens: used, windowTokens: usage["modelContextWindow"]?.int)))
                // The thread total only ever grows within a session, so its
                // positive deltas are exact spend no matter how often this
                // event fires. The per-update `last` value is the fallback
                // for servers that omit the total.
                if let total = usage["total"]?["totalTokens"]?.int, total > 0 {
                    let spend = spendTracker.spend(total: total)
                    if spend > 0 { TokenLedger.shared.record(spend: spend) }
                } else {
                    TokenLedger.shared.record(spend: used)
                }
            case .head(let head):
                // A head's spend is the account's spend too; its total lives on its own thread.
                let total = usage["total"]?["totalTokens"]?.int ?? 0
                if total > 0 {
                    var tracker = headSpend[head] ?? TokenSpendTracker()
                    let spend = tracker.spend(total: total)
                    headSpend[head] = tracker
                    if spend > 0 { TokenLedger.shared.record(spend: spend) }
                } else {
                    TokenLedger.shared.record(spend: used)
                }
                onEvent?(.agentProgress(agentID: head, summary: nil, lastTool: nil, tokens: total > 0 ? total : used, toolCalls: nil))
            }
        case "error":
            if case .lead = target, params["willRetry"]?.bool == true {
                let message = Self.errorMessage(params["error"])
                onEvent?(.notice(Notice(level: .warning, message: "\(message) Retrying.")))
            }
        case "turn/completed":
            let turn = params["turn"]
            let status: TurnStatus = switch turn?["status"]?.string {
            case "interrupted": .interrupted
            case "failed": .failed
            default: .completed
            }
            let error = turn?["error"].flatMap { $0.isNull ? nil : Self.errorMessage($0) }
            switch target {
            case .lead:
                turnID = nil
                if status == .failed, let error, UsageLimitSignal.matches(error) {
                    onEvent?(.usageLimit(resetsAt: UsageLimitSignal.resetTime(in: error)))
                }
                onEvent?(.turnCompleted(status: status, error: error))
            case .head(let head):
                headTurns[head] = nil
                onEvent?(.agentFinished(agentID: head, status: status, summary: error))
            }
        case "serverRequest/resolved":
            if let raw = params["requestId"], let id = RPCID(raw), pendingRequests.removeValue(forKey: id.key) != nil {
                onEvent?(.requestResolved(id: id.key))
            }
        case "thread/compacted":
            sink(.notice(Notice(level: .info, message: "Context compacted.")))
        default:
            break
        }
    }

    /// A thread the server started: a head, when the lead's thread is its parent.
    private func headStarted(_ thread: JSONValue) {
        guard let id = thread["id"]?.string, let threadID, !headThreads.contains(id) else { return }
        guard thread["parentThreadId"]?.string == threadID else {
            // Another thread on the same server, unless it is a spawned agent whose parent
            // the server names otherwise (a resumed lead whose id rotated): a thread with
            // an agent nickname or role is a head of this session's team and is taken in,
            // logged, so a team that never shows in the panel can be traced.
            let nickname = thread["agentNickname"]?.string, role = thread["agentRole"]?.string
            guard nickname != nil || role != nil else { return }
            Self.log.notice("thread/started \(id, privacy: .public) names parent \(thread["parentThreadId"]?.string ?? "none", privacy: .public), not this lead \(threadID, privacy: .public); taken as a head by its agent nickname")
            headThreads.insert(id)
            let preview = thread["preview"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let description = !preview.isEmpty ? TextCleanup.singleLine(preview, limit: 80) : ([nickname, role].compactMap { $0 }.joined(separator: " · ").nilIfEmpty ?? "Subagent")
            onEvent?(.agentStarted(AgentSpawn(id: id, taskID: nil, toolUseID: nil, description: description, prompt: preview.nilIfEmpty, model: thread["model"]?.string)))
            return
        }
        headThreads.insert(id)
        let nickname = thread["agentNickname"]?.string
        let role = thread["agentRole"]?.string
        let preview = thread["preview"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let description = !preview.isEmpty ? TextCleanup.singleLine(preview, limit: 80) : ([nickname, role].compactMap { $0 }.joined(separator: " · ").nilIfEmpty ?? "Subagent")
        onEvent?(.agentStarted(AgentSpawn(id: id, taskID: nil, toolUseID: nil, description: description, prompt: preview.nilIfEmpty, model: thread["model"]?.string)))
    }

    private func itemStarted(_ item: JSONValue, target: Target, sink: (ProviderEvent) -> Void) {
        guard let id = item["id"]?.string else { return }
        switch item["type"]?.string {
        case "agentMessage":
            sink(.messageDelta(id: id, text: item["text"]?.string ?? ""))
        case "reasoning":
            sink(.reasoningDelta(id: id, text: ""))
        case "plan":
            sink(.planDelta(id: id, text: item["text"]?.string ?? ""))
        case "userMessage":
            // A head's brief arrives as its first user message.
            if case .head(let head) = target, let text = Self.userText(item) {
                onEvent?(.agentStarted(AgentSpawn(id: head, taskID: nil, toolUseID: nil, description: TextCleanup.singleLine(text, limit: 80), prompt: text, model: nil)))
            }
        case "hookPrompt", "functionCallOutput", "contextCompaction":
            break
        default:
            if let call = toolCall(from: item) { sink(.toolStarted(id: id, call: call)) }
            // A spawn that already names its heads registers them as it starts, so a
            // completion lost to compaction never leaves them unknown.
            if item["type"]?.string == "collabAgentToolCall" { describeHeads(from: item) }
        }
    }

    private func itemCompleted(_ item: JSONValue, target: Target, sink: (ProviderEvent) -> Void) {
        guard let id = item["id"]?.string else { return }
        switch item["type"]?.string {
        case "agentMessage":
            sink(.messageCompleted(id: id, text: item["text"]?.string ?? ""))
        case "reasoning":
            let summary = (item["summary"]?.array ?? []).compactMap(\.string).joined(separator: "\n\n")
            let content = (item["content"]?.array ?? []).compactMap(\.string).joined(separator: "\n\n")
            sink(.reasoningCompleted(id: id, text: summary.isEmpty ? content : summary))
        case "plan":
            sink(.planCompleted(id: id, markdown: item["text"]?.string ?? ""))
        case "userMessage", "hookPrompt", "functionCallOutput", "contextCompaction":
            break
        default:
            if let call = toolCall(from: item) {
                sink(.toolStarted(id: id, call: call))
                sink(.toolUpdated(id: id, update: toolUpdate(from: item)))
            }
            // On either target: the payload says whose head it is, whatever thread the
            // server filed the notice under.
            describeHeads(from: item)
        }
    }

    /// A finished collab call names the heads it reached: a spawn carries the brief the
    /// head was given, which is a better task line than its nickname. The lead's
    /// sub-agent activity items bracket a head's life too, so a head whose own thread
    /// stays quiet still ends.
    private func describeHeads(from item: JSONValue) {
        switch item["type"]?.string {
        case "collabAgentToolCall":
            // A spawn names the heads it reached; the brief is the best task line when it
            // carries one, and a spawn without one still registers its heads.
            guard item["tool"]?.string == "spawnAgent" else { return }
            let prompt = item["prompt"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            for receiver in (item["receiverThreadIds"]?.array ?? []).compactMap(\.string) {
                if !headThreads.contains(receiver) { headThreads.insert(receiver) }
                onEvent?(.agentStarted(AgentSpawn(id: receiver, taskID: nil, toolUseID: item["id"]?.string, description: prompt.isEmpty ? "Head" : TextCleanup.singleLine(prompt, limit: 80), prompt: prompt.nilIfEmpty, model: item["model"]?.string)))
            }
        case "subAgentActivity":
            guard let agentThread = item["agentThreadId"]?.string else { return }
            if !headThreads.contains(agentThread) {
                // A head the app-server announced only through the lead's activity note:
                // no `thread/started` naming the lead as parent and no receiver id on the
                // spawn (older app-servers). It is registered from here, so it still gets
                // its face in the panel and its report in the chat rather than passing as
                // a bare "Head started" line; its own thread's events are heard from now on.
                headThreads.insert(agentThread)
                let nickname = item["agentPath"]?.string?.split(separator: "/").last.map(String.init) ?? "Head"
                Self.log.notice("head \(agentThread, privacy: .public) registered from a subAgentActivity note (\(nickname, privacy: .public)); no thread/started or spawn receiver id preceded it")
                onEvent?(.agentStarted(AgentSpawn(id: agentThread, taskID: nil, toolUseID: nil, description: nickname.replacingOccurrences(of: "_", with: " "), prompt: nil, model: nil)))
            }
            switch item["kind"]?.string {
            case "completed":
                headTurns[agentThread] = nil
                onEvent?(.agentFinished(agentID: agentThread, status: .completed, summary: nil))
            case "interrupted":
                headTurns[agentThread] = nil
                onEvent?(.agentFinished(agentID: agentThread, status: .interrupted, summary: nil))
            default:
                break
            }
        default:
            break
        }
    }

    /// The text of a user message item.
    private static func userText(_ item: JSONValue) -> String? {
        let parts = (item["content"]?.array ?? []).compactMap { part -> String? in
            part["text"]?.string ?? (part["type"]?.string == "text" ? part["value"]?.string : nil)
        }
        let text = parts.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    private func toolCall(from item: JSONValue) -> ToolCall? {
        switch item["type"]?.string {
        case "commandExecution":
            let command = ToolTitles.unwrapShell(item["command"]?.string ?? "")
            let actions = item["commandActions"]?.array ?? []
            let kinds = Set(actions.compactMap { $0["type"]?.string })
            if kinds == ["read"], let path = actions.first?["path"]?.string {
                return ToolCall(kind: .read, title: ToolTitles.relativePath(path, to: workingDirectory), detail: command)
            }
            if !kinds.isEmpty, kinds.isSubset(of: ["search", "listFiles"]) {
                return ToolCall(kind: .search, title: command)
            }
            return ToolCall(kind: .command, title: command)
        case "fileChange":
            let paths = (item["changes"]?.array ?? []).compactMap { $0["path"]?.string }
            let title = paths.count == 1 ? ToolTitles.relativePath(paths[0], to: workingDirectory) : "Edit \(paths.count) files"
            return ToolCall(kind: .edit, title: title)
        case "mcpToolCall":
            let server = item["server"]?.string ?? "MCP"
            let tool = item["tool"]?.string ?? "tool"
            let arguments = item["arguments"].map(\.compactString)
            return ToolCall(kind: .mcp, title: "\(server) · \(tool)", detail: arguments == "null" ? nil : arguments)
        case "dynamicToolCall":
            return ToolCall(kind: .other, title: item["tool"]?.string ?? "Tool")
        case "webSearch":
            return ToolCall(kind: .web, title: item["query"]?.string ?? "Web search")
        case "collabAgentToolCall":
            let verb = switch item["tool"]?.string {
            case "spawnAgent": "Send out a head"
            case "sendInput", "sendMessage", "followupTask": "Brief a head"
            case "wait": "Wait for heads"
            case "closeAgent", "interruptAgent": "Recall a head"
            case "resumeAgent": "Resume a head"
            case "listAgents": "List heads"
            default: "Agent"
            }
            let brief = item["prompt"]?.string.map { TextCleanup.singleLine($0, limit: 90) }
            return ToolCall(kind: .agent, title: brief ?? verb, detail: brief == nil ? nil : verb)
        case "subAgentActivity":
            // The lead's notes on a head starting, finishing or being stopped drive the
            // head's own status (see `describeHeads`); as rows they only doubled the
            // head's report pill as "Delegated Head finished", so they draw nothing.
            return nil
        case "imageView":
            return ToolCall(kind: .read, title: ToolTitles.relativePath(item["path"]?.string ?? "Image", to: workingDirectory))
        case "imageGeneration":
            return ToolCall(kind: .other, title: item["revisedPrompt"]?.string ?? "Generate image")
        case "enteredReviewMode", "exitedReviewMode":
            return ToolCall(kind: .other, title: "Review", detail: item["review"]?.string)
        default:
            return nil
        }
    }

    private func toolUpdate(from item: JSONValue) -> ToolUpdate {
        var update = ToolUpdate()
        let rawStatus = item["status"]?.string
        switch item["type"]?.string {
        case "commandExecution":
            update.output = item["aggregatedOutput"]?.string
            update.exitCode = item["exitCode"]?.int
            update.status = Self.toolStatus(rawStatus, exitCode: update.exitCode)
        case "fileChange":
            let changes: [JSONValue] = item["changes"]?.array ?? []
            let edits = changes.map { (change: JSONValue) -> FileEdit in
                let diff: String? = change["diff"]?.string
                let stats = ToolTitles.diffStats(diff ?? "")
                let path = ToolTitles.relativePath(change["path"]?.string ?? "", to: workingDirectory)
                return FileEdit(path: path, diff: diff, additions: stats.additions, deletions: stats.deletions)
            }
            update.edits = edits
            update.status = Self.toolStatus(rawStatus, exitCode: nil)
        case "mcpToolCall":
            if let message = item["error"]?["message"]?.string {
                update.output = message
            } else if let content = item["result"]?["content"]?.array {
                update.output = content.compactMap { $0["text"]?.string }.joined(separator: "\n")
            }
            update.status = Self.toolStatus(rawStatus, exitCode: nil)
        case "webSearch", "imageView", "enteredReviewMode", "exitedReviewMode":
            update.status = .completed
        default:
            update.status = Self.toolStatus(rawStatus, exitCode: nil)
        }
        return update
    }

    private static func toolStatus(_ raw: String?, exitCode: Int?) -> ToolCall.Status {
        switch raw {
        case "inProgress": return .running
        case "failed", "interrupted": return .failed
        case "declined": return .declined
        default:
            if let exitCode, exitCode != 0 { return .failed }
            return .completed
        }
    }

    static func errorMessage(_ error: JSONValue?) -> String {
        guard let error else { return "Codex reported an error." }
        let raw = error["message"]?.string ?? error.displayText
        if let nested = JSONValue.parse(raw),
           let message = nested["error"]?["message"]?.string ?? nested["message"]?.string {
            return message
        }
        return raw
    }

    // MARK: - Requests

    private func handleRequest(_ id: RPCID, _ method: String, _ params: JSONValue) {
        switch method {
        case "item/commandExecution/requestApproval":
            let (options, payloads) = decisionOptions(params["availableDecisions"]?.array, command: true)
            pendingRequests[id.key] = .decision(id, payloads: payloads)
            let command = params["command"]?.string.map(ToolTitles.unwrapShell) ?? "Command"
            onEvent?(.approval(ApprovalRequest(
                id: id.key,
                kind: .command,
                title: command,
                detail: params["cwd"]?.string.map { ToolTitles.relativePath($0, to: workingDirectory) },
                reason: params["reason"]?.string,
                options: options,
                toolItemID: params["itemId"]?.string
            )))
        case "item/fileChange/requestApproval":
            let (options, payloads) = decisionOptions(nil, command: false)
            pendingRequests[id.key] = .decision(id, payloads: payloads)
            onEvent?(.approval(ApprovalRequest(
                id: id.key,
                kind: .fileChange,
                title: "Apply file changes",
                detail: params["grantRoot"]?.string,
                reason: params["reason"]?.string,
                options: options,
                toolItemID: params["itemId"]?.string
            )))
        case "item/permissions/requestApproval":
            let requested = params["permissions"] ?? [:]
            pendingRequests[id.key] = .permissions(id, requested: requested)
            onEvent?(.approval(ApprovalRequest(
                id: id.key,
                kind: .permissions,
                title: Self.describePermissions(requested),
                detail: nil,
                reason: params["reason"]?.string,
                options: [
                    .init(id: "turn", title: "Allow once", role: .approve),
                    .init(id: "session", title: "Allow for session", role: .approveAlways),
                    .init(id: "decline", title: "Decline", role: .decline),
                ],
                toolItemID: params["itemId"]?.string
            )))
        case "item/tool/requestUserInput":
            let questions = (params["questions"]?.array ?? []).compactMap { question -> QuestionRequest.Question? in
                guard let questionID = question["id"]?.string else { return nil }
                return QuestionRequest.Question(
                    id: questionID,
                    header: question["header"]?.string ?? "",
                    prompt: question["question"]?.string ?? "",
                    choices: (question["options"]?.array ?? []).compactMap { option in
                        option["label"]?.string.map { QuestionRequest.Choice(label: $0, detail: option["description"]?.string) }
                    },
                    allowsMultiple: false,
                    allowsOther: question["isOther"]?.bool ?? false,
                    isSecret: question["isSecret"]?.bool ?? false
                )
            }
            pendingRequests[id.key] = .question(id)
            onEvent?(.question(QuestionRequest(id: id.key, questions: questions)))
        case "mcpServer/elicitation/request":
            connection?.respond(to: id, result: ["action": "decline"])
            let server = params["serverName"]?.string ?? "An MCP server"
            onEvent?(.notice(Notice(level: .warning, message: "\(server) asked for input, which Droppy Code cannot show yet.")))
        default:
            connection?.respond(to: id, errorCode: -32601, message: "Droppy Code does not support \(method).")
        }
    }

    private func decisionOptions(_ available: [JSONValue]?, command: Bool) -> ([ApprovalRequest.Option], [String: JSONValue]) {
        let decisions = available ?? ["accept", "acceptForSession", "decline", "cancel"]
        var options: [ApprovalRequest.Option] = []
        var payloads: [String: JSONValue] = [:]
        for decision in decisions {
            if let name = decision.string {
                let option: ApprovalRequest.Option? = switch name {
                case "accept": .init(id: name, title: "Approve", role: .approve)
                case "acceptForSession": .init(id: name, title: "Approve for session", role: .approveAlways)
                case "decline": .init(id: name, title: "Decline", role: .decline)
                case "cancel": .init(id: name, title: "Stop turn", role: .cancel)
                default: nil
                }
                if let option {
                    options.append(option)
                    payloads[name] = decision
                }
            } else if command, decision["acceptWithExecpolicyAmendment"] != nil {
                options.append(.init(id: "amend", title: "Always allow", role: .approveAlways))
                payloads["amend"] = decision
            }
        }
        let order: [ApprovalRequest.Option.Role] = [.approve, .approveAlways, .decline, .cancel]
        options.sort { (order.firstIndex(of: $0.role) ?? 0) < (order.firstIndex(of: $1.role) ?? 0) }
        return (options, payloads)
    }

    private static func describePermissions(_ permissions: JSONValue) -> String {
        var parts: [String] = []
        if let write = permissions["fileSystem"]?["write"]?.array, !write.isEmpty {
            parts.append("Write to " + write.compactMap(\.string).joined(separator: ", "))
        }
        if let read = permissions["fileSystem"]?["read"]?.array, !read.isEmpty {
            parts.append("Read " + read.compactMap(\.string).joined(separator: ", "))
        }
        if permissions["network"]?["enabled"]?.bool == true {
            parts.append("Network access")
        }
        return parts.isEmpty ? "Additional permissions" : parts.joined(separator: " · ")
    }

    private func handleClose(_ tail: String) {
        let resolved = Array(pendingRequests.keys)
        pendingRequests.removeAll()
        for key in resolved { onEvent?(.requestResolved(id: key)) }
        guard !isStopping else { return }
        onEvent?(.exited(error: TextCleanup.lastLines(tail)))
    }

    private func policySettings(_ mode: RuntimeMode) -> PolicySettings {
        switch mode {
        case .supervised:
            PolicySettings(approvalPolicy: "untrusted", sandbox: "read-only", sandboxPolicy: ["type": "readOnly"], reviewer: "user")
        case .autoAcceptEdits:
            PolicySettings(approvalPolicy: "on-request", sandbox: "workspace-write", sandboxPolicy: ["type": "workspaceWrite"], reviewer: "user")
        case .auto:
            PolicySettings(approvalPolicy: "on-request", sandbox: "workspace-write", sandboxPolicy: ["type": "workspaceWrite"], reviewer: "auto_review")
        case .fullAccess:
            PolicySettings(approvalPolicy: "never", sandbox: "danger-full-access", sandboxPolicy: ["type": "dangerFullAccess"], reviewer: "user")
        }
    }
}
