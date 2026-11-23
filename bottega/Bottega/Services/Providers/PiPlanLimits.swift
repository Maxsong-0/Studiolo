// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The subscription windows Pi itself reports for the providers it is signed into.
/// Pi keeps those providers' OAuth tokens in `<config>/auth.json`, so the usage
/// panel can show the same windows Pi's own TUI shows without owning the logins.
enum PiPlanLimits {
    private static let anthropicUsageURL = URL(string: "https://api.anthropic.com/api/oauth/usage")!
    private static let codexUsageURL = URL(string: "https://chatgpt.com/backend-api/wham/usage")!

    /// Every OAuth login Pi holds that still has a live token, in sorted provider order.
    static func read(environment: [String: String]) async -> PlanLimits? {
        let file = PiCLI.configDirectory(environment: environment).appendingPathComponent("auth.json")
        guard let data = try? Data(contentsOf: file),
              let json = JSONValue.parse(data),
              let object = json.object else { return nil }
        var windows: [PlanLimits.Window] = []
        var names: [String] = []
        for key in object.keys.sorted() {
            guard let entry = object[key],
                  entry["type"]?.string == "oauth",
                  let token = entry["access"]?.string, !token.isEmpty else { continue }
            // An expired token buys nothing from the usage endpoint, so it is skipped.
            if let expires = entry["expires"]?.double,
               Date(timeIntervalSince1970: expires / 1_000) < Date.now {
                continue
            }
            let limits: PlanLimits?
            if key == "anthropic" {
                limits = await readAnthropic(token: token)
            } else if key == "openai-codex" || key == "openai" {
                limits = await readCodex(token: token)
            } else {
                continue
            }
            guard let limits else { continue }
            windows.append(contentsOf: limits.windows)
            if let name = limits.planName { names.append(name) }
        }
        guard !windows.isEmpty else { return nil }
        return PlanLimits(planName: names.isEmpty ? nil : names.joined(separator: " · "), windows: windows)
    }

    /// Anthropic's OAuth usage endpoint answers the same shape Claude Code reads itself.
    private static func readAnthropic(token: String) async -> PlanLimits? {
        var request = URLRequest(url: anthropicUsageURL, timeoutInterval: 15)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("oauth-2025-04-20", forHTTPHeaderField: "anthropic-beta")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse, http.statusCode == 200,
              let json = JSONValue.parse(data),
              var limits = PlanLimitsReader.parseClaudeUsage(json, limits: json["limits"]?.array ?? [], planName: nil, titlePrefix: "Claude") else {
            return nil
        }
        limits.planName = json["subscription_type"]?.string.flatMap(PlanLimitsReader.planName).map { "Claude \($0)" } ?? "Claude"
        return limits
    }

    /// The usage payload behind ChatGPT's quota display, with its primary and secondary windows.
    private static func readCodex(token: String) async -> PlanLimits? {
        var request = URLRequest(url: codexUsageURL, timeoutInterval: 15)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        // The endpoint scopes quota to the account, which only the token's own claim names.
        if let accountID = codexAccountID(token: token), !accountID.isEmpty {
            request.setValue(accountID, forHTTPHeaderField: "ChatGPT-Account-Id")
        }
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse, http.statusCode == 200,
              let json = JSONValue.parse(data) else { return nil }
        let rateLimit = json["rate_limit"] ?? json
        let windows = [
            codexWindow(id: "codex-primary", raw: rateLimit["primary_window"]),
            codexWindow(id: "codex-secondary", raw: rateLimit["secondary_window"]),
        ].compactMap { $0 }
        guard !windows.isEmpty else { return nil }
        let name = json["plan_type"]?.string.flatMap(PlanLimitsReader.codexPlanName).map { "Codex \($0)" } ?? "Codex"
        return PlanLimits(planName: name, windows: windows)
    }

    private static func codexWindow(id: String, raw: JSONValue?) -> PlanLimits.Window? {
        guard let raw,
              let percent = raw["used_percent"]?.double,
              let windowSeconds = raw["limit_window_seconds"]?.int else { return nil }
        let resetsAt: Date? = {
            if let resetAt = raw["reset_at"]?.double {
                return Date(timeIntervalSince1970: resetAt)
            }
            if let after = raw["reset_after_seconds"]?.int {
                return Date.now.addingTimeInterval(TimeInterval(after))
            }
            return nil
        }()
        return PlanLimits.Window(
            id: id,
            title: "Codex · " + PlanLimitsReader.windowTitle(minutes: windowSeconds / 60),
            percent: percent,
            resetsAt: resetsAt
        )
    }

    /// The account id lives in the token payload's OpenAI auth claim, so no extra call is needed to find it.
    private static func codexAccountID(token: String) -> String? {
        let parts = token.split(separator: ".")
        guard parts.count >= 2 else { return nil }
        var base64 = String(parts[1]).replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        let remainder = base64.count % 4
        if remainder != 0 {
            base64 += String(repeating: "=", count: 4 - remainder)
        }
        guard let data = Data(base64Encoded: base64),
              let json = JSONValue.parse(data) else { return nil }
        return json["https://api.openai.com/auth"]?["chatgpt_account_id"]?.string
    }
}
