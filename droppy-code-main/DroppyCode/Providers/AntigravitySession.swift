import Foundation

/// Drives Google's Antigravity CLI (`agy`) in headless streaming mode.
///
/// The CLI speaks newline-delimited JSON on stdio, much like Claude's
/// stream-json: Droppy Code holds one persistent process per thread with
/// `--input-format stream-json --output-format stream-json`, writes one
/// `{"event":"user","message":{"content":"…"}}` object per turn on stdin,
/// and reads `init` / `step_update` / `result` events on stdout.
///
/// Protocol reference: https://antigravity.google/docs/cli/headless/
/// Verified live against `agy 1.2.2` (models TSV, `/usage` and `/credits`
/// headless envelopes, tool step shapes, `denied_actions`).
@MainActor
final class AntigravitySession: ProviderSession {
    var onEvent: ((ProviderEvent) -> Void)?

    private let configuration: SessionConfiguration
    private var process: StdioProcess?
    private var readTask: Task<Void, Never>?
    private var initContinuation: CheckedContinuation<String, Error>?
    private var conversationID: String?
    private var runtimeMode: RuntimeMode
    private var interactionMode: InteractionMode
    private var model: String?
    private var effort: String?
    private var turnActive = false
    private var interruptRequested = false
    private var isStopping = false
    /// Resolves the cumulative session token total into per-turn spend.
    private var spendTracker = TokenSpendTracker()
    /// Namespaces step ids per launch. `step_index` counts up within one
    /// process but a resumed conversation starts over, and a repeated id
    /// would stream a new turn's text into an old entry of the thread.
    private let launchID = String(UUID().uuidString.prefix(8))
    /// Whether any reply text streamed this turn; without it, the result's
    /// `response` is the reply.
    private var streamedText = false
    /// Files an edit step is about to change, captured when the step went
    /// active. The stream never carries the edit itself, so the row's diff
    /// is the file before against the file after.
    private var editSnapshots: [String: String] = [:]
    /// Tool steps whose start was announced, so a step that surfaces only as
    /// finished still gets its proper row rather than a bare "Tool".
    private var startedTools: Set<String> = []

    /// Reasoning efforts `agy --effort` accepts.
    static let efforts = ["low", "medium", "high"]

    init(configuration: SessionConfiguration) {
        self.configuration = configuration
        runtimeMode = configuration.runtimeMode
        interactionMode = configuration.interactionMode
        model = configuration.model
        effort = configuration.effort
    }

    var isRunning: Bool { process?.isRunning ?? false }

    private var workingDirectory: String { configuration.workingDirectory.path }

    // MARK: - Lifecycle

    func start() async throws -> String {
        guard let executable = configuration.executable else { throw ProviderError.notInstalled(configuration.provider) }
        // `--add-dir` is what makes the thread's folder the workspace. The
        // CLI does not adopt its cwd (verified live, git repo or not): without
        // the flag the agent works in `~/.gemini/antigravity-cli/scratch` and
        // spends its first steps hunting the home directory for the project.
        var arguments = [
            "--input-format", "stream-json",
            "--output-format", "stream-json",
            "--print-timeout", "30m",
            "--add-dir", workingDirectory,
        ]
        let (resolvedModel, resolvedEffort) = Self.normalize(model: model, effort: effort)
        if let resolvedModel, !resolvedModel.isEmpty { arguments += ["--model", resolvedModel] }
        if let resolvedEffort, !resolvedEffort.isEmpty { arguments += ["--effort", resolvedEffort] }
        arguments += modeArguments(runtimeMode: configuration.runtimeMode, interaction: configuration.interactionMode)
        if let resumeID = configuration.resumeID, !resumeID.isEmpty {
            arguments += ["--conversation", resumeID]
        }

        let process = StdioProcess(
            executable: executable,
            arguments: arguments,
            directory: configuration.workingDirectory,
            environment: configuration.environment
        )
        self.process = process
        isStopping = false
        try process.start()
        let messages = process.messages
        readTask = Task { [weak self] in
            for await message in messages {
                guard let self else { return }
                self.handle(message)
            }
            self?.didClose()
        }
        return try await withCheckedThrowingContinuation { continuation in
            initContinuation = continuation
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(30))
                self?.failStartAfterTimeout()
            }
        }
    }

    /// `agy --mode` only knows `accept-edits` and `plan`: the default review
    /// mode is the absence of the flag. Full access additionally approves
    /// every tool, since headless runs soft-deny unapproved actions instead
    /// of prompting.
    private func modeArguments(runtimeMode: RuntimeMode, interaction: InteractionMode) -> [String] {
        var arguments: [String] = []
        if interaction == .plan {
            arguments += ["--mode", "plan"]
        } else {
            switch runtimeMode {
            case .supervised:
                break
            case .autoAcceptEdits, .auto, .fullAccess:
                arguments += ["--mode", "accept-edits"]
            }
        }
        if runtimeMode == .fullAccess {
            arguments += ["--dangerously-skip-permissions"]
        }
        return arguments
    }

    private func failStartAfterTimeout() {
        guard let continuation = initContinuation else { return }
        initContinuation = nil
        process?.terminate()
        continuation.resume(throwing: ProviderError.failed("Antigravity did not answer. Is `agy` signed in? Run `agy` once in Terminal."))
    }

    func send(_ input: TurnInput) async throws {
        guard let process, process.isRunning else { throw ProviderError.notRunning }
        runtimeMode = input.runtimeMode
        interactionMode = input.interactionMode
        if let model = input.model, !model.isEmpty { self.model = model }
        if let effort = input.effort, !effort.isEmpty { self.effort = effort }
        // Stream-json only takes text blocks; anything else ends the session
        // with an error, so images never go on the wire (see supportsImages).
        turnActive = true
        interruptRequested = false
        streamedText = false
        onEvent?(.turnStarted(providerTurnID: nil))
        process.send(["event": "user", "message": ["content": .string(input.text)]])
    }

    func interrupt() async {
        guard turnActive else { return }
        interruptRequested = true
        // The process going is the stop, not a crash: without this `didClose`
        // would follow the interrupted turn with `.exited` and the chat would
        // read "Antigravity stopped unexpectedly" every time the user stops one.
        // `start` clears the flag again for the next launch.
        isStopping = true
        // The stream has no interrupt request: stopping the process ends the
        // turn, and the conversation persists server-side for the next turn's
        // `--conversation` resume.
        process?.terminate()
    }

    func compact() async throws {
        throw ProviderError.failed("Antigravity manages its own context.")
    }

    func resolveApproval(_ requestID: String, optionID: String) {}
    func answerQuestion(_ requestID: String, answers: [String: [String]]) {}

    func stop() {
        isStopping = true
        process?.terminate()
    }

    // MARK: - Stream

    private func handle(_ message: JSONValue) {
        switch message["event"]?.string {
        case "init":
            guard let id = message["conversation_id"]?.string, !id.isEmpty else { return }
            conversationID = id
            if let continuation = initContinuation {
                initContinuation = nil
                continuation.resume(returning: id)
            }
            onEvent?(.sessionReady(sessionID: id))
        case "step_update":
            handleStep(message["step_update"] ?? .null)
        case "result":
            handleResult(message["result"] ?? .null)
        default:
            break
        }
    }

    private func handleStep(_ step: JSONValue) {
        let index = step["step_index"]?.int ?? 0
        switch step["step_type"]?.string {
        case "agent_response":
            if let usage = Self.contextUsage(step["usage"]) { onEvent?(.usage(usage)) }
            guard let text = step["text_delta"]?.string, !text.isEmpty else { return }
            streamedText = true
            onEvent?(.messageDelta(id: "agy-\(launchID)-message-\(index)", text: text))
        case "tool":
            guard let name = step["tool_name"]?.string ?? step["tool_info"]?["name"]?.string else { return }
            let id = "agy-\(launchID)-tool-\(index)"
            let info = step["tool_info"] ?? .null
            let parameters = info["parameters"] ?? .null
            let error = info["error"]?["message"]?.string
            let state = step["state"]?.string
            if !startedTools.contains(id) {
                startedTools.insert(id)
                let call = makeToolCall(name: name, parameters: parameters)
                if call.kind == .edit, state == "ACTIVE", let path = Self.path(in: parameters) {
                    editSnapshots[id] = Self.snapshot(path) ?? ""
                }
                onEvent?(.toolStarted(id: id, call: call))
                if state == "ACTIVE" { return }
            }
            // Errors ride on the `error` field (the documented shape) as well
            // as an `ERROR` state, so either ends the call as a failure.
            if state == "ERROR" || (state == "DONE" && error != nil) {
                let message = error ?? "The tool failed."
                let status: ToolCall.Status = Self.isDenied(message) ? .declined : .failed
                onEvent?(.toolUpdated(id: id, update: ToolUpdate(output: message, status: status, edits: finishEdit(id, parameters: parameters))))
            } else if state == "DONE" {
                var update = ToolUpdate(status: .completed)
                if let output = info["output"]?.string, !output.isEmpty { update.output = output }
                update.edits = finishEdit(id, parameters: parameters)
                onEvent?(.toolUpdated(id: id, update: update))
            }
        default:
            // `user_input` acknowledgements and `checkpoint` markers carry
            // nothing Droppy Code renders.
            break
        }
    }

    /// The finished edit as a diff of the file before against after, for the
    /// row's inline diff and the turn's changed-files attribution. Nil for
    /// non-edit steps, so the update leaves the call's edits alone.
    private func finishEdit(_ id: String, parameters: JSONValue) -> [FileEdit]? {
        guard let before = editSnapshots.removeValue(forKey: id), let path = Self.path(in: parameters) else { return nil }
        let relative = ToolTitles.relativePath(path, to: workingDirectory)
        let after = Self.snapshot(path) ?? ""
        guard before != after else { return [FileEdit(path: relative)] }
        return [FileEdit(path: relative, old: before, new: after)]
    }

    /// Text files up to 1 MB; anything else (binary, huge, missing) reads as
    /// empty so a write shows as a whole-file addition.
    private static func snapshot(_ path: String) -> String? {
        let url = URL(fileURLWithPath: path)
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= 1_000_000 else { return nil }
        return try? String(contentsOf: url, encoding: .utf8)
    }

    /// The soft-deny wording headless runs use when a permission rule blocks
    /// a tool: the agent asked, the user (or a rule) said no.
    private static func isDenied(_ message: String) -> Bool {
        let lowered = message.lowercased()
        return lowered.contains("permission check failed") || lowered.contains("denied permission")
            || lowered.contains("permission denied")
    }

    /// The context in use after a model call: the step's fresh input plus
    /// what was served from cache, plus the reply. Result totals are the
    /// session's cumulative spend and say nothing about the window.
    private static func contextUsage(_ usage: JSONValue?) -> ContextUsage? {
        guard let usage, !usage.isNull else { return nil }
        let used = (usage["input_tokens"]?.int ?? 0) + (usage["cache_read_tokens"]?.int ?? 0) + (usage["output_tokens"]?.int ?? 0)
        guard used > 0 else { return nil }
        return ContextUsage(usedTokens: used, windowTokens: nil)
    }

    private func handleResult(_ result: JSONValue) {
        turnActive = false
        editSnapshots.removeAll()
        startedTools.removeAll()
        if let usage = result["usage"], !usage.isNull {
            // Totals are cumulative over the session (per the headless docs),
            // so the spend tracker resolves them into per-turn deltas. The
            // context meter is fed per step instead (see handleStep).
            let used = usage["total_tokens"]?.int ?? ((usage["input_tokens"]?.int ?? 0) + (usage["output_tokens"]?.int ?? 0))
            if used > 0 {
                let spend = spendTracker.spend(total: used)
                if spend > 0 { TokenLedger.shared.record(spend: spend) }
            }
        }
        // A turn whose reply never streamed (resumed conversations can answer
        // in the result alone) still shows its text.
        if !streamedText, let response = result["response"]?.string,
           !response.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            onEvent?(.messageCompleted(id: "agy-\(launchID)-result-\(UUID().uuidString.prefix(8))", text: response))
        }
        streamedText = false
        let denied = (result["denied_actions"]?.array ?? []).compactMap { $0["display_name"]?.string ?? $0["action"]?.string }
        if !denied.isEmpty {
            let names = denied.joined(separator: ", ")
            onEvent?(.notice(Notice(
                level: .warning,
                message: "Antigravity held back \(names) without prompting. Use Full access or add an allow rule in the CLI's settings."
            )))
        }
        if let id = result["conversation_id"]?.string, !id.isEmpty {
            conversationID = id
            onEvent?(.sessionReady(sessionID: id))
        }
        if interruptRequested {
            onEvent?(.turnCompleted(status: .interrupted, error: nil))
        } else {
            switch result["status"]?.string {
            case "WAITING" where !denied.isEmpty:
                // Stopped for a permission nobody could grant headless, with actions held
                // back: reported as a failure so a head's lead hears it was starved of
                // permissions rather than reading a clean completion.
                onEvent?(.turnCompleted(status: .failed, error: "Antigravity held back \(denied.joined(separator: ", ")) waiting for a permission it could not ask for. Use Full access for this chat."))
            case "SUCCESS", "WAITING":
                // WAITING: the agent stopped to ask something it cannot ask
                // headless; the turn is over and the reply says what it needs.
                onEvent?(.turnCompleted(status: .completed, error: nil))
            case "CANCELED", "INTERRUPTED":
                onEvent?(.turnCompleted(status: .interrupted, error: nil))
            default:
                let error = result["error"]?.string
                onEvent?(.turnCompleted(status: .failed, error: (error?.isEmpty == false ? error : "Antigravity stopped before finishing.")))
            }
        }
        interruptRequested = false
    }

    private func didClose() {
        if let continuation = initContinuation {
            initContinuation = nil
            let tail = process?.errorTail ?? ""
            continuation.resume(throwing: ProviderError.failed(authHint(in: tail) ?? (TextCleanup.lastLines(tail) ?? "Antigravity exited.")))
        }
        if turnActive {
            turnActive = false
            if interruptRequested {
                onEvent?(.turnCompleted(status: .interrupted, error: nil))
            } else {
                let tail = process?.errorTail ?? ""
                onEvent?(.turnCompleted(status: .failed, error: authHint(in: tail) ?? TextCleanup.lastLines(tail) ?? "Antigravity exited."))
            }
            interruptRequested = false
        }
        guard !isStopping else { return }
        onEvent?(.exited(error: TextCleanup.lastLines(process?.errorTail ?? "")))
    }

    private func authHint(in text: String) -> String? {
        let lowered = text.lowercased()
        guard lowered.contains("authentication required") || lowered.contains("not signed in")
            || lowered.contains("unauthenticated") || (lowered.contains("login") && lowered.contains("agy")) else { return nil }
        return "Antigravity is not signed in. Run `agy` once in Terminal to sign in."
    }

    // MARK: - Model and Effort Normalization

    struct ParsedModelRow {
        let baseSlug: String
        let baseName: String
        let effort: String?
    }

    static func parseModelRow(slug: String, name: String) -> ParsedModelRow {
        for effort in ["high", "medium", "low"] {
            if slug.hasSuffix("-\(effort)") {
                let baseSlug = String(slug.dropLast(effort.count + 1))
                var baseName = name
                let suffix = " (\(effort.capitalized))"
                if baseName.hasSuffix(suffix) {
                    baseName = String(baseName.dropLast(suffix.count))
                }
                return ParsedModelRow(baseSlug: baseSlug, baseName: baseName, effort: effort)
            }
        }
        return ParsedModelRow(baseSlug: slug, baseName: name, effort: nil)
    }

    /// Normalizes model and effort flags for `agy`.
    /// Strips any legacy effort suffixes from model slugs, resolves the effective effort level,
    /// and ensures --effort is only passed for models that support it.
    static func normalize(model: String?, effort: String?) -> (model: String?, effort: String?) {
        guard let model, !model.isEmpty else { return (nil, nil) }
        let parsed = parseModelRow(slug: model, name: "")
        let baseSlug = parsed.baseSlug

        // Models that do not take --effort (such as Claude models hosted on Antigravity)
        if baseSlug.contains("claude") {
            return (baseSlug, nil)
        }

        var resolvedEffort = effort
        if resolvedEffort == nil || resolvedEffort?.isEmpty == true {
            resolvedEffort = parsed.effort
        }
        if resolvedEffort == nil || resolvedEffort?.isEmpty == true {
            resolvedEffort = baseSlug == "gpt-oss-120b" ? "medium" : "high"
        }

        // Validate effort against model constraints
        if baseSlug == "gemini-3.1-pro" && resolvedEffort == "medium" {
            resolvedEffort = "high"
        } else if baseSlug == "gpt-oss-120b" {
            resolvedEffort = "medium"
        }

        return (baseSlug, resolvedEffort)
    }

    // MARK: - Catalog

    /// `agy models` prints `slug<TAB>Name` rows on stdout (progress goes to
    /// stderr). Rows with effort variants (-high, -medium, -low) are collapsed
    /// into a single base model entry whose supported efforts are toggled via
    /// the reasoning effort slider.
    static func listModels(executable: URL, environment: [String: String]) async throws -> [ModelOption] {
        async let modelsFetch: ShellResult = try Shell.run(executable, ["models"], environment: environment, timeout: 30)
        async let defaultFetch: ShellResult = try Shell.run(executable, ["-p", "/model", "--output-format", "json"], environment: environment, timeout: 30)
        let models = try? await modelsFetch
        let current = try? await defaultFetch
        var currentID: String?
        if let stdout = current?.stdout, let json = JSONValue.parse(stdout) {
            currentID = json["command"]?["data"]?["id"]?.string
        }
        guard let output = models?.stdout, let text = String(data: output, encoding: .utf8) else {
            throw ShellError("`agy models` produced no output.")
        }

        var grouped: [String: (name: String, efforts: [String], isDefault: Bool)] = [:]
        var order: [String] = []

        for line in TextCleanup.stripANSI(text).components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { continue }
            let parts = trimmed.split(separator: "\t", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            guard let slug = parts.first, Self.isSlug(slug) else { continue }
            let name = parts.count > 1 && !parts[1].isEmpty ? parts[1] : slug
            let parsed = parseModelRow(slug: slug, name: name)

            let isCurrent = slug == currentID || parsed.baseSlug == currentID || (currentID?.hasPrefix(parsed.baseSlug) == true)

            if grouped[parsed.baseSlug] == nil {
                order.append(parsed.baseSlug)
                grouped[parsed.baseSlug] = (
                    name: parsed.baseName,
                    efforts: parsed.effort.map { [$0] } ?? [],
                    isDefault: isCurrent
                )
            } else {
                if let eff = parsed.effort, !grouped[parsed.baseSlug]!.efforts.contains(eff) {
                    grouped[parsed.baseSlug]!.efforts.append(eff)
                }
                if isCurrent {
                    grouped[parsed.baseSlug]!.isDefault = true
                }
            }
        }

        var list: [ModelOption] = []
        for baseSlug in order {
            guard let data = grouped[baseSlug] else { continue }
            let sortedEfforts = ["low", "medium", "high"].filter { data.efforts.contains($0) }
            let defaultEffort = sortedEfforts.contains("high") ? "high" : sortedEfforts.last
            list.append(ModelOption(
                id: baseSlug,
                name: data.name,
                efforts: sortedEfforts,
                defaultEffort: defaultEffort,
                isDefault: data.isDefault
            ))
        }

        guard !list.isEmpty else { throw ShellError("`agy models` listed no models.") }
        if !list.contains(where: \.isDefault), let first = list.first {
            list[list.startIndex] = ModelOption(
                id: first.id, name: first.name, detail: first.detail,
                efforts: first.efforts, defaultEffort: first.defaultEffort, isDefault: true
            )
        }
        return list
    }

    private static func isSlug(_ text: String) -> Bool {
        guard let first = text.first, first.isLetter || first.isNumber else { return false }
        return text.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" || $0 == "." }
            && !text.contains(" ")
    }

    /// Sign-in state without spending tokens: `agy models` needs the cached
    /// Google session (or `GEMINI_API_KEY`) and fails loudly without one.
    static func authStatus(executable: URL, environment: [String: String]) async -> ProviderStatus.Auth {
        guard let result = try? await Shell.run(executable, ["models"], environment: environment, timeout: 20) else {
            return .unknown
        }
        let text = TextCleanup.stripANSI(result.output + result.errorOutput)
        let rows = text.components(separatedBy: .newlines).filter { isSlug($0.split(separator: "\t").first.map(String.init) ?? "") }
        if result.succeeded, !rows.isEmpty { return .signedIn(nil) }
        let lowered = text.lowercased()
        if lowered.contains("authentication required") || lowered.contains("not signed in")
            || lowered.contains("unauthenticated") || lowered.contains("sign in") || lowered.contains("login") {
            return .signedOut
        }
        return result.succeeded ? .signedIn(nil) : .unknown
    }

    // MARK: - Plan limits

    /// Model quotas (`/usage`: weekly + 5-hour buckets per model group) and
    /// the AI credit balance (`/credits`). Both are zero-token headless
    /// calls, so reading limits never spends quota itself.
    static func readPlanLimits(executable: URL, environment: [String: String]) async throws -> PlanLimits? {
        async let usageFetch: ShellResult = try Shell.run(executable, ["-p", "/usage", "--output-format", "json"], environment: environment, timeout: 30)
        async let creditsFetch: ShellResult = try Shell.run(executable, ["-p", "/credits", "--output-format", "json"], environment: environment, timeout: 30)
        let usage = try? await usageFetch
        let credits = try? await creditsFetch
        var windows: [PlanLimits.Window] = []
        if let usage, let data = String(data: usage.stdout, encoding: .utf8), let json = JSONValue.parse(data) {
            windows += parseUsage(json["command"]?["data"], fallback: json["response"]?.string)
        }
        if let credits, let data = String(data: credits.stdout, encoding: .utf8), let json = JSONValue.parse(data),
           let window = parseCredits(json["command"]?["data"], fallback: json["response"]?.string) {
            windows.append(window)
        }
        guard !windows.isEmpty else { return nil }
        return PlanLimits(planName: nil, windows: windows)
    }

    /// The structured envelope: groups of models sharing a weekly and a
    /// 5-hour bucket. `remaining_fraction` is what is left, so the usage
    /// panel's percent is the complement.
    private static func parseUsage(_ data: JSONValue?, fallback response: String?) -> [PlanLimits.Window] {
        var windows: [PlanLimits.Window] = []
        for group in data?["groups"]?.array ?? [] {
            let groupName = group["name"]?.string ?? "Models"
            for bucket in group["buckets"]?.array ?? [] {
                guard let remaining = bucket["remaining_fraction"]?.double else { continue }
                windows.append(PlanLimits.Window(
                    id: bucket["id"]?.string ?? "\(groupName)-\(windows.count)",
                    title: "\(groupName) · \(bucket["name"]?.string ?? "Limit")",
                    percent: max(0, min(100, (1 - remaining) * 100)),
                    resetsAt: bucket["reset_time"]?.string.flatMap(ISO8601DateFormatter().date(from:))
                ))
            }
        }
        if windows.isEmpty, let response { windows += parseUsageText(response) }
        return windows
    }

    /// Older CLIs without the structured envelope: `Group<TAB>Bucket<TAB>95%<TAB><ISO date>`
    /// rows, where the percent is what remains.
    private static func parseUsageText(_ response: String) -> [PlanLimits.Window] {
        var windows: [PlanLimits.Window] = []
        for line in response.components(separatedBy: .newlines) {
            let parts = line.split(separator: "\t").map { $0.trimmingCharacters(in: .whitespaces) }
            guard parts.count >= 3 else { continue }
            let percentText = parts[2].replacingOccurrences(of: "%", with: "")
            guard let remaining = Double(percentText) else { continue }
            let reset = parts.count >= 4 ? ISO8601DateFormatter().date(from: parts[3]) : nil
            windows.append(PlanLimits.Window(
                id: "usage-\(windows.count)",
                title: "\(parts[0]) · \(parts[1])",
                percent: max(0, min(100, 100 - remaining)),
                resetsAt: reset
            ))
        }
        return windows
    }

    private static func parseCredits(_ data: JSONValue?, fallback response: String?) -> PlanLimits.Window? {
        if let remaining = data?["remaining_credits"]?.int {
            return PlanLimits.Window(
                id: "ai-credits",
                title: "AI credits · \(remaining) remaining",
                percent: remaining > 0 ? 0 : 100,
                resetsAt: nil
            )
        }
        guard let response else { return nil }
        for line in response.components(separatedBy: .newlines) {
            let parts = line.split(separator: "\t").map { $0.trimmingCharacters(in: .whitespaces) }
            guard parts.count >= 2, parts[0].localizedCaseInsensitiveContains("credit"),
                  let remaining = Int(parts[1]) else { continue }
            return PlanLimits.Window(
                id: "ai-credits",
                title: "AI credits · \(remaining) remaining",
                percent: remaining > 0 ? 0 : 100,
                resetsAt: nil
            )
        }
        return nil
    }

    // MARK: - Tool mapping

    /// Same row shapes as Claude's: the subject is the command, the path, the
    /// pattern or the URL, and the detail is only what qualifies it (a search
    /// folder, an MCP tool's input). The stream sends a display subset of the
    /// parameters (`AbsolutePath`, `TargetFile`, `SearchDirectory`, ...), never
    /// the edit content.
    private func makeToolCall(name: String, parameters: JSONValue) -> ToolCall {
        let kind = Self.kind(of: name)
        let path = Self.path(in: parameters).map { ToolTitles.relativePath($0, to: workingDirectory) }
        switch kind {
        case .command:
            let command = Self.string(in: parameters, keys: ["CommandLine", "command", "Command"]).map(ToolTitles.unwrapShell)
            return ToolCall(kind: .command, title: command ?? Self.humanize(name))
        case .read:
            var call = ToolCall(kind: .read, title: path ?? Self.string(in: parameters, keys: ["Url", "URL", "url"]) ?? Self.humanize(name))
            if path != nil, let range = Self.lineRange(in: parameters) { call.detail = range }
            return call
        case .edit:
            var call = ToolCall(kind: .edit, title: path ?? Self.humanize(name))
            if let path { call.edits = [FileEdit(path: path)] }
            return call
        case .search:
            let pattern = Self.string(in: parameters, keys: ["Query", "query", "Pattern", "pattern"])
            let folder = Self.string(in: parameters, keys: ["SearchPath", "SearchDirectory", "DirectoryPath", "Path", "path"])
                .map { ToolTitles.relativePath($0, to: workingDirectory) }
            if let pattern {
                return ToolCall(kind: .search, title: pattern, detail: folder)
            }
            return ToolCall(kind: .search, title: folder ?? Self.humanize(name))
        case .web:
            let subject = Self.string(in: parameters, keys: ["Url", "URL", "url", "query", "Query"])
            return ToolCall(kind: .web, title: subject ?? Self.humanize(name))
        case .agent:
            let task = Self.string(in: parameters, keys: ["Task", "task", "Prompt", "prompt", "Description", "description"])
            return ToolCall(kind: .agent, title: task.map { TextCleanup.singleLine($0, limit: 96) } ?? Self.humanize(name))
        case .mcp:
            let server = Self.string(in: parameters, keys: ["ServerName", "server_name", "Server", "server"]) ?? "MCP"
            let tool = Self.string(in: parameters, keys: ["ToolName", "tool_name", "Tool", "tool"]) ?? "tool"
            let input = parameters["Arguments"] ?? parameters["arguments"] ?? parameters["Input"] ?? .null
            return ToolCall(kind: .mcp, title: "\(server) · \(tool)", detail: input.isNull ? nil : input.compactString)
        case .other:
            var call = ToolCall(kind: .other, title: Self.humanize(name))
            let detail = parameters.isNull ? nil : parameters.compactString
            if let detail, !detail.isEmpty, detail != "{}", detail.count <= 300 { call.detail = detail }
            return call
        }
    }

    private static func kind(of name: String) -> ToolCall.Kind {
        switch name {
        case "run_command", "send_command_input", "command_status", "wait", "wait_5_seconds", "schedule":
            .command
        case "view_file", "view_file_outline", "view_code_item", "view_content_chunk", "read_resource",
             "read_browser_page", "capture_browser_screenshot", "capture_browser_console_logs":
            .read
        case "write_to_file", "replace_file_content", "multi_replace_file_content", "sed_file", "notebook_edit", "notebook_execution":
            .edit
        case "list_dir", "grep_search", "find_by_name", "list_resources", "list_browser_pages":
            .search
        case "search_web", "read_url_content", "open_browser_url", "browser_click_element", "browser_drag_pixel_to_pixel",
             "browser_get_dom", "browser_get_network_request", "browser_input", "browser_list_network_requests",
             "browser_mouse_down", "browser_mouse_up", "browser_move_mouse", "browser_press_key", "browser_refresh_page",
             "browser_resize_window", "browser_scroll", "browser_scroll_dom", "browser_select_option", "click_browser_pixel",
             "execute_browser_javascript":
            .web
        case "invoke_subagent", "browser_subagent", "define_subagent", "manage_subagents":
            .agent
        case "call_mcp_tool":
            .mcp
        default:
            .other
        }
    }

    private static func path(in parameters: JSONValue) -> String? {
        string(in: parameters, keys: [
            "AbsolutePath", "TargetFile", "NotebookPath", "FilePath", "File", "file", "path", "Path",
            "DirectoryPath", "directory", "dir",
        ])
    }

    private static func string(in parameters: JSONValue, keys: [String]) -> String? {
        for key in keys {
            if let value = parameters[key]?.string, !value.isEmpty { return value }
        }
        return nil
    }

    /// "L12–40" when a read names a line window, so partial reads read as such.
    private static func lineRange(in parameters: JSONValue) -> String? {
        guard let start = parameters["StartLine"]?.int else { return nil }
        if let end = parameters["EndLine"]?.int, end > start { return "L\(start)–\(end)" }
        return "L\(start)"
    }

    private static func humanize(_ name: String) -> String {
        let words = name.split(separator: "_").map(String.init)
        guard let first = words.first else { return name }
        return ([first.capitalized] + words.dropFirst()).joined(separator: " ")
    }
}
