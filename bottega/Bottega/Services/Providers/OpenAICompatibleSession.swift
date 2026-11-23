// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// One agent implementation for every OpenAI-compatible native provider
/// (DeepSeek, Meta, Z.ai). The three sessions used to be byte-for-byte forks of
/// this file with only the provider name, endpoints, effort mapping and error
/// strings different — a bug fixed in one had to be found and re-applied in the
/// other two (the `search_text` containment fix was). The tool loop, SSE
/// draining, history trimming, approvals and plan-mode blocking live here;
/// subclasses carry a `Config` plus the few real per-provider hooks.
@MainActor
class OpenAICompatibleSession: ProviderSession {
    /// The constants that genuinely differ per provider.
    struct Config {
        /// The `ProviderKind` refused with when no key is set.
        let kind: ProviderKind
        /// Short user-facing name for notices and errors ("DeepSeek", "Meta", "Z.ai").
        let title: String
        /// The identity phrase inside the system prompt, per model.
        /// Meta's names the model: "Meta coding agent (Muse Spark muse-spark-1.3)".
        let agentIdentity: (String) -> String
        let defaultModel: String
        let chatURL: URL
        let contextWindow: Int
        /// The response cap Z.ai asks for explicitly; nil where the provider needs none.
        let maxOutputTokens: Int?
        /// DeepSeek's thinking mode requires `reasoning_content` replayed back into
        /// the history with every assistant message (an empty string is accepted);
        /// Meta's Chat Completions must not carry it and Z.ai's accepts it.
        let replaysReasoning: Bool
        /// Whether a model accepts images, asked before an image is sent.
        let isVisionModel: (String) -> Bool
    }

    let config: Config
    var onEvent: ((ProviderEvent) -> Void)?

    let configuration: SessionConfiguration
    private let fileTools: NativeFileTools
    /// Root AGENTS.md/CLAUDE.md, injected into the system prompt and refreshed when
    /// they change so a resumed turn carries rules edited since the last one.
    private var guidance: ProjectGuidance
    private var sessionID: String?
    private var messages: [JSONValue] = []
    private var runtimeMode: RuntimeMode
    private var interactionMode: InteractionMode
    private var currentModel: String
    private var counter = 0
    private var interrupted = false
    private var isStopping = false
    private var approveAllRemaining = false
    /// The whole turn, so a stop reaches a tool still running and an approval still
    /// waited on, not just the request in flight.
    private var sendTask: Task<Void, Error>?
    private var roundTask: Task<StreamRound, Error>?
    /// The detached SSE drain for the in-flight round, so a stop cancels it
    /// directly instead of waiting for the next chunk to reach the consumer.
    private var sseDrain: Task<Void, Never>?
    private var pendingApprovals: [String: CheckedContinuation<Bool, Never>] = [:]
    private var mcpTools: [MCPHub.Tool] = []
    private var didLoadMCPTools = false
    private var mcpToolDefinitions: [JSONValue] {
        mcpTools.map { tool in
            ["type": "function", "function": ["name": .string(tool.callName), "description": .string(tool.description), "parameters": tool.inputSchema]]
        }
    }

    /// Runaway guard only. A turn used to stop after 12 rounds and report
    /// itself as completed, so any real task ended mid-edit with no reply and
    /// needed a "continue" every dozen tool calls. Real work runs until the
    /// model answers without tools or the user hits stop; hitting this many
    /// rounds means the model is looping, and the turn says so.
    private static let maxRoundsPerTurn = 200

    /// Tool results older than the last few are condensed once the ones in history add
    /// up to this much: the model has long since read them, and every round re-sends the
    /// whole history. A condensed result says how to get the full one back.
    private static let condenseHistoryAt = 160_000
    private static let condenseHistoryTo = 100_000
    private static let condenseKeepsRecent = 10
    private static let condensedMarker = "…[Bottega condensed this output"

    /// A lost connection is retried twice before the turn reports it: once after a
    /// second, once after two more. The request is the same payload over the same
    /// history, so nothing the round already did is done again — and nothing that is
    /// not a dropped connection (a rejected key, a validation error, a stop) is retried.
    private static let maxTransportRetries = 2
    private static let transportRetryDelays: [Duration] = [.seconds(1), .seconds(2)]

    /// The transport failures worth another attempt, by code rather than by message:
    /// the connection went away, the request timed out, or the host was unreachable.
    private static func isTransientTransportFailure(_ error: Error) -> Bool {
        guard let urlError = error as? URLError else { return false }
        switch urlError.code {
        case .networkConnectionLost, .timedOut, .cannotConnectToHost: return true
        default: return false
        }
    }

    private struct StreamRound: Sendable {
        var content = ""
        var reasoning = ""
        var toolCalls: [PendingToolCall] = []
        var usage: ContextUsage?
        /// The usage components exactly as the response reported them, for the
        /// spend event. Nil when the response carried no usage object.
        var tokenUsage: ReportedTokenUsage?
        var rawError: String?
        var statusCode: Int = 200
    }

    /// One response's reported token counts, kept apart from `ContextUsage` (which
    /// only needs a total against the window). `input` already includes `cached`,
    /// as the API reports it, so nothing here is summed from the others.
    private struct ReportedTokenUsage: Sendable {
        var total: Int
        var input: Int?
        var output: Int?
        var cachedInput: Int?

        /// Whether any component was actually reported: an all-zero usage object
        /// describes no spend and gets no event.
        var hasSpend: Bool { total > 0 || (input ?? 0) > 0 || (output ?? 0) > 0 }
    }

    private struct PendingToolCall: Sendable {
        var id: String
        var name: String
        var arguments: String
    }

    /// One rejected attempt at a round, with whether the user already saw part of an
    /// answer. The error stays the object the transport threw — a `URLError` is still a
    /// `URLError`, not its wording — so the retry decision is made on the code.
    private struct StreamAttemptFailure: Error {
        var underlying: Error
        var emittedOutput: Bool
    }

    /// One parsed SSE value, ready for the main actor to emit.
    private enum StreamChunk: Sendable {
        case text(String)
        case reasoning(String)
        case toolCall(index: Int, id: String, name: String, arguments: String)
        case usage(ContextUsage)
        case tokenUsage(ReportedTokenUsage)
        case streamError(String)
        case transportError(Error)
    }

    /// Holds consecutive text off the main actor and yields it as one larger
    /// delta, at most 30 ms after the first piece, so the main actor hops less.
    private actor StreamCoalescer {
        private let continuation: AsyncStream<StreamChunk>.Continuation
        private var pendingText = ""
        private var pendingReasoning = ""
        private var finished = false

        init(continuation: AsyncStream<StreamChunk>.Continuation) {
            self.continuation = continuation
        }

        func appendText(_ value: String) {
            if !pendingReasoning.isEmpty { flush() }
            let idle = pendingText.isEmpty
            pendingText += value
            if idle { scheduleFlush() }
        }

        func appendReasoning(_ value: String) {
            if !pendingText.isEmpty { flush() }
            let idle = pendingReasoning.isEmpty
            pendingReasoning += value
            if idle { scheduleFlush() }
        }

        func emitToolCall(index: Int, id: String, name: String, arguments: String) {
            flush()
            continuation.yield(.toolCall(index: index, id: id, name: name, arguments: arguments))
        }

        func emitUsage(_ value: ContextUsage) {
            flush()
            continuation.yield(.usage(value))
        }

        func emitTokenUsage(_ value: ReportedTokenUsage) {
            flush()
            continuation.yield(.tokenUsage(value))
        }

        func emitStreamError(_ message: String) {
            flush()
            continuation.yield(.streamError(message))
        }

        func finish() {
            guard !finished else { return }
            finished = true
            flush()
            continuation.finish()
        }

        func fail(chunkError error: Error) {
            guard !finished else { return }
            finished = true
            flush()
            continuation.yield(.transportError(error))
            continuation.finish()
        }

        private func flush() {
            if !pendingText.isEmpty {
                continuation.yield(.text(pendingText))
                pendingText = ""
            }
            if !pendingReasoning.isEmpty {
                continuation.yield(.reasoning(pendingReasoning))
                pendingReasoning = ""
            }
        }

        private func scheduleFlush() {
            Task.detached {
                try? await Task.sleep(for: .milliseconds(30))
                await self.flush()
            }
        }
    }

    init(config: Config, configuration: SessionConfiguration) {
        self.config = config
        self.configuration = configuration
        fileTools = NativeFileTools(workingDirectory: configuration.workingDirectory.path)
        guidance = ProjectGuidance(workingDirectory: configuration.workingDirectory.path)
        runtimeMode = configuration.runtimeMode
        interactionMode = configuration.interactionMode
        currentModel = configuration.model ?? config.defaultModel
    }

    var isRunning: Bool { sessionID != nil && !isStopping }

    private var workingDirectory: String { configuration.workingDirectory.path }
    private var apiKey: String { (configuration.apiKey ?? "").trimmingCharacters(in: .whitespacesAndNewlines) }

    // MARK: - Lifecycle

    func start() async throws -> String {
        guard !apiKey.isEmpty else { throw ProviderError.notInstalled(config.kind) }
        let id = configuration.resumeID ?? UUID().uuidString.lowercased()
        sessionID = id
        interrupted = false
        isStopping = false
        if messages.isEmpty {
            messages = [["role": "system", "content": .string(systemPrompt())]]
            // A relaunch starts with nothing said: the thread's past exchanges go back in,
            // so the model still knows its brief and what it did about it.
            for message in configuration.transcript {
                messages.append(["role": message.role == .user ? "user" : "assistant", "content": .string(message.text)])
            }
            trimHistory()
        }
        // No `.models` event: the registry owns the live catalog, and a seed
        // sent here overwrote it every time a chat started.
        return id
    }

    func send(_ input: TurnInput) async throws {
        guard sendTask == nil, sessionID != nil, !isStopping else { throw ProviderError.notRunning }
        let task = Task {
            defer { self.sendTask = nil }
            try await self.performTurn(input)
        }
        sendTask = task
        try await withTaskCancellationHandler {
            try await task.value
        } onCancel: {
            task.cancel()
        }
    }

    private func performTurn(_ input: TurnInput) async throws {
        guard !Task.isCancelled else {
            await finishInterrupted()
            return
        }
        guard sessionID != nil, !isStopping else { throw ProviderError.notRunning }
        guard !apiKey.isEmpty else { throw ProviderError.notInstalled(config.kind) }
        runtimeMode = input.runtimeMode
        interactionMode = input.interactionMode
        if let model = input.model, !model.isEmpty { currentModel = model }
        interrupted = false
        approveAllRemaining = false
        // A turn that resumes after AGENTS.md or CLAUDE.md changed gets the new rules;
        // the files are only re-read when their size or mtime moved.
        refreshSystemPrompt()

        let userContent = await userContentValue(text: input.text, images: input.images)
        messages.append(["role": "user", "content": userContent])
        trimHistory()
        onEvent?(.turnStarted(providerTurnID: nil))

        var finalText = ""
        var rounds = 0
        var toolsSinceChange = 0
        // A hand's budget for the turn (Settings › Crew › Limits); a hand set never to be
        // paced is never nudged.
        let limits = configuration.crewLimits
        var pacingAt = limits.pacingTools ?? Int.max
        var toolsUsed = 0
        var wrapUpAsked = false
        let turnStartedAt = Date.now
        var hitCeiling = false
        do {
            while !interrupted {
                try Task.checkCancellation()
                guard rounds < Self.maxRoundsPerTurn else { hitCeiling = true; break }
                rounds += 1
                // A hand past its budget writes its report with no tools left to reach
                // for; a hand stopped by its runtime for the same reason answers the same way.
                let budgetSpent = configuration.isCrewHand && limits.handBudgetSpent(tools: toolsUsed, elapsed: Date.now.timeIntervalSince(turnStartedAt))
                if input.isFinalReport || budgetSpent {
                    if budgetSpent { messages.append(["role": "user", "content": .string(CrewBudget.finalNote)]) }
                    let last = try await streamOneRound(model: currentModel, effort: input.effort, withTools: false)
                    reportUsage(last)
                    if let error = last.rawError, !error.isEmpty { throw ProviderError.failed(friendlyError(error, statusCode: last.statusCode)) }
                    if !last.content.isEmpty {
                        finalText = last.content
                        messages.append(assistantTurn(content: last.content, reasoning: last.reasoning, toolCalls: []))
                    }
                    break
                }
                if configuration.isCrewHand, !wrapUpAsked, limits.handShouldWrapUp(tools: toolsUsed, elapsed: Date.now.timeIntervalSince(turnStartedAt)) {
                    wrapUpAsked = true
                    messages.append(["role": "user", "content": .string(CrewBudget.wrapUpNote)])
                }
                // A hand that only ever reads is paced: every so many tools without a
                // change, a note in its history asks it to act or report. Its lead is
                // waiting, and a research task has an answer by then.
                if configuration.isCrewHand, let pacing = limits.pacingTools, toolsSinceChange >= pacingAt {
                    messages.append(["role": "user", "content": .string(CrewBudget.pacingNote(pacingTools: pacing))])
                    pacingAt = toolsSinceChange + pacing
                }
                condenseHistory()
                let round = try await streamOneRound(model: currentModel, effort: input.effort)
                reportUsage(round)
                if round.statusCode == 401 || (round.rawError?.localizedCaseInsensitiveContains("invalid") == true && round.rawError?.localizedCaseInsensitiveContains("key") == true) {
                    throw ProviderError.failed("\(config.title) rejected the API key. Check it in Settings → Providers → \(config.title).")
                }
                if let error = round.rawError, !error.isEmpty {
                    throw ProviderError.failed(friendlyError(error, statusCode: round.statusCode))
                }
                if !round.content.isEmpty { finalText = round.content }
                if round.toolCalls.isEmpty {
                    // Keep the reply in history, or the next turn forgets it.
                    if !round.content.isEmpty {
                        messages.append(assistantTurn(content: round.content, reasoning: round.reasoning, toolCalls: []))
                    }
                    break
                }
                // Record the assistant turn with its tool calls so the next
                // request keeps the OpenAI tool-call chain intact.
                messages.append(assistantTurn(content: round.content, reasoning: round.reasoning, toolCalls: round.toolCalls))
                var shouldContinue = true
                var toolIndex = 0
                while toolIndex < round.toolCalls.count {
                    if interrupted || Task.isCancelled { shouldContinue = false; break }
                    let tool = round.toolCalls[toolIndex]
                    // Consecutive reads that need no approval run together, four at a
                    // time. Everything else — a write, a shell command, an MCP call, an
                    // unknown tool, or a read the user has to approve — is a barrier: it
                    // waits for the group before it, and the model's order is kept.
                    if let kind = Self.readOnlyKind(for: tool.name), !requiresApproval(kind: kind) {
                        var end = toolIndex
                        while end < round.toolCalls.count,
                              let nextKind = Self.readOnlyKind(for: round.toolCalls[end].name),
                              !requiresApproval(kind: nextKind) {
                            end += 1
                        }
                        let batch = Array(round.toolCalls[toolIndex..<end])
                        toolsUsed += batch.count
                        toolsSinceChange += batch.count
                        let outputs = await executeReadBatch(batch)
                        if Task.isCancelled || interrupted { shouldContinue = false; break }
                        for (offset, output) in outputs.enumerated() {
                            messages.append(["role": "tool", "tool_call_id": .string(batch[offset].id), "content": .string(output)])
                        }
                        trimHistory()
                        toolIndex = end
                        continue
                    }
                    toolsUsed += 1
                    if tool.name == "write_file" || tool.name == "edit_file" {
                        toolsSinceChange = 0
                        pacingAt = limits.pacingTools ?? Int.max
                    } else {
                        toolsSinceChange += 1
                    }
                    // The tool runs inside the turn's task: a stop cancels it with the turn,
                    // an approval it is waiting on comes back declined, and a process it
                    // launched is told to stop. Nothing it produced is said or remembered.
                    let output = await executeTool(tool)
                    if Task.isCancelled || interrupted { shouldContinue = false; break }
                    messages.append(["role": "tool", "tool_call_id": .string(tool.id), "content": .string(output)])
                    trimHistory()
                    toolIndex += 1
                }
                if !shouldContinue || interrupted { break }
            }
        } catch is CancellationError {
            await finishInterrupted()
            return
        } catch let error as ProviderError {
            onEvent?(.turnCompleted(status: .failed, error: error.localizedDescription))
            return
        } catch {
            if interrupted || Task.isCancelled {
                await finishInterrupted()
            } else {
                onEvent?(.turnCompleted(status: .failed, error: friendlyError(error.localizedDescription, statusCode: 0)))
            }
            return
        }

        if interrupted || Task.isCancelled {
            await finishInterrupted()
            return
        }
        if hitCeiling {
            // The tool results are in history, so "continue" resumes cleanly.
            onEvent?(.notice(Notice(level: .warning, message: "\(config.title) paused after \(Self.maxRoundsPerTurn) rounds of tool calls in one turn. Say “continue” to pick up where it left off.")))
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
            return
        }
        if interactionMode == .plan, !finalText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            onEvent?(.planCompleted(id: nextID("plan"), markdown: finalText))
        }
        onEvent?(.turnCompleted(status: .completed, error: nil))
    }

    func interrupt() async {
        interrupted = true
        sendTask?.cancel()
        roundTask?.cancel()
        roundTask = nil
        sseDrain?.cancel()
        sseDrain = nil
        for (_, continuation) in pendingApprovals { continuation.resume(returning: false) }
        pendingApprovals.removeAll()
    }

    func resolveApproval(_ requestID: String, optionID: String) {
        guard let continuation = pendingApprovals.removeValue(forKey: requestID) else { return }
        if optionID == "always" { approveAllRemaining = true }
        continuation.resume(returning: optionID == "allow" || optionID == "always")
        onEvent?(.requestResolved(id: requestID))
    }

    func answerQuestion(_ requestID: String, answers: [String: [String]]) {}

    func compact() async throws {
        // Keep the system prompt plus the most recent exchanges.
        guard messages.count > 12 else {
            onEvent?(.notice(Notice(level: .info, message: "Nothing to compact yet.")))
            return
        }
        let system = messages.first
        let tail = Array(messages.suffix(10))
        messages = (system.map { [$0] } ?? []) + tail
        // Slicing can orphan tool messages from their tool_calls.
        sanitizeHistory()
        onEvent?(.notice(Notice(level: .info, message: "Context compacted.")))
    }

    func stop() {
        isStopping = true
        interrupted = true
        sendTask?.cancel()
        roundTask?.cancel()
        roundTask = nil
        sseDrain?.cancel()
        sseDrain = nil
        for (_, continuation) in pendingApprovals { continuation.resume(returning: false) }
        pendingApprovals.removeAll()
        sessionID = nil
    }

    /// One API response's usage, reported in both shapes: the context-window
    /// `usage` event and the ledger keep what they always did, and the spend event
    /// carries the selected provider and model plus the response's own components.
    /// Called once per response, including the final-report response.
    private func reportUsage(_ round: StreamRound) {
        if let usage = round.usage {
            onEvent?(.usage(usage))
            TokenLedger.shared.record(spend: usage.usedTokens)
        }
        guard let reported = round.tokenUsage, reported.hasSpend else { return }
        onEvent?(.tokenSpend(TokenSpend(
            provider: config.kind,
            totalTokens: reported.total,
            inputTokens: reported.input,
            outputTokens: reported.output,
            cachedInputTokens: reported.cachedInput,
            model: currentModel
        )))
    }

    // MARK: - Chat round

    private func streamOneRound(model: String, effort: String?, withTools: Bool = true) async throws -> StreamRound {
        // History trims slice blindly and interrupted turns leave calls
        // unanswered; either poisons every later request, so repair first.
        sanitizeHistory()
        if withTools, !didLoadMCPTools { mcpTools = await MCPHub.shared.tools(); didLoadMCPTools = true }
        let payload = requestPayload(model: model, effort: effort, withTools: withTools)
        // A round is a request, and a dropped connection is answered by the same
        // request again, not by restarting the turn: the payload is built once, the
        // history it carries already holds every completed tool result, and a round
        // that failed is never returned, so its tool calls never run.
        var attempt = 0
        while true {
            try Task.checkCancellation()
            // A stopped turn does not spend its retries, and a failed drain does not
            // read on into the next attempt.
            if interrupted || isStopping { throw CancellationError() }
            sseDrain?.cancel()
            sseDrain = nil
            let task = Task<StreamRound, Error> { try await self.performStream(payload: payload) }
            roundTask = task
            do {
                let round = try await task.value
                roundTask = nil
                try Task.checkCancellation()
                if interrupted || isStopping { throw CancellationError() }
                return round
            } catch let failure as StreamAttemptFailure {
                roundTask = nil
                sseDrain?.cancel()
                sseDrain = nil
                try Task.checkCancellation()
                if interrupted || isStopping { throw CancellationError() }
                // Anything that is not a dropped connection keeps the error it came
                // with: only the transport's three codes are worth another request.
                guard Self.isTransientTransportFailure(failure.underlying) else { throw failure.underlying }
                guard attempt < Self.maxTransportRetries else {
                    throw ProviderError.failed(terminalTransportMessage(failure.underlying))
                }
                // Part of the answer was already shown: it is abandoned where it stands
                // (the next attempt takes fresh response ids) and the user is told why
                // the reply they were reading stopped.
                if failure.emittedOutput {
                    onEvent?(.notice(Notice(level: .warning, message: "\(config.title) lost the connection while answering. That reply was cut off, and this response is restarting.")))
                }
                let delay = Self.transportRetryDelays[min(attempt, Self.transportRetryDelays.count - 1)]
                attempt += 1
                try await Task.sleep(for: delay)
            } catch {
                roundTask = nil
                throw error
            }
        }
    }

    /// What is left when the connection keeps dropping: how many times the same
    /// request was tried, and the transport's own reason, so the user can act on it.
    private func terminalTransportMessage(_ error: Error) -> String {
        let reason = error.localizedDescription
        let attempts = Self.maxTransportRetries + 1
        let base = "\(config.title) could not reach the network after \(attempts) attempts. Check the connection, then try again."
        return reason.isEmpty ? base : "\(base) (\(reason))"
    }

    private func requestPayload(model: String, effort: String?, withTools: Bool = true) -> [String: JSONValue] {
        var payload: [String: JSONValue] = [
            "model": .string(model),
            "messages": .array(messages),
            "stream": true,
        ]
        if withTools {
            payload["tools"] = .array(Self.toolDefinitions + mcpToolDefinitions)
            payload["tool_choice"] = "auto"
        }
        applyProviderPayload(&payload, model: model, effort: effort, withTools: withTools)
        return payload
    }

    /// The per-provider payload fields. DeepSeek turns thinking on and maps the
    /// effort; Meta takes a top-level `reasoning_effort` (its Responses API nests
    /// it as `reasoning.effort`, not used here; omitting it reasons by default and
    /// "none" is rejected with HTTP 400); Z.ai sets `clear_thinking`, a `max_tokens`
    /// cap and `tool_stream`.
    func applyProviderPayload(_ payload: inout [String: JSONValue], model: String, effort: String?, withTools: Bool) {
        if let mapped = reasoningEffort(for: effort, model: model) { payload["reasoning_effort"] = .string(mapped) }
    }

    /// The subclass's effort mapping, applied when the payload is built.
    func reasoningEffort(for effort: String?, model: String) -> String? { nil }

    @concurrent
    private nonisolated static func encode(_ payload: [String: JSONValue]) async -> Data {
        JSONValue.object(payload).data()
    }

    private func performStream(payload: [String: JSONValue]) async throws -> StreamRound {
        var request = URLRequest(url: config.chatURL, timeoutInterval: 300)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        // The whole history goes out every round; serializing it belongs off the main actor.
        request.httpBody = await Self.encode(payload)
        if Task.isCancelled || interrupted { throw CancellationError() }

        let messageID = nextID("message")
        let thoughtID = nextID("thought")
        var content = ""
        var reasoning = ""
        var sawContent = false
        var sawReasoning = false
        struct Accumulator { var id = ""; var name = ""; var arguments = "" }
        var accumulators: [Int: Accumulator] = [:]
        var usage: ContextUsage?
        var tokenUsage: ReportedTokenUsage?
        var streamError: String?
        let statusCode = 200

        let bytes: URLSession.AsyncBytes
        let response: URLResponse
        do {
            (bytes, response) = try await URLSession.shared.bytes(for: request)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            // Nothing was streamed, but the error still goes up as it was thrown.
            throw StreamAttemptFailure(underlying: error, emittedOutput: false)
        }
        if let http = response as? HTTPURLResponse, http.statusCode != 200 {
            // Read the error body for a useful message.
            var body = ""
            for try await line in bytes.lines {
                body += line + "\n"
                if body.count > 8_000 { break }
            }
            var round = StreamRound()
            round.statusCode = http.statusCode
            round.rawError = errorMessage(fromBody: body, statusCode: http.statusCode)
            return round
        }

        // The byte loop and JSON parsing run in a detached task; only
        // coalesced chunks hop back here to emit.
        sseDrain?.cancel()
        let contextWindow = config.contextWindow
        let stream = AsyncStream<StreamChunk> { continuation in
            let coalescer = StreamCoalescer(continuation: continuation)
            let drain = Task.detached(priority: .userInitiated) {
                do {
                    try await Self.drainSSE(bytes: bytes, into: coalescer, contextWindow: contextWindow)
                } catch is CancellationError {
                    await coalescer.finish()
                } catch {
                    await coalescer.fail(chunkError: error)
                }
            }
            // A stop cancels the drain directly, so its per-line
            // Task.isCancelled check fires instead of it reading on. (Cancelling the
            // waiting task alone would leave the detached drain reading.)
            sseDrain = Task {
                await withTaskCancellationHandler { await drain.value } onCancel: { drain.cancel() }
            }
            // A stop cancels the consumer; cancelling the drain makes its
            // per-line Task.isCancelled check fire instead of reading on.
            continuation.onTermination = { _ in drain.cancel() }
        }
        defer { sseDrain = nil }
        for await chunk in stream {
            if Task.isCancelled || interrupted { throw CancellationError() }
            switch chunk {
            case .text(let text):
                content += text
                if !sawContent { sawContent = true }
                onEvent?(.messageDelta(id: messageID, text: text))
            case .reasoning(let think):
                reasoning += think
                if !sawReasoning { sawReasoning = true }
                onEvent?(.reasoningDelta(id: thoughtID, text: think))
            case .toolCall(let index, let id, let name, let args):
                var acc = accumulators[index] ?? Accumulator()
                if !id.isEmpty { acc.id = id }
                if !name.isEmpty { acc.name = name }
                acc.arguments += args
                accumulators[index] = acc
            case .usage(let value):
                usage = value
            case .tokenUsage(let value):
                tokenUsage = value
            case .streamError(let message):
                streamError = message
            case .transportError(let error):
                if let streamError, !streamError.isEmpty {
                    throw ProviderError.failed(friendlyError(streamError, statusCode: statusCode))
                }
                // Delegations can launch while text is streaming. Replaying that
                // response could repeat work already dispatched by the runtime.
                if CrewPrompts.streamedDelegations(in: content)?.delegations.isEmpty == false {
                    throw ProviderError.failed("\(error.localizedDescription) This response was not retried because it had already streamed a hand delegation.")
                }
                // The typed error (a URLError, normally) is carried up intact, with
                // what this attempt had already shown, so the caller can tell a
                // dropped connection from any other failure.
                throw StreamAttemptFailure(underlying: error, emittedOutput: sawContent || sawReasoning)
            }
        }

        var round = StreamRound()
        round.content = content
        round.reasoning = reasoning
        round.usage = usage
        round.tokenUsage = tokenUsage
        round.rawError = streamError
        round.statusCode = statusCode
        if sawContent || !content.isEmpty {
            onEvent?(.messageCompleted(id: messageID, text: content))
        }
        if sawReasoning || !reasoning.isEmpty {
            onEvent?(.reasoningCompleted(id: thoughtID, text: reasoning))
        }
        let ordered = accumulators.keys.sorted()
        for index in ordered {
            let acc = accumulators[index]!
            guard !acc.name.isEmpty else { continue }
            round.toolCalls.append(PendingToolCall(
                id: acc.id.isEmpty ? nextID("tool") : acc.id,
                name: acc.name,
                arguments: acc.arguments
            ))
        }
        return round
    }

    /// Runs off the main actor: trims, parses and picks apart each SSE line,
    /// then feeds chunks to the coalescer. A transport failure is delivered as
    /// a chunk; CancellationError is rethrown for the stop path.
    private nonisolated static func drainSSE(bytes: URLSession.AsyncBytes, into coalescer: StreamCoalescer, contextWindow: Int) async throws {
        do {
            for try await line in bytes.lines {
                if Task.isCancelled { throw CancellationError() }
                let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
                guard trimmed.hasPrefix("data:") else { continue }
                let data = trimmed.dropFirst(5).trimmingCharacters(in: .whitespaces)
                if data == "[DONE]" { break }
                guard let json = JSONValue.parse(String(data)) else { continue }
                if let err = json["error"], !err.isNull {
                    await coalescer.emitStreamError(err["message"]?.string ?? err.displayText)
                    continue
                }
                if let u = json["usage"], !u.isNull {
                    let input = u["prompt_tokens"]?.int ?? u["input_tokens"]?.int
                    let output = u["completion_tokens"]?.int ?? u["output_tokens"]?.int
                    let cached = u["prompt_tokens_details"]?["cached_tokens"]?.int
                        ?? u["input_tokens_details"]?["cached_tokens"]?.int
                        ?? u["prompt_cache_hit_tokens"]?.int
                    // Reported totals only: the cached count is already part of the
                    // input, and the context window says nothing about consumption.
                    let total = u["total_tokens"]?.int ?? ((input ?? 0) + (output ?? 0))
                    if total > 0 {
                        await coalescer.emitUsage(ContextUsage(usedTokens: total, windowTokens: contextWindow))
                    }
                    await coalescer.emitTokenUsage(ReportedTokenUsage(total: total, input: input, output: output, cachedInput: cached))
                }
                guard let choice = json["choices"]?.array?.first else { continue }
                let delta = choice["delta"] ?? .null
                if let text = delta["content"]?.string, !text.isEmpty {
                    await coalescer.appendText(text)
                }
                if let think = (delta["reasoning_content"]?.string ?? delta["reasoning"]?.string), !think.isEmpty {
                    await coalescer.appendReasoning(think)
                }
                for call in delta["tool_calls"]?.array ?? [] {
                    let index = call["index"]?.int ?? 0
                    let id = call["id"]?.string ?? ""
                    let name = call["function"]?["name"]?.string ?? ""
                    let args = call["function"]?["arguments"]?.string ?? ""
                    await coalescer.emitToolCall(index: index, id: id, name: name, arguments: args)
                }
            }
            await coalescer.finish()
        } catch is CancellationError {
            await coalescer.finish()
            throw CancellationError()
        } catch {
            await coalescer.fail(chunkError: error)
        }
    }

    private func errorMessage(fromBody body: String, statusCode: Int) -> String {
        if let json = JSONValue.parse(body) {
            let message = json["error"]?["message"]?.string ?? json["message"]?.string
            if let message, !message.isEmpty { return message }
        }
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return String(trimmed.prefix(500)) }
        return "\(config.title) returned status \(statusCode)."
    }

    /// Turns a provider's raw failure into something the user can act on.
    /// Every provider's mapping differs, so subclasses override.
    func friendlyError(_ message: String, statusCode: Int) -> String {
        message.isEmpty ? "\(config.title) stopped before finishing." : message
    }

    private func finishInterrupted() async {
        for (_, continuation) in pendingApprovals { continuation.resume(returning: false) }
        pendingApprovals.removeAll()
        onEvent?(.turnCompleted(status: .interrupted, error: nil))
    }

    // MARK: - Tools

    /// The same six tools every round, built once.
    private static let toolDefinitions: [JSONValue] = [
            ["type": "function", "function": [
                "name": "read_file",
                "description": "Read a file inside the project. Path is relative to the project root. The whole file by default; for a big file (hundreds of lines) pass start_line and line_count to read just the part you need, and the result says how many lines the file has.",
                "parameters": ["type": "object", "properties": [
                    "path": ["type": "string", "description": "Relative file path"],
                    "start_line": ["type": "integer", "description": "First line to read, counted from 1"],
                    "line_count": ["type": "integer", "description": "How many lines to read from start_line"],
                ], "required": ["path"]],
            ]],
            ["type": "function", "function": [
                "name": "list_files",
                "description": "List files in a directory inside the project.",
                "parameters": ["type": "object", "properties": ["path": ["type": "string", "description": "Relative directory, empty for the root"], "recursive": ["type": "boolean"]]],
            ]],
            ["type": "function", "function": [
                "name": "search_text",
                "description": "Search file contents for a fixed string or regex.",
                "parameters": ["type": "object", "properties": ["pattern": ["type": "string"], "path": ["type": "string", "description": "Relative directory or file"], "regex": ["type": "boolean"]], "required": ["pattern"]],
            ]],
            ["type": "function", "function": [
                "name": "write_file",
                "description": "Create or overwrite a file inside the project with the full new content.",
                "parameters": ["type": "object", "properties": ["path": ["type": "string"], "content": ["type": "string"]], "required": ["path", "content"]],
            ]],
            ["type": "function", "function": [
                "name": "edit_file",
                "description": "Replace one exact block of text in a file with new text. old_string must appear exactly once.",
                "parameters": ["type": "object", "properties": ["path": ["type": "string"], "old_string": ["type": "string"], "new_string": ["type": "string"]], "required": ["path", "old_string", "new_string"]],
            ]],
            ["type": "function", "function": [
                "name": "run_command",
                "description": "Run a shell command in the project root and capture its output.",
                "parameters": ["type": "object", "properties": ["command": ["type": "string"]], "required": ["command"]],
            ]],
        ]

    /// The most built-in reads one batch may have in flight.
    private static let maxConcurrentReads = 4

    /// The tool kind of the three built-in reads, nil for everything else. Only
    /// calls whose kind runs without approval take part in a concurrent batch.
    private static func readOnlyKind(for name: String) -> ToolCall.Kind? {
        switch name {
        case "read_file": return .read
        case "list_files", "search_text": return .search
        default: return nil
        }
    }

    /// Runs a group of approval-free built-in reads, at most four at a time, and
    /// returns their results in the order the model asked for them. The child tasks
    /// are structured, so stopping the turn cancels every read still in flight and
    /// a cancelled read's output is never appended.
    private func executeReadBatch(_ tools: [PendingToolCall]) async -> [String] {
        var outputs = [String](repeating: "", count: tools.count)
        await withTaskGroup(of: (Int, String).self) { group in
            let limit = min(Self.maxConcurrentReads, tools.count)
            var next = 0
            while next < limit {
                let index = next
                group.addTask { (index, await self.executeTool(tools[index])) }
                next += 1
            }
            while let (index, output) = await group.next() {
                guard !Task.isCancelled, !interrupted else { group.cancelAll(); break }
                outputs[index] = output
                if next < tools.count {
                    let index = next
                    group.addTask { (index, await self.executeTool(tools[index])) }
                    next += 1
                }
            }
        }
        return outputs
    }

    private func executeTool(_ tool: PendingToolCall) async -> String {
        guard !Task.isCancelled, !interrupted else { return "Stopped." }
        let args = JSONValue.parse(tool.arguments) ?? .object([:])
        let callID = tool.id
        if let (server, name) = MCPHub.parse(callName: tool.name) {
            onEvent?(.toolStarted(id: callID, call: ToolCall(kind: .mcp, title: "\(server) · \(name)", detail: args.isNull ? nil : args.prettyString)))
            guard await approveIfNeeded(kind: .mcp, title: "Call \(server) \(name)", detail: args.compactString, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                let result = try await MCPHub.shared.call(tool.name, arguments: args)
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                if result.isError {
                    onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: result.text, status: .failed)))
                    return result.text.isEmpty ? "Error: the MCP tool reported failure with no output." : result.text
                }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: summary(result.text, limit: 12_000), status: .completed)))
                return result.text.isEmpty ? "(no output)" : result.text
            } catch {
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        }
        switch tool.name {
        case "read_file":
            let path = args["path"]?.string ?? ""
            let call = ToolCall(kind: .read, title: fileTools.displayPath(path))
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .read, title: "Read \(fileTools.displayPath(path))", detail: path, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                let text = try await fileTools.readFile(path, startLine: args["start_line"]?.int, lineCount: args["line_count"]?.int)
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: summary(text, limit: 4_000), status: .completed)))
                return text
            } catch {
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "list_files":
            let path = args["path"]?.string ?? ""
            // Same shape as Claude's LS row: the folder is the subject.
            let call = ToolCall(kind: .search, title: path.isEmpty ? (workingDirectory as NSString).lastPathComponent : fileTools.displayPath(path))
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .search, title: call.title, detail: path, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                let text = try await fileTools.listFiles(path, recursive: args["recursive"]?.bool ?? false)
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: summary(text, limit: 4_000), status: .completed)))
                return text
            } catch {
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "search_text":
            let pattern = args["pattern"]?.string ?? ""
            let path = args["path"]?.string ?? ""
            let call = ToolCall(kind: .search, title: pattern.isEmpty ? "Search" : pattern, detail: path.isEmpty ? nil : fileTools.displayPath(path))
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .search, title: "Search for “\(pattern)”", detail: path.isEmpty ? nil : path, toolItemID: nil) else {
                return declined(callID)
            }
            let text = await searchText(pattern: pattern, path: path, regex: args["regex"]?.bool ?? false)
            guard !Task.isCancelled, !interrupted else { return "Stopped." }
            onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: summary(text, limit: 6_000), status: .completed)))
            return text
        case "write_file":
            let path = args["path"]?.string ?? ""
            let content = args["content"]?.string ?? ""
            var call = ToolCall(kind: .edit, title: fileTools.displayPath(path))
            call.edits = [FileEdit(path: fileTools.displayPath(path), old: "", new: content)]
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .edit, title: "Write \(fileTools.displayPath(path))", detail: nil, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                try await fileTools.writeFile(path: path, content: content)
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(
                    output: "Wrote \(fileTools.displayPath(path)) (\(content.utf8.count) bytes).",
                    status: .completed,
                    edits: [FileEdit(path: fileTools.displayPath(path), old: "", new: content)]
                )))
                return "Wrote \(fileTools.displayPath(path)) (\(content.utf8.count) bytes)."
            } catch {
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "edit_file":
            let path = args["path"]?.string ?? ""
            let old = args["old_string"]?.string ?? args["old_text"]?.string ?? ""
            let new = args["new_string"]?.string ?? args["new_text"]?.string ?? ""
            var call = ToolCall(kind: .edit, title: fileTools.displayPath(path))
            call.edits = [FileEdit(path: fileTools.displayPath(path), old: old, new: new)]
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .edit, title: "Edit \(fileTools.displayPath(path))", detail: nil, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                try await fileTools.editFile(path: path, old: old, new: new)
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(
                    output: "Edited \(fileTools.displayPath(path)).",
                    status: .completed,
                    edits: [FileEdit(path: fileTools.displayPath(path), old: old, new: new)]
                )))
                return "Edited \(fileTools.displayPath(path))."
            } catch {
                guard !Task.isCancelled, !interrupted else { return "Stopped." }
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "run_command":
            let command = ToolTitles.unwrapShell(args["command"]?.string ?? "")
            let call = ToolCall(kind: .command, title: command.isEmpty ? "Run command" : command)
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .command, title: command, detail: nil, toolItemID: nil) else {
                return declined(callID)
            }
            let result = await runCommand(command)
            // The user stopped the turn while the command ran: say no more about it.
            guard !Task.isCancelled, !interrupted else { return "Stopped." }
            var update = ToolUpdate()
            update.output = result.output.isEmpty ? "" : summary(result.output, limit: 12_000)
            update.exitCode = Int(result.exitCode)
            update.status = result.exitCode == 0 ? .completed : .failed
            onEvent?(.toolUpdated(id: callID, update: update))
            return result.output.isEmpty ? "(exit \(result.exitCode), no output)" : "exit \(result.exitCode)\n\(summary(result.output, limit: 12_000))"
        default:
            let call = ToolCall(kind: .other, title: tool.name)
            onEvent?(.toolStarted(id: callID, call: call))
            onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: "Unknown tool.", status: .failed)))
            return "Error: unknown tool \(tool.name)."
        }
    }

    private func declined(_ callID: String) -> String {
        onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: "The user declined this action.", status: .declined)))
        return "The user declined this action. Ask how to proceed instead of retrying it."
    }

    /// Whether this kind of tool would ask the user before running. The concurrent
    /// read batch uses it to stay off the approval path entirely: only calls this
    /// returns false for may run together.
    ///
    /// Plan mode is a promise the turn keeps, not a preference, so it is decided before
    /// "approve for turn" rather than after it: one click of that button on a read used
    /// to switch the promise off for every edit and command left in the turn.
    private func requiresApproval(kind: ToolCall.Kind) -> Bool {
        let planBlocks = interactionMode == .plan && (kind == .edit || kind == .command)
        if approveAllRemaining, !planBlocks { return false }
        if planBlocks { return true }
        switch runtimeMode {
        case .supervised: return true
        case .autoAcceptEdits, .auto: return kind == .command
        case .fullAccess: return false
        }
    }

    private func approveIfNeeded(kind: ToolCall.Kind, title: String, detail: String?, toolItemID: String?) async -> Bool {
        // A stopped turn declines whatever is still asking, before anything else.
        if interrupted { return false }
        guard requiresApproval(kind: kind) else { return true }
        let id = nextID("approval")
        let approvalKind: ApprovalRequest.Kind = switch kind {
        case .command: .command
        case .edit: .fileChange
        default: .tool
        }
        let options: [ApprovalRequest.Option] = [
            .init(id: "allow", title: "Approve", role: .approve),
            .init(id: "always", title: "Approve for turn", role: .approveAlways),
            .init(id: "decline", title: "Decline", role: .decline),
        ]
        await MainActor.run {
            self.onEvent?(.approval(ApprovalRequest(id: id, kind: approvalKind, title: title, detail: detail, options: options, toolItemID: toolItemID)))
        }
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                guard !Task.isCancelled, !interrupted else {
                    continuation.resume(returning: false)
                    return
                }
                pendingApprovals[id] = continuation
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.pendingApprovals.removeValue(forKey: id)?.resume(returning: false)
            }
        }
    }

    // MARK: - Local execution

    /// Old tool results give way once history carries too much of them. Every round
    /// re-sends the whole history, and a result the model read twenty rounds ago is
    /// only weight by now; the most recent ones stay whole, since an edit is written
    /// from what was just read. A condensed result keeps its head and says how to get
    /// the rest back.
    private func condenseHistory() {
        var toolIndices: [Int] = []
        var total = 0
        for (index, message) in messages.enumerated() where message["role"]?.string == "tool" {
            toolIndices.append(index)
            total += message["content"]?.string?.utf8.count ?? 0
        }
        guard total > Self.condenseHistoryAt else { return }
        for index in toolIndices.dropLast(Self.condenseKeepsRecent) {
            guard total > Self.condenseHistoryTo else { break }
            guard var message = messages[index].object, let content = message["content"]?.string,
                  content.utf8.count > 1_200, !content.contains(Self.condensedMarker) else { continue }
            let kept = String(content.prefix(400))
            let condensed = kept + "\n\(Self.condensedMarker) (\(content.utf8.count) bytes); call the tool again if you need it]"
            message["content"] = .string(condensed)
            messages[index] = .object(message)
            total -= content.utf8.count - condensed.utf8.count
        }
    }

    private func searchText(pattern: String, path: String, regex: Bool) async -> String {
        let trimmed = pattern.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "Error: a search pattern is required." }
        let directory = URL(fileURLWithPath: workingDirectory)
        // Search is the one file tool that used to hand the model's path to the shell
        // untouched, so an absolute path or a climb out of the root recursively grepped
        // ~/.ssh and returned the matches into the transcript. It answers to the same
        // guard as read, list, write and edit now.
        let target: String
        if path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            target = "."
        } else {
            do {
                target = try fileTools.resolveURL(path).path
            } catch {
                return "Error: \(error.localizedDescription)"
            }
        }
        let grep = URL(fileURLWithPath: "/usr/bin/grep")
        var args = ["-R", "-n", "-I", "--exclude-dir=.git", "--exclude-dir=node_modules", "--exclude-dir=.build", "--exclude-dir=build"]
        args.append(regex ? "-E" : "-F")
        // Everything past `--` is an operand, so a pattern or a path that starts with a
        // dash is searched for rather than parsed as a flag.
        args.append("--")
        args.append(trimmed)
        args.append(target)
        guard let result = try? await Shell.run(grep, args, in: directory, environment: LoginEnvironment.current, timeout: 30, outputLimit: 64_000) else {
            return "Search failed."
        }
        let output = (result.output + result.errorOutput).trimmingCharacters(in: .whitespacesAndNewlines)
        if output.isEmpty { return "(no matches)" }
        let lines = output.split(separator: "\n")
        let limited = lines.prefix(120).joined(separator: "\n")
        return lines.count > 120 ? limited + "\n…(\(lines.count) matches, truncated)" : String(limited)
    }

    private func runCommand(_ command: String) async -> (output: String, exitCode: Int32) {
        let trimmed = command.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return ("Error: a command is required.", 1) }
        let shell = URL(fileURLWithPath: "/bin/zsh")
        let directory = URL(fileURLWithPath: workingDirectory)
        guard let result = try? await Shell.run(shell, ["-lc", trimmed], in: directory, environment: LoginEnvironment.current, timeout: 120, outputLimit: 24_000) else {
            return ("The command could not start.", 1)
        }
        var output = (result.output + (result.errorOutput.isEmpty ? "" : "\n" + result.errorOutput)).trimmingCharacters(in: .whitespacesAndNewlines)
        if output.utf8.count > 24_000 { output = "…" + String(output.suffix(24_000)) }
        return (output, result.status)
    }

    // MARK: - Messages

    /// Rebuilds the system prompt in place so the current project guidelines are the
    /// ones the model sees. Only the system message is touched; the higher-priority
    /// policy inside it is rebuilt identically.
    private func refreshSystemPrompt() {
        guard let first = messages.first, first["role"]?.string == "system" else { return }
        messages[0] = ["role": "system", "content": .string(systemPrompt())]
    }

    private func systemPrompt() -> String {
        let modeLine = switch (interactionMode, runtimeMode) {
        case (.plan, _):
            "You are in PLAN mode: research the project, then write a clear step-by-step plan. Do not edit files or run commands that change anything; reads, lists and searches are fine."
        case (.build, .supervised):
            "You are in BUILD mode with supervised permissions: use tools freely but the user approves each one in Bottega."
        case (.build, .autoAcceptEdits), (.build, .auto):
            "You are in BUILD mode: read and edit files freely; the user approves shell commands in Bottega."
        case (.build, .fullAccess):
            "You are in BUILD mode with full access: use tools freely without asking."
        }
        // The optional Crew policy and the anti-slop guidance go in once each. The off
        // notice goes out too, so a resumed thread cannot keep earlier guidance alive.
        let policy = [
            configuration.crew.map(CrewPrompts.fallbackPolicy),
            AntiSlopPolicy.instructions(enabled: configuration.antiSlopEnabled),
        ]
        .compactMap { $0 }
        .joined(separator: "\n\n")
        // The project's own root guidelines come last, so the policy above stays
        // higher priority. They are re-read here only when the files changed.
        let appendedPolicy = [policy.isEmpty ? nil : policy, guidance.systemPromptBlock()]
            .compactMap { $0 }
            .joined(separator: "\n\n")
        return """
        You are Bottega's \(config.agentIdentity(currentModel)), working inside \(workingDirectory) on macOS.
        \(modeLine)
        Rules:
        - Prefer the provided tools over asking the user to run things. Read files before editing them.
        - Keep file paths relative to the project root and never touch paths outside it.
        - Report the result and material limitations when the task is complete.
        - Never hard-wrap prose at a fixed column: write each paragraph of a reply as one unbroken
          line, however long, so a copied reply keeps its full line length wherever it is pasted.
          Lines inside a fenced code block keep their own breaks.
        - If a tool result shows the user declined an action, do not retry it: ask how to proceed.
        - Today's date is \(ISO8601DateFormatter().string(from: Date())).
        \n\(appendedPolicy)
        """
    }

    private func userContentValue(text: String, images: [Attachment]) async -> JSONValue {
        let usable = images.filter(\.isImage).prefix(4)
        guard !usable.isEmpty, config.isVisionModel(currentModel) else {
            if usable.isEmpty { return .string(text) }
            let names = usable.map(\.name).joined(separator: ", ")
            return .string(text + "\n\n[Attached images (model cannot see them): \(names)]")
        }
        var parts: [JSONValue] = [["type": "text", "text": .string(text)]]
        for (image, base64) in await Attachment.base64Images(Array(usable)) {
            let url = "data:\(image.mimeType);base64,\(base64)"
            parts.append(["type": "image_url", "image_url": ["url": .string(url)]])
        }
        return .array(parts)
    }

    /// DeepSeek's thinking mode rejects the follow-up request with "The
    /// `reasoning_content` in the thinking mode must be passed back to the API"
    /// unless every tool-call message carries it (an empty string is accepted);
    /// where reasoning is not replayed, the field is left off entirely.
    private func assistantTurn(content: String, reasoning: String, toolCalls: [PendingToolCall]) -> JSONValue {
        var message: [String: JSONValue] = ["role": "assistant", "content": .string(content)]
        if config.replaysReasoning { message["reasoning_content"] = .string(reasoning) }
        if !toolCalls.isEmpty {
            message["tool_calls"] = .array(toolCalls.map { tool in
                ["id": .string(tool.id), "type": "function", "function": ["name": .string(tool.name), "arguments": .string(tool.arguments)]]
            })
        }
        return .object(message)
    }

    private func trimHistory() {
        // System prompt plus roughly the last 40 exchanges; each exchange is
        // user + assistant + tool messages, so cap the raw message count.
        // The running turn is never cut: a blind suffix dropped its user
        // message once the turn passed 50 messages, leaving the model tool
        // results with no task, so it stopped and asked what to do.
        guard messages.count > 60 else { return }
        let system = messages.first
        let body = messages.dropFirst()
        let turnStart = body.lastIndex { $0["role"]?.string == "user" } ?? body.endIndex
        let current = Array(body[turnStart...])
        let earlier = Array(body[..<turnStart].suffix(max(0, 50 - current.count)))
        messages = (system.map { [$0] } ?? []) + earlier + current
    }

    /// Repairs tool-call chains after trimming, compaction or interruption.
    /// These APIs reject any history where a `tool` message has no preceding
    /// `assistant` message with a matching `tool_calls` id (and vice versa).
    /// Once such an orphan exists, every later request fails identically, so
    /// no retry can heal it — this drops orphaned tool messages and strips
    /// tool calls that lost their answers, keeping the history sendable.
    private func sanitizeHistory() {
        var clean: [JSONValue] = []
        clean.reserveCapacity(messages.count)
        // Assistant messages in `clean` still awaiting tool answers.
        var open: [(index: Int, ids: Set<String>)] = []

        func closeOpen() {
            // Calls that can never be answered now must go. Applied
            // last-index-first so removals never shift pending indices.
            for entry in open.sorted(by: { $0.index > $1.index }) {
                guard case .object(var values) = clean[entry.index],
                      let calls = values["tool_calls"]?.array else { continue }
                let kept = calls.filter { call in
                    guard let id = call["id"]?.string, !id.isEmpty else { return false }
                    return !entry.ids.contains(id)
                }
                if kept.isEmpty {
                    values.removeValue(forKey: "tool_calls")
                    let content = (values["content"]?.string ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                    if content.isEmpty {
                        clean.remove(at: entry.index)
                    } else {
                        clean[entry.index] = .object(values)
                    }
                } else {
                    values["tool_calls"] = .array(kept)
                    clean[entry.index] = .object(values)
                }
            }
            open.removeAll()
        }

        for message in messages {
            let role = message["role"]?.string ?? ""
            if role == "tool" {
                let id = message["tool_call_id"]?.string ?? ""
                if let slot = open.firstIndex(where: { $0.ids.contains(id) }) {
                    clean.append(message)
                    open[slot].ids.remove(id)
                    if open[slot].ids.isEmpty { open.remove(at: slot) }
                }
                // else: orphaned by a trim/compact cut — drop it.
                continue
            }
            if role == "assistant", let calls = message["tool_calls"]?.array, !calls.isEmpty {
                closeOpen()
                clean.append(message)
                let ids = Set(calls.compactMap { $0["id"]?.string }.filter { !$0.isEmpty })
                if !ids.isEmpty { open.append((clean.count - 1, ids)) }
                continue
            }
            closeOpen()
            clean.append(message)
        }
        closeOpen()
        messages = clean
    }

    private func summary(_ text: String, limit: Int) -> String {
        // Bytes bound characters, so a short output skips the grapheme count.
        guard text.utf8.count > limit, text.count > limit else { return text }
        return String(text.prefix(limit)) + "\n…(truncated)"
    }

    private func nextID(_ prefix: String) -> String {
        counter += 1
        return "\(prefix)-\(counter)-\(UUID().uuidString.prefix(8))"
    }
}
