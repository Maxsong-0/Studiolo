import Foundation

/// Native Z.ai agent: talks to https://api.z.ai/api/coding/paas/v4/chat/completions
/// directly (OpenAI-compatible, the GLM Coding Plan endpoint) and executes coding
/// tools locally, so no CLI install is needed — just an API key in Settings → Providers → Z.ai.
@MainActor
final class ZaiSession: ProviderSession {
    var onEvent: ((ProviderEvent) -> Void)?

    private let configuration: SessionConfiguration
    private var sessionID: String?
    private var messages: [JSONValue] = []
    private var runtimeMode: RuntimeMode
    private var interactionMode: InteractionMode
    private var currentModel: String
    private var counter = 0
    private var interrupted = false
    private var isStopping = false
    private var approveAllRemaining = false
    private var roundTask: Task<StreamRound, Error>?
    /// The detached SSE drain for the in-flight round, so a stop cancels it
    /// directly instead of waiting for the next chunk to reach the consumer.
    private var sseDrain: Task<Void, Never>?
    /// The tool running right now, in a task of its own so a stop can cancel it.
    /// A shell command can hold the turn for two minutes, and nothing it produced
    /// should reach the timeline or the history once the user has stopped the turn.
    private var toolTask: Task<String, Never>?
    private var pendingApprovals: [String: CheckedContinuation<Bool, Never>] = [:]

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
    private static let condensedMarker = "…[Droppy Code condensed this output"

    private struct StreamRound: Sendable {
        var content = ""
        var reasoning = ""
        var toolCalls: [PendingToolCall] = []
        var usage: ContextUsage?
        var rawError: String?
        var statusCode: Int = 200
    }

    private struct PendingToolCall: Sendable {
        var id: String
        var name: String
        var arguments: String
    }

    /// One parsed SSE value, ready for the main actor to emit.
    private enum StreamChunk: Sendable {
        case text(String)
        case reasoning(String)
        case toolCall(index: Int, id: String, name: String, arguments: String)
        case usage(ContextUsage)
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

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
        interactionMode = configuration.interactionMode
        currentModel = configuration.model ?? "glm-5.3"
    }

    var isRunning: Bool { sessionID != nil && !isStopping }

    private var workingDirectory: String { configuration.workingDirectory.path }
    private var apiKey: String { (configuration.apiKey ?? "").trimmingCharacters(in: .whitespacesAndNewlines) }

    // MARK: - Lifecycle

    func start() async throws -> String {
        guard !apiKey.isEmpty else { throw ProviderError.notInstalled(.zai) }
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
        guard sessionID != nil, !isStopping else { throw ProviderError.notRunning }
        guard !apiKey.isEmpty else { throw ProviderError.notInstalled(.zai) }
        runtimeMode = input.runtimeMode
        interactionMode = input.interactionMode
        if let model = input.model, !model.isEmpty { currentModel = model }
        interrupted = false
        approveAllRemaining = false

        let userContent = userContentValue(text: input.text, images: input.images)
        messages.append(["role": "user", "content": userContent])
        trimHistory()
        onEvent?(.turnStarted(providerTurnID: nil))

        var finalText = ""
        var rounds = 0
        var toolsSinceChange = 0
        var pacingAt = HydraBudget.pacingTools
        var toolsUsed = 0
        var wrapUpAsked = false
        let turnStartedAt = Date.now
        var hitCeiling = false
        do {
            while !interrupted {
                guard rounds < Self.maxRoundsPerTurn else { hitCeiling = true; break }
                rounds += 1
                // A head past its budget writes its report with no tools left to reach
                // for; a head stopped by its runtime for the same reason answers the same way.
                let budgetSpent = configuration.isHydraHead && (toolsUsed >= HydraBudget.maxTools || Date.now.timeIntervalSince(turnStartedAt) >= HydraBudget.maxSeconds)
                if input.isFinalReport || budgetSpent {
                    if budgetSpent { messages.append(["role": "user", "content": .string(HydraBudget.finalNote)]) }
                    let last = try await streamOneRound(model: currentModel, effort: input.effort, withTools: false)
                    if let usage = last.usage {
                        onEvent?(.usage(usage))
                        TokenLedger.shared.record(spend: usage.usedTokens)
                    }
                    if let error = last.rawError, !error.isEmpty { throw ProviderError.failed(friendlyError(error, statusCode: last.statusCode)) }
                    if !last.content.isEmpty {
                        finalText = last.content
                        messages.append(["role": "assistant", "content": .string(last.content), "reasoning_content": .string(last.reasoning)])
                    }
                    break
                }
                if configuration.isHydraHead, !wrapUpAsked, toolsUsed >= HydraBudget.wrapUpTools || Date.now.timeIntervalSince(turnStartedAt) >= HydraBudget.wrapUpSeconds {
                    wrapUpAsked = true
                    messages.append(["role": "user", "content": .string(HydraBudget.wrapUpNote)])
                }
                // A head that only ever reads is paced: every so many tools without a
                // change, a note in its history asks it to act or report. Its lead is
                // waiting, and a research task has an answer by then.
                if configuration.isHydraHead, toolsSinceChange >= pacingAt {
                    messages.append(["role": "user", "content": .string(HydraBudget.pacingNote)])
                    pacingAt = toolsSinceChange + HydraBudget.pacingTools
                }
                condenseHistory()
                let round = try await streamOneRound(model: currentModel, effort: input.effort)
                // A round is one API response, so its total is exactly that
                // response's spend.
                if let usage = round.usage {
                    onEvent?(.usage(usage))
                    TokenLedger.shared.record(spend: usage.usedTokens)
                }
                if round.statusCode == 401 || (round.rawError?.localizedCaseInsensitiveContains("invalid") == true && round.rawError?.localizedCaseInsensitiveContains("key") == true) {
                    throw ProviderError.failed("Z.ai rejected the API key. Check it in Settings → Providers → Z.ai.")
                }
                if let error = round.rawError, !error.isEmpty {
                    throw ProviderError.failed(friendlyError(error, statusCode: round.statusCode))
                }
                if !round.content.isEmpty { finalText = round.content }
                if round.toolCalls.isEmpty {
                    // Keep the reply in history, or the next turn forgets it.
                    if !round.content.isEmpty {
                        messages.append(["role": "assistant", "content": .string(round.content), "reasoning_content": .string(round.reasoning)])
                    }
                    break
                }
                // Record the assistant turn with its tool calls so the next
                // request keeps the OpenAI tool-call chain intact.
                messages.append(assistantMessage(content: round.content, reasoning: round.reasoning, toolCalls: round.toolCalls))
                var shouldContinue = true
                for tool in round.toolCalls {
                    if interrupted { shouldContinue = false; break }
                    toolsUsed += 1
                    if tool.name == "write_file" || tool.name == "edit_file" {
                        toolsSinceChange = 0
                        pacingAt = HydraBudget.pacingTools
                    } else {
                        toolsSinceChange += 1
                    }
                    let running = Task { [weak self] in await self?.executeTool(tool) ?? "" }
                    toolTask = running
                    let output = await running.value
                    toolTask = nil
                    // Stopped while the tool ran: its output is not recorded and no further
                    // tool starts. A process already launched runs to its own end or its
                    // timeout, since Shell.run cannot be cancelled, but nothing it produced
                    // is said or remembered.
                    if running.isCancelled || interrupted { shouldContinue = false; break }
                    messages.append(["role": "tool", "tool_call_id": .string(tool.id), "content": .string(output)])
                    trimHistory()
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
            if interrupted {
                await finishInterrupted()
            } else {
                onEvent?(.turnCompleted(status: .failed, error: friendlyError(error.localizedDescription, statusCode: 0)))
            }
            return
        }

        if interrupted {
            await finishInterrupted()
            return
        }
        if hitCeiling {
            // The tool results are in history, so "continue" resumes cleanly.
            onEvent?(.notice(Notice(level: .warning, message: "Z.ai paused after \(Self.maxRoundsPerTurn) rounds of tool calls in one turn. Say “continue” to pick up where it left off.")))
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
        roundTask?.cancel()
        roundTask = nil
        sseDrain?.cancel()
        sseDrain = nil
        toolTask?.cancel()
        toolTask = nil
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
        roundTask?.cancel()
        roundTask = nil
        sseDrain?.cancel()
        sseDrain = nil
        toolTask?.cancel()
        toolTask = nil
        for (_, continuation) in pendingApprovals { continuation.resume(returning: false) }
        pendingApprovals.removeAll()
        sessionID = nil
    }

    // MARK: - Chat round

    private func streamOneRound(model: String, effort: String?, withTools: Bool = true) async throws -> StreamRound {
        // History trims slice blindly and interrupted turns leave calls
        // unanswered; either poisons every later request, so repair first.
        sanitizeHistory()
        let payload = requestPayload(model: model, effort: effort, withTools: withTools)
        let task = Task<StreamRound, Error> { try await self.performStream(payload: payload) }
        roundTask = task
        defer { roundTask = nil }
        return try await task.value
    }

    private func requestPayload(model: String, effort: String?, withTools: Bool = true) -> [String: JSONValue] {
        var payload: [String: JSONValue] = [
            "model": .string(model),
            "messages": .array(messages),
            "stream": true,
        ]
        if withTools {
            payload["tools"] = .array(toolDefinitions())
            payload["tool_choice"] = "auto"
            payload["tool_stream"] = true
        }
        payload["thinking"] = ["type": "enabled", "clear_thinking": false]
        payload["max_tokens"] = .int(ZaiAPI.maxOutputTokens)
        if let mapped = Self.reasoningEffort(effort) { payload["reasoning_effort"] = .string(mapped) }
        return payload
    }

    static func reasoningEffort(_ effort: String?) -> String? {
        guard let effort, !effort.isEmpty else { return nil }
        switch effort {
        case "low": return "low"
        case "medium", "high": return "high"
        case "xhigh", "extra-high", "max": return "max"
        default: return nil
        }
    }

    private func performStream(payload: [String: JSONValue]) async throws -> StreamRound {
        var request = URLRequest(url: ZaiAPI.chatURL, timeoutInterval: 300)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = JSONValue.object(payload).data()
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
        var streamError: String?
        let statusCode = 200

        let (bytes, response) = try await URLSession.shared.bytes(for: request)
        if let http = response as? HTTPURLResponse, http.statusCode != 200 {
            // Read the error body for a useful message.
            var body = ""
            for try await line in bytes.lines {
                body += line + "\n"
                if body.count > 8_000 { break }
            }
            var round = StreamRound()
            round.statusCode = http.statusCode
            round.rawError = Self.errorMessage(fromBody: body, statusCode: http.statusCode)
            return round
        }

        // The byte loop and JSON parsing run in a detached task; only
        // coalesced chunks hop back here to emit.
        sseDrain?.cancel()
        let stream = AsyncStream<StreamChunk> { continuation in
            let coalescer = StreamCoalescer(continuation: continuation)
            let drain = Task.detached(priority: .userInitiated) {
                do {
                    try await Self.drainSSE(bytes: bytes, into: coalescer, contextWindow: ZaiAPI.contextWindow)
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
            case .streamError(let message):
                streamError = message
            case .transportError(let error):
                throw error
            }
        }

        var round = StreamRound()
        round.content = content
        round.reasoning = reasoning
        round.usage = usage
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
                    let total = u["total_tokens"]?.int ?? 0
                    if total > 0 {
                        await coalescer.emitUsage(ContextUsage(usedTokens: total, windowTokens: contextWindow))
                    }
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

    private static func errorMessage(fromBody body: String, statusCode: Int) -> String {
        if let json = JSONValue.parse(body) {
            let message = json["error"]?["message"]?.string ?? json["message"]?.string
            if let message, !message.isEmpty { return message }
        }
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return String(trimmed.prefix(500)) }
        return "Z.ai returned status \(statusCode)."
    }

    private func friendlyError(_ message: String, statusCode: Int) -> String {
        let lower = message.lowercased()
        // A context-length message also says "limit", so it is read before the plan window.
        if lower.contains("context") && (lower.contains("length") || lower.contains("window") || lower.contains("token")) {
            return "The conversation no longer fits GLM-5.3's context window. Start a new thread or compact first."
        }
        if statusCode == 401 || lower.contains("token expired") || lower.contains("incorrect") {
            return "Z.ai rejected the API key. Check it in Settings → Providers → Z.ai."
        }
        // "usage limit" is what auto-continue listens for; it then reads the reset time from Z.ai's quota API.
        if statusCode == 429 || lower.contains("quota") || lower.contains("exhaust") || lower.contains("limit") {
            return "Your GLM Coding Plan usage limit is reached for this window. It refreshes on its own; the usage ring shows when. Details at z.ai/manage-apikey/subscription."
        }
        if lower.contains("balance") || lower.contains("insufficient") || lower.contains("1113") {
            return "This Z.ai key has no active Coding Plan. Subscribe at z.ai/subscribe, then try again."
        }
        if lower.contains("model") && lower.contains("not exist") {
            return "Z.ai no longer offers that model. Pick GLM-5.3 or GLM-5.3 Flash in the model picker."
        }
        return message.isEmpty ? "Z.ai stopped before finishing." : message
    }

    private func finishInterrupted() async {
        for (_, continuation) in pendingApprovals { continuation.resume(returning: false) }
        pendingApprovals.removeAll()
        onEvent?(.turnCompleted(status: .interrupted, error: nil))
    }

    // MARK: - Tools

    private func toolDefinitions() -> [JSONValue] {
        [
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
    }

    private func executeTool(_ tool: PendingToolCall) async -> String {
        let args = JSONValue.parse(tool.arguments) ?? .object([:])
        let callID = tool.id
        switch tool.name {
        case "read_file":
            let path = args["path"]?.string ?? ""
            let call = ToolCall(kind: .read, title: displayPath(path))
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .read, title: "Read \(displayPath(path))", detail: path, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                let text = try readFile(path, startLine: args["start_line"]?.int, lineCount: args["line_count"]?.int)
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: summary(text, limit: 4_000), status: .completed)))
                return text
            } catch {
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "list_files":
            let path = args["path"]?.string ?? ""
            // Same shape as Claude's LS row: the folder is the subject.
            let call = ToolCall(kind: .search, title: path.isEmpty ? (workingDirectory as NSString).lastPathComponent : displayPath(path))
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .search, title: call.title, detail: path, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                let text = try listFiles(path, recursive: args["recursive"]?.bool ?? false)
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: summary(text, limit: 4_000), status: .completed)))
                return text
            } catch {
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "search_text":
            let pattern = args["pattern"]?.string ?? ""
            let path = args["path"]?.string ?? ""
            let call = ToolCall(kind: .search, title: pattern.isEmpty ? "Search" : pattern, detail: path.isEmpty ? nil : displayPath(path))
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
            var call = ToolCall(kind: .edit, title: displayPath(path))
            call.edits = [FileEdit(path: displayPath(path), old: "", new: content)]
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .edit, title: "Write \(displayPath(path))", detail: nil, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                try writeFile(path: path, content: content)
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(
                    output: "Wrote \(displayPath(path)) (\(content.utf8.count) bytes).",
                    status: .completed,
                    edits: [FileEdit(path: displayPath(path), old: "", new: content)]
                )))
                return "Wrote \(displayPath(path)) (\(content.utf8.count) bytes)."
            } catch {
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(output: error.localizedDescription, status: .failed)))
                return "Error: \(error.localizedDescription)"
            }
        case "edit_file":
            let path = args["path"]?.string ?? ""
            let old = args["old_string"]?.string ?? args["old_text"]?.string ?? ""
            let new = args["new_string"]?.string ?? args["new_text"]?.string ?? ""
            var call = ToolCall(kind: .edit, title: displayPath(path))
            call.edits = [FileEdit(path: displayPath(path), old: old, new: new)]
            onEvent?(.toolStarted(id: callID, call: call))
            guard await approveIfNeeded(kind: .edit, title: "Edit \(displayPath(path))", detail: nil, toolItemID: nil) else {
                return declined(callID)
            }
            do {
                try editFile(path: path, old: old, new: new)
                onEvent?(.toolUpdated(id: callID, update: ToolUpdate(
                    output: "Edited \(displayPath(path)).",
                    status: .completed,
                    edits: [FileEdit(path: displayPath(path), old: old, new: new)]
                )))
                return "Edited \(displayPath(path))."
            } catch {
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

    private func approveIfNeeded(kind: ToolCall.Kind, title: String, detail: String?, toolItemID: String?) async -> Bool {
        if approveAllRemaining || interrupted { return interrupted ? false : approveAllRemaining }
        let needsApproval: Bool = {
            if interactionMode == .plan, (kind == .edit || kind == .command) { return true }
            switch runtimeMode {
            case .supervised: return true
            case .autoAcceptEdits, .auto: return kind == .command
            case .fullAccess: return false
            }
        }()
        guard needsApproval else { return true }
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
        return await withCheckedContinuation { continuation in
            pendingApprovals[id] = continuation
        }
    }

    // MARK: - Local execution

    private func resolveURL(_ path: String) throws -> URL {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw ProviderError.failed("A file path is required.") }
        let root = URL(fileURLWithPath: workingDirectory).standardizedFileURL
        let candidate: URL = trimmed.hasPrefix("/") || trimmed.hasPrefix("~")
            ? URL(fileURLWithPath: (trimmed as NSString).expandingTildeInPath)
            : root.appendingPathComponent(trimmed)
        let standardized = candidate.standardizedFileURL
        guard standardized.path == root.path || standardized.path.hasPrefix(root.path + "/") else {
            throw ProviderError.failed("That path is outside the project. Stay inside \(workingDirectory).")
        }
        return standardized
    }

    private func displayPath(_ path: String) -> String {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return workingDirectory }
        if trimmed.hasPrefix(workingDirectory) { return ToolTitles.relativePath(trimmed, to: workingDirectory) }
        return trimmed
    }

    private func readFile(_ path: String, startLine: Int? = nil, lineCount: Int? = nil) throws -> String {
        let url = try resolveURL(path)
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw ProviderError.failed("No such file: \(displayPath(path)).")
        }
        let data = try Data(contentsOf: url)
        guard data.count <= 1_000_000 else { throw ProviderError.failed("That file is too large to read (\(data.count) bytes).") }
        guard let text = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1) else {
            throw ProviderError.failed("That file is not readable as text.")
        }
        // A slice of the file, when asked for: what it costs to read is what it costs to
        // re-send every round after, so a big file is best read where it matters.
        if startLine != nil || lineCount != nil {
            let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
            let first = max(1, startLine ?? 1)
            guard first <= lines.count else { return "(the file has \(lines.count) lines; start_line \(first) is past the end)" }
            let count = max(1, lineCount ?? 200)
            let last = min(lines.count, first + count - 1)
            let slice = lines[(first - 1)..<last].joined(separator: "\n")
            return "(lines \(first)-\(last) of \(lines.count))\n" + slice
        }
        if text.count > 60_000 { return String(text.prefix(60_000)) + "\n…(truncated, \(text.count) chars total; read the rest with start_line and line_count)" }
        return text
    }

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
            total += message["content"]?.string?.count ?? 0
        }
        guard total > Self.condenseHistoryAt else { return }
        for index in toolIndices.dropLast(Self.condenseKeepsRecent) {
            guard total > Self.condenseHistoryTo else { break }
            guard var message = messages[index].object, let content = message["content"]?.string,
                  content.count > 1_200, !content.contains(Self.condensedMarker) else { continue }
            let kept = String(content.prefix(400))
            let condensed = kept + "\n\(Self.condensedMarker) (\(content.count) characters); call the tool again if you need it]"
            message["content"] = .string(condensed)
            messages[index] = .object(message)
            total -= content.count - condensed.count
        }
    }

    private func writeFile(path: String, content: String) throws {
        let url = try resolveURL(path)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        guard let data = content.data(using: .utf8) else { throw ProviderError.failed("Could not encode that content.") }
        try data.write(to: url, options: .atomic)
    }

    private func editFile(path: String, old: String, new: String) throws {
        guard !old.isEmpty else { throw ProviderError.failed("old_string must not be empty. Read the file first, then edit an exact block.") }
        let url = try resolveURL(path)
        let current = try String(contentsOf: url, encoding: .utf8)
        let occurrences = current.components(separatedBy: old).count - 1
        guard occurrences == 1 else {
            if occurrences == 0 { throw ProviderError.failed("That exact text was not found in \(displayPath(path)). Read the file and copy it exactly.") }
            throw ProviderError.failed("That text appears \(occurrences) times in \(displayPath(path)). Include more context so it matches once.")
        }
        try current.replacingOccurrences(of: old, with: new).write(to: url, atomically: true, encoding: .utf8)
    }

    private func listFiles(_ path: String, recursive: Bool) throws -> String {
        let base: URL = path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? URL(fileURLWithPath: workingDirectory)
            : try resolveURL(path)
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: base.path, isDirectory: &isDirectory), isDirectory.boolValue else {
            throw ProviderError.failed("No such directory: \(displayPath(path.isEmpty ? workingDirectory : path)).")
        }
        let skipped: Set<String> = [".git", "node_modules", ".build", "build", "dist", "DerivedData", "Pods", "target", ".venv", "venv"]
        var results: [String] = []
        if recursive {
            guard let enumerator = FileManager.default.enumerator(at: base, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles, .skipsPackageDescendants]) else {
                return "(empty)"
            }
            while let url = enumerator.nextObject() as? URL {
                if skipped.contains(url.lastPathComponent) { enumerator.skipDescendants(); continue }
                if (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true { continue }
                results.append(ToolTitles.relativePath(url.path, to: workingDirectory))
                if results.count >= 300 { results.append("…(truncated)"); break }
            }
        } else {
            let items = (try? FileManager.default.contentsOfDirectory(atPath: base.path)) ?? []
            for item in items.sorted() where !skipped.contains(item) && !item.hasPrefix(".DS_Store") {
                var isDir: ObjCBool = false
                let full = base.appendingPathComponent(item).path
                FileManager.default.fileExists(atPath: full, isDirectory: &isDir)
                results.append(ToolTitles.relativePath(full, to: workingDirectory) + (isDir.boolValue ? "/" : ""))
                if results.count >= 300 { results.append("…(truncated)"); break }
            }
        }
        return results.isEmpty ? "(empty)" : results.joined(separator: "\n")
    }

    private func searchText(pattern: String, path: String, regex: Bool) async -> String {
        let trimmed = pattern.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "Error: a search pattern is required." }
        let directory = URL(fileURLWithPath: workingDirectory)
        let grep = URL(fileURLWithPath: "/usr/bin/grep")
        var args = ["-R", "-n", "-I", "--exclude-dir=.git", "--exclude-dir=node_modules", "--exclude-dir=.build", "--exclude-dir=build"]
        args += regex ? ["-E", trimmed] : ["-F", trimmed]
        args.append(path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "." : path)
        guard let result = try? await Shell.run(grep, args, in: directory, environment: LoginEnvironment.current, timeout: 30) else {
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
        guard let result = try? await Shell.run(shell, ["-lc", trimmed], in: directory, environment: LoginEnvironment.current, timeout: 120) else {
            return ("The command could not start.", 1)
        }
        var output = (result.output + (result.errorOutput.isEmpty ? "" : "\n" + result.errorOutput)).trimmingCharacters(in: .whitespacesAndNewlines)
        if output.utf8.count > 24_000 { output = "…" + String(output.suffix(24_000)) }
        return (output, result.status)
    }

    // MARK: - Messages

    private func systemPrompt() -> String {
        let modeLine = switch (interactionMode, runtimeMode) {
        case (.plan, _):
            "You are in PLAN mode: research the project, then write a clear step-by-step plan. Do not edit files or run commands that change anything; reads, lists and searches are fine."
        case (.build, .supervised):
            "You are in BUILD mode with supervised permissions: use tools freely but the user approves each one in Droppy Code."
        case (.build, .autoAcceptEdits), (.build, .auto):
            "You are in BUILD mode: read and edit files freely; the user approves shell commands in Droppy Code."
        case (.build, .fullAccess):
            "You are in BUILD mode with full access: use tools freely without asking."
        }
        return """
        You are Droppy Code's Z.ai GLM coding agent, working inside \(workingDirectory) on macOS.
        \(modeLine)
        Rules:
        - Prefer the provided tools over asking the user to run things. Read files before editing them.
        - Keep file paths relative to the project root and never touch paths outside it.
        - Explain briefly what you did after tool calls; keep chat replies concise markdown.
        - If a tool result shows the user declined an action, do not retry it: ask how to proceed.
        - Today's date is \(ISO8601DateFormatter().string(from: Date())).
        \(configuration.hydra.map { "\n" + HydraPrompts.fallbackPolicy($0) } ?? "")
        """
    }

    private func userContentValue(text: String, images: [Attachment]) -> JSONValue {
        let usable = images.filter(\.isImage).prefix(4)
        guard !usable.isEmpty, ZaiAPI.isVisionModel(currentModel) else {
            if usable.isEmpty { return .string(text) }
            let names = usable.map(\.name).joined(separator: ", ")
            return .string(text + "\n\n[Attached images (model cannot see them): \(names)]")
        }
        var parts: [JSONValue] = [["type": "text", "text": .string(text)]]
        for image in usable {
            guard let data = try? Data(contentsOf: image.url) else { continue }
            let url = "data:\(image.mimeType);base64,\(data.base64EncodedString())"
            parts.append(["type": "image_url", "image_url": ["url": .string(url)]])
        }
        return .array(parts)
    }

    /// Thinking mode rejects the follow-up request with "The `reasoning_content`
    /// in the thinking mode must be passed back to the API" unless every
    /// tool-call message carries it (an empty string is accepted).
    private func assistantMessage(content: String, reasoning: String, toolCalls: [PendingToolCall]) -> JSONValue {
        let calls: [JSONValue] = toolCalls.map { tool in
            ["id": .string(tool.id), "type": "function", "function": ["name": .string(tool.name), "arguments": .string(tool.arguments)]]
        }
        return ["role": "assistant", "content": .string(content), "reasoning_content": .string(reasoning), "tool_calls": .array(calls)]
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
    /// Z.ai rejects any history where a `tool` message has no preceding
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
        guard text.count > limit else { return text }
        return String(text.prefix(limit)) + "\n…(truncated)"
    }

    private func nextID(_ prefix: String) -> String {
        counter += 1
        return "\(prefix)-\(counter)-\(UUID().uuidString.prefix(8))"
    }
}
