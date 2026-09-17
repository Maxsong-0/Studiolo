import Foundation

/// Minimal OpenAI-compatible HTTP client for api.deepseek.com.
/// Chat streaming itself lives in DeepSeekSession; this covers the
/// lightweight calls (key validation, model catalog) the registry needs.
enum DeepSeekAPI {
    static let baseURL = URL(string: "https://api.deepseek.com")!
    static let chatURL = URL(string: "https://api.deepseek.com/chat/completions")!
    static let modelsURL = URL(string: "https://api.deepseek.com/models")!
    static let balanceURL = URL(string: "https://api.deepseek.com/user/balance")!

    /// True when the key lists models successfully.
    static func validate(apiKey: String) async -> Bool {
        let key = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return false }
        var request = URLRequest(url: modelsURL, timeoutInterval: 15)
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (_, response) = try? await URLSession.shared.data(for: request),
              let http = response as? HTTPURLResponse else { return false }
        return http.statusCode == 200
    }

    static func listModels(apiKey: String) async throws -> [ModelOption] {
        let key = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return [] }
        var request = URLRequest(url: modelsURL, timeoutInterval: 15)
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200,
              let json = JSONValue.parse(data) else { return [] }
        let ids = (json["data"]?.array ?? []).compactMap { $0["id"]?.string }
        guard !ids.isEmpty else { return [] }
        return ids.compactMap(Self.option(for:)).sorted { lhs, rhs in
            (lhs.isDefault ? 0 : 1, lhs.id) < (rhs.isDefault ? 0 : 1, rhs.id)
        }
    }

    /// The prepaid balance, from the endpoint platform.deepseek.com reads for its own
    /// header. Nil when the key is missing, rejected or the API is unreachable, so the
    /// usage panel says nothing rather than showing a number that is not the real one.
    static func balance(apiKey: String) async -> ProviderCredits? {
        let key = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return nil }
        var request = URLRequest(url: balanceURL, timeoutInterval: 15)
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let json = JSONValue.parse(data) else { return nil }
        return credits(from: json)
    }

    /// `{"is_available": true, "balance_infos": [{"currency": "USD", "total_balance": "12.34",
    /// "granted_balance": "2.34", "topped_up_balance": "10.00"}]}`. One entry per currency,
    /// and the amounts arrive as strings, so they are parsed rather than decoded as numbers.
    static func credits(from json: JSONValue) -> ProviderCredits? {
        guard let info = json["balance_infos"]?.array?.first,
              let total = info["total_balance"]?.string.flatMap(Double.init) else { return nil }
        return ProviderCredits(
            total: total,
            currency: info["currency"]?.string ?? "",
            toppedUp: info["topped_up_balance"]?.string.flatMap(Double.init),
            granted: info["granted_balance"]?.string.flatMap(Double.init),
            isUsable: json["is_available"]?.bool ?? true
        )
    }

    static let deepseekEfforts = ["low", "high", "max"]

    /// Both current models have a 1M-token context (api-docs.deepseek.com, Models & Pricing).
    static let contextWindow = 1_048_576

    /// Per api-docs.deepseek.com (2026-09-10): `deepseek-flash` is DeepSeek-V4.1-Flash and
    /// `deepseek-v4-pro` is V4-Pro-0813. `deepseek-v4-flash` and `deepseek-v4-flash-vision-exp`
    /// are retired names temporarily routed to V4.1 Flash; `deepseek-chat` and
    /// `deepseek-reasoner` were discontinued on 2026-07-24. None of those are offered.
    static func option(for id: String) -> ModelOption? {
        switch id {
        case "deepseek-v4-pro":
            return ModelOption(id: id, name: "V4 Pro", detail: "Best reasoning and coding quality", efforts: deepseekEfforts, defaultEffort: "high", isDefault: true)
        case "deepseek-flash":
            return ModelOption(id: id, name: "V4.1 Flash", detail: "Newest, fast, reads images", efforts: deepseekEfforts, defaultEffort: "high")
        case "deepseek-v4-flash", "deepseek-v4-flash-vision-exp", "deepseek-chat", "deepseek-reasoner":
            return nil
        default:
            // Any other id the API ships (including future ones) still shows up
            // instead of vanishing. The models endpoint already scopes to DeepSeek.
            return ModelOption(id: id, name: id, efforts: deepseekEfforts, defaultEffort: "high")
        }
    }

    /// Only Flash accepts images; V4 Pro returns an error for them. The retired Flash
    /// names route to V4.1 Flash, so threads saved on them keep their images.
    static func isVisionModel(_ id: String?) -> Bool {
        ["deepseek-flash", "deepseek-v4-flash", "deepseek-v4-flash-vision-exp"].contains(id ?? "")
    }
}
