import Foundation

/// Minimal OpenAI-compatible HTTP client for api.z.ai (GLM Coding Plan).
/// Chat streaming itself lives in ZaiSession; this covers the
/// lightweight calls (key validation, model catalog, plan limits) the registry needs.
enum ZaiAPI {
    static let baseURL = URL(string: "https://api.z.ai/api/coding/paas/v4")!
    static let chatURL = baseURL.appendingPathComponent("chat/completions")
    static let modelsURL = baseURL.appendingPathComponent("models")
    static let quotaURL = URL(string: "https://api.z.ai/api/monitor/usage/quota/limit")!
    static let resetListURL = URL(string: "https://api.z.ai/api/biz/customer-package-reset/list?targetType=PERSONAL")!
    static let resetUseURL = URL(string: "https://api.z.ai/api/biz/customer-package-reset/use")!
    static let subscriptionURL = URL(string: "https://z.ai/manage-apikey/subscription")!

    static let zaiEfforts = ["low", "high", "max"]

    static let contextWindow = 1_000_000
    static let maxOutputTokens = 128_000

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

    /// Only the two 5.3 Coding Plan models are offered. Older GLM ids are routed
    /// to 5.3 by Z.ai, so they are not offered.
    static func option(for id: String) -> ModelOption? {
        switch id {
        case "glm-5.3":
            return ModelOption(id: id, name: "GLM-5.3", detail: "Best reasoning and coding · 1M context", efforts: zaiEfforts, defaultEffort: "max", isDefault: true)
        case "glm-5.3-flash":
            return ModelOption(id: id, name: "GLM-5.3 Flash", detail: "Fast, reads images · 1M context", efforts: zaiEfforts, defaultEffort: "max")
        default:
            guard id.hasPrefix("glm-5.3") else { return nil }
            return ModelOption(id: id, name: id, efforts: zaiEfforts, defaultEffort: "max")
        }
    }

    /// Only Flash accepts images; the full model returns an error for them.
    static func isVisionModel(_ id: String?) -> Bool {
        guard let id else { return false }
        return id.contains("flash") || id.hasSuffix("v")
    }

    /// The Coding Plan's rolling windows from the monitor quota endpoint. Nil when the
    /// key is missing, rejected or the API is unreachable.
    static func planLimits(apiKey: String) async -> PlanLimits? {
        let key = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else { return nil }
        var request = URLRequest(url: quotaURL, timeoutInterval: 15)
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let json = JSONValue.parse(data) else { return nil }
        guard json["code"]?.int == 200 || json["success"]?.bool == true else { return nil }
        guard var limits = limits(from: json) else { return nil }
        limits.resetCredits = await resetCredits(apiKey: key)
        return limits
    }

    static func limits(from json: JSONValue) -> PlanLimits? {
        let planName = PlanLimitsReader.planName(json["data"]?["level"]?.string).map { "Coding Plan \($0)" }
        let entries = json["data"]?["limits"]?.array ?? []
        var windows: [PlanLimits.Window] = []
        var creditIndex = 0
        var toolsIndex = 0
        for entry in entries {
            let type = entry["type"]?.string ?? ""
            let usage = entry["usage"]?.double ?? 0
            let current = entry["currentValue"]?.double ?? 0
            var percent: Double
            if let reported = entry["percentage"]?.double {
                percent = reported
            } else if usage > 0 {
                percent = current / usage * 100
            } else {
                percent = 0
            }
            percent = min(max(percent, 0), 100)
            let resetsAt = entry["nextResetTime"]?.double.map { Date(timeIntervalSince1970: $0 / 1_000) }
            switch type {
            case "CREDIT_LIMIT", "TOKENS_LIMIT":
                let unit = entry["unit"]?.int
                let number = entry["number"]?.double ?? 0
                let minutes: Int? = switch unit {
                case 3: number > 0 ? Int(number * 60) : nil
                case 4: number > 0 ? Int(number * 1_440) : nil
                case 6: number > 0 ? Int(number * 10_080) : nil
                case 5: number > 0 ? Int(number * 43_200) : nil
                default: nil
                }
                let title: String
                if let minutes {
                    title = PlanLimitsReader.windowTitle(minutes: minutes)
                } else if creditIndex == 0 {
                    title = "5-hour limit"
                } else {
                    title = "Weekly"
                }
                windows.append(PlanLimits.Window(
                    id: "credit-\(creditIndex)",
                    title: title,
                    percent: percent,
                    resetsAt: resetsAt,
                    detail: "\(formatCount(current)) of \(formatCount(usage)) credits"
                ))
                creditIndex += 1
            case "TIME_LIMIT":
                windows.append(PlanLimits.Window(
                    id: "tools-\(toolsIndex)",
                    title: "Monthly · MCP tools",
                    percent: percent,
                    resetsAt: resetsAt,
                    detail: "\(formatCount(current)) of \(formatCount(usage)) calls"
                ))
                toolsIndex += 1
            default:
                continue
            }
        }
        guard !windows.isEmpty else { return nil }
        return PlanLimits(planName: planName, windows: windows)
    }

    /// The reset cards the account holds, one row per card: the 5-hour ones first, then
    /// the weekly ones. Nil when the list cannot be read, so the section stays hidden.
    static func resetCredits(apiKey: String) async -> PlanLimits.ResetCredits? {
        var request = URLRequest(url: resetListURL, timeoutInterval: 15)
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let json = JSONValue.parse(data), json["code"]?.int == 200,
              let payload = json["data"] else { return nil }
        var credits: [PlanLimits.ResetCredit] = []
        for (key, type, title) in [("fiveHourResets", "FIVE_HOUR", "Resets the 5-hour limit"), ("weekResets", "WEEK", "Resets the weekly limit")] {
            for card in payload[key]?.array ?? [] {
                guard card["available"]?.bool != false, let record = card["recordId"]?.int else { continue }
                credits.append(PlanLimits.ResetCredit(
                    id: "\(type):\(record)",
                    title: title,
                    detail: nil,
                    expiresAt: card["expireTime"]?.string.flatMap(resetDate)
                ))
            }
        }
        return PlanLimits.ResetCredits(availableCount: credits.count, credits: credits)
    }

    /// Spends one card. The id is the `type:recordId` pair `resetCredits` built.
    static func consumeResetCredit(apiKey: String, creditID: String) async throws -> PlanLimits.ResetOutcome {
        let parts = creditID.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2, let record = Int(parts[1]) else { throw ResetCreditError.unknownOutcome }
        var request = URLRequest(url: resetUseURL, timeoutInterval: 20)
        request.httpMethod = "POST"
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let body: JSONValue = .object([
            "targetType": .string("PERSONAL"),
            "resetType": .string(parts[0]),
            "recordId": .int(record),
            "requestId": .string(UUID().uuidString.lowercased()),
        ])
        request.httpBody = body.data()
        let (data, _) = try await URLSession.shared.data(for: request)
        guard let json = JSONValue.parse(data) else { throw ResetCreditError.unknownOutcome }
        guard json["code"]?.int == 200, json["success"]?.bool == true else {
            throw ResetCreditError.rejected(json["msg"]?.string ?? "Z.ai declined the reset.")
        }
        // The meters lag the reset by a moment; the dashboard waits the same two seconds.
        try? await Task.sleep(for: .seconds(2))
        return .reset
    }

    /// Z.ai writes its card expiries as naive `yyyy-MM-dd HH:mm:ss` in the operator's zone.
    private static func resetDate(_ text: String) -> Date? {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "Asia/Shanghai")
        formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return formatter.date(from: text)
    }

    private static let countFormatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.locale = Locale(identifier: "en_US")
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 0
        return formatter
    }()

    private static func formatCount(_ value: Double) -> String {
        countFormatter.string(from: NSNumber(value: value)) ?? "\(Int(value))"
    }
}
