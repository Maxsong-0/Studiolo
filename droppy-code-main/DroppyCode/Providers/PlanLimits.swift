import Foundation

/// A provider's subscription limits: each rolling window, how much of it is used and when it resets.
struct PlanLimits: Sendable, Equatable {
    struct Window: Sendable, Equatable, Identifiable {
        var id: String
        var title: String
        var percent: Double
        var resetsAt: Date?
        var detail: String? = nil
    }

    var planName: String?
    var windows: [Window]
    var resetCredits: ResetCredits? = nil

    /// One reset Codex has banked for the account. `title` and `detail` are the app-server's own words when it gives any.
    struct ResetCredit: Sendable, Equatable, Identifiable {
        var id: String
        var title: String?
        var detail: String?
        var expiresAt: Date?
    }

    /// Codex only: resets the account has banked, each spendable to clear the active windows. `credits` can be shorter than `availableCount` when the app-server reported only a total.
    struct ResetCredits: Sendable, Equatable {
        var availableCount: Int
        var credits: [ResetCredit]
    }

    /// What the app-server answered when a reset was spent.
    enum ResetOutcome: String, Sendable, Equatable {
        case reset, nothingToReset, noCredit, alreadyRedeemed

        var message: String {
            switch self {
            case .reset: "Usage was reset."
            case .nothingToReset: "There was no active usage to reset."
            case .noCredit: "No banked resets are left."
            case .alreadyRedeemed: "That reset was already used."
            }
        }
    }
}

/// Why a banked reset could not be spent.
enum ResetCreditError: LocalizedError {
    case codexUnavailable
    case unknownOutcome
    case rejected(String)

    var errorDescription: String? {
        switch self {
        case .codexUnavailable: "Codex CLI not found."
        case .unknownOutcome: "Codex returned an unknown reset result."
        case .rejected(let message): message
        }
    }
}

@MainActor
enum PlanLimitsReader {
    /// Codex, Claude, Antigravity, Copilot, Command Code and Z.ai report plan limits. Cursor,
    /// OpenCode, Grok, DeepSeek, Meta and Devin expose none.
    static func exposesLimits(_ provider: ProviderKind) -> Bool {
        provider == .codex || provider == .claude || provider == .antigravity || provider == .copilot || provider == .commandcode || provider == .zai
    }

    static func read(_ provider: ProviderKind, executable: URL, environment: [String: String]) async -> PlanLimits? {
        switch provider {
        case .codex: try? await CodexSession.readPlanLimits(executable: executable, environment: environment)
        case .claude: await readClaude(executable: executable, environment: environment)
        case .antigravity: try? await AntigravitySession.readPlanLimits(executable: executable, environment: environment)
        case .copilot: try? await CopilotSession.readPlanLimits(executable: executable, environment: environment)
        case .commandcode: await CommandCodeAPI.planLimits(environment: environment)
        case .cursor, .opencode, .grok, .deepseek, .meta, .zai, .devin, .pi: nil
        }
    }

    /// API-key subscriptions read their windows from the API instead of a CLI.
    static func read(_ provider: ProviderKind, apiKey: String) async -> PlanLimits? {
        switch provider {
        case .zai: await ZaiAPI.planLimits(apiKey: apiKey)
        default: nil
        }
    }

    nonisolated static func windowTitle(minutes: Int?) -> String {
        guard let minutes else { return "Usage limit" }
        switch minutes {
        case 300: return "5-hour limit"
        case 1_440: return "Daily"
        case 10_080: return "Weekly"
        case 40_320...44_640: return "Monthly"
        default:
            return minutes % 1_440 == 0 ? "\(minutes / 1_440)-day limit" : "\(max(1, minutes / 60))-hour limit"
        }
    }

    nonisolated static func planName(_ raw: String?) -> String? {
        guard let raw, !raw.isEmpty else { return nil }
        return raw.prefix(1).uppercased() + raw.dropFirst()
    }

    // MARK: - Claude

    /// Claude Code's own OAuth usage endpoint first, the way it reads limits itself; the CLI's
    /// `get_usage` control request when no stored login can be read.
    private static func readClaude(executable: URL, environment: [String: String]) async -> PlanLimits? {
        if let limits = await readClaudeUsageEndpoint(executable: executable, environment: environment) {
            return limits
        }
        return await readClaudeFromCLI(executable: executable, environment: environment)
    }

    private struct ClaudeLogin {
        let accessToken: String
        let expiresAt: Date?
        let subscriptionType: String?
        let rateLimitTier: String?

        /// "Max (20x)" from `max` and `default_claude_max_20x`.
        var planName: String? {
            guard var name = PlanLimitsReader.planName(subscriptionType) else { return nil }
            if let tier = rateLimitTier?.range(of: #"\d+x"#, options: .regularExpression) {
                name += " (\(rateLimitTier![tier]))"
            }
            return name
        }
    }

    private static let usageURL = URL(string: "https://api.anthropic.com/api/oauth/usage")!
    private static let keychainService = "Claude Code-credentials"

    private static func readClaudeUsageEndpoint(executable: URL, environment: [String: String]) async -> PlanLimits? {
        guard var login = await claudeLogin(environment: environment) else { return nil }
        // A token about to expire is refreshed by the CLI, which owns the single-use refresh token.
        if let expiresAt = login.expiresAt, expiresAt < Date.now.addingTimeInterval(300) {
            await refreshClaudeLogin(executable: executable, environment: environment)
            guard let refreshed = await claudeLogin(environment: environment) else { return nil }
            login = refreshed
        }
        var response = await fetchClaudeUsage(token: login.accessToken)
        if response?.status == 401 || response?.status == 403 {
            await refreshClaudeLogin(executable: executable, environment: environment)
            guard let refreshed = await claudeLogin(environment: environment), refreshed.accessToken != login.accessToken else {
                return nil
            }
            login = refreshed
            response = await fetchClaudeUsage(token: login.accessToken)
        }
        guard let response, response.status == 200, let json = JSONValue.parse(response.data) else { return nil }
        return parseClaudeUsage(json, limits: json["limits"]?.array ?? [], planName: login.planName)
    }

    /// The stored Claude Code login: `.credentials.json`, or the macOS Keychain item the CLI writes.
    private static func claudeLogin(environment: [String: String]) async -> ClaudeLogin? {
        var directories: [String] = []
        if let configured = environment["CLAUDE_CONFIG_DIR"], !configured.isEmpty {
            directories.append(configured)
        }
        directories.append((LoginEnvironment.homeDirectory as NSString).appendingPathComponent(".claude"))
        for directory in directories {
            let file = URL(fileURLWithPath: directory).appendingPathComponent(".credentials.json")
            if let data = try? Data(contentsOf: file), let login = parseLogin(data) {
                return login
            }
        }

        let security = URL(fileURLWithPath: "/usr/bin/security")
        var attempts: [[String]] = []
        if let user = environment["USER"] ?? environment["LOGNAME"], !user.isEmpty {
            attempts.append(["find-generic-password", "-s", keychainService, "-a", user, "-w"])
        }
        attempts.append(["find-generic-password", "-s", keychainService, "-w"])
        for arguments in attempts {
            guard let result = try? await Shell.run(security, arguments, environment: environment, timeout: 5),
                  result.succeeded else { continue }
            let text = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
            let data = text.hasPrefix("{") ? Data(text.utf8) : (hexDecoded(text) ?? Data(text.utf8))
            if let login = parseLogin(data) {
                return login
            }
        }
        return nil
    }

    private static func parseLogin(_ data: Data) -> ClaudeLogin? {
        guard let oauth = JSONValue.parse(data)?["claudeAiOauth"],
              let token = oauth["accessToken"]?.string, !token.isEmpty else { return nil }
        return ClaudeLogin(
            accessToken: token,
            expiresAt: oauth["expiresAt"]?.double.map { Date(timeIntervalSince1970: $0 / 1_000) },
            subscriptionType: oauth["subscriptionType"]?.string,
            rateLimitTier: oauth["rateLimitTier"]?.string
        )
    }

    private static func hexDecoded(_ text: String) -> Data? {
        guard text.count.isMultiple(of: 2), text.allSatisfy(\.isHexDigit) else { return nil }
        var bytes: [UInt8] = []
        bytes.reserveCapacity(text.count / 2)
        var index = text.startIndex
        while index < text.endIndex {
            let next = text.index(index, offsetBy: 2)
            guard let byte = UInt8(text[index..<next], radix: 16) else { return nil }
            bytes.append(byte)
            index = next
        }
        return Data(bytes)
    }

    private static func refreshClaudeLogin(executable: URL, environment: [String: String]) async {
        _ = try? await Shell.run(executable, ["auth", "status"], environment: environment, timeout: 20)
    }

    private static func fetchClaudeUsage(token: String) async -> (status: Int, data: Data)? {
        var request = URLRequest(url: usageURL, timeoutInterval: 15)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("oauth-2025-04-20", forHTTPHeaderField: "anthropic-beta")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse else { return nil }
        return (http.statusCode, data)
    }

    /// Claude reports limits through the `get_usage` control request, so a short-lived session asks for them.
    private static func readClaudeFromCLI(executable: URL, environment: [String: String]) async -> PlanLimits? {
        let process = StdioProcess(
            executable: executable,
            arguments: ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
            directory: FileManager.default.temporaryDirectory,
            environment: environment
        )
        do {
            try process.start()
        } catch {
            return nil
        }
        // Ending the process finishes the message stream, which is what bounds a request that never answers.
        let watchdog = Task {
            try? await Task.sleep(for: .seconds(20))
            process.terminate()
        }
        defer {
            watchdog.cancel()
            process.terminate()
        }
        process.send(["type": "control_request", "request_id": "droppy-code-init", "request": ["subtype": "initialize", "hooks": .null]])
        for await message in process.messages {
            guard message["type"]?.string == "control_response" else { continue }
            let response = message["response"] ?? .null
            switch response["request_id"]?.string {
            case "droppy-code-init":
                process.send(["type": "control_request", "request_id": "droppy-code-usage", "request": ["subtype": "get_usage"]])
            case "droppy-code-usage":
                let usage = response["response"] ?? .null
                let rateLimits = usage["rate_limits"] ?? .null
                return parseClaudeUsage(rateLimits, limits: rateLimits["limits"]?.array ?? [], planName: planName(usage["subscription_type"]?.string))
            default:
                continue
            }
        }
        return nil
    }

    /// The `limits` rows (session, weekly, weekly per model), falling back to the top-level
    /// `five_hour` and `seven_day` windows, plus extra usage when it is turned on.
    private static func parseClaudeUsage(_ root: JSONValue, limits: [JSONValue], planName: String?) -> PlanLimits? {
        var windows = limits.enumerated().compactMap { index, limit -> PlanLimits.Window? in
            guard let percent = limit["percent"]?.double else { return nil }
            let kind = limit["kind"]?.string ?? ""
            let group = self.planName(limit["group"]?.string) ?? "Usage"
            let scope = limit["scope"]?["model"]?["display_name"]?.string
            let title = switch kind {
            case "session": "5-hour limit"
            case "weekly_all": "Weekly · all models"
            default: scope.map { "\(group) · \($0)" } ?? group
            }
            return PlanLimits.Window(
                id: "\(kind)-\(index)",
                title: title,
                percent: percent,
                resetsAt: limit["resets_at"]?.string.flatMap(parseDate)
            )
        }
        if windows.isEmpty {
            for (key, title) in [("five_hour", "5-hour limit"), ("seven_day", "Weekly · all models")] {
                guard let window = root[key], let percent = window["utilization"]?.double else { continue }
                windows.append(PlanLimits.Window(id: key, title: title, percent: percent, resetsAt: window["resets_at"]?.string.flatMap(parseDate)))
            }
        }
        if let extra = root["extra_usage"], extra["is_enabled"]?.bool == true, let percent = extra["utilization"]?.double {
            windows.append(PlanLimits.Window(id: "extra-usage", title: "Extra usage", percent: percent, resetsAt: nil))
        }
        guard !windows.isEmpty else { return nil }
        return PlanLimits(planName: planName, windows: windows)
    }

    /// Claude writes microsecond timestamps, which the ISO 8601 parser does not take, so the fraction is dropped.
    private static func parseDate(_ text: String) -> Date? {
        let trimmed = text.replacingOccurrences(of: #"\.\d+"#, with: "", options: .regularExpression)
        return ISO8601DateFormatter().date(from: trimmed)
    }
}
