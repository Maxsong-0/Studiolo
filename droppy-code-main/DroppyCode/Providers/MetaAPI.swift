import Foundation

/// Minimal OpenAI-compatible HTTP client for api.meta.ai.
/// Chat streaming itself lives in MetaSession; this covers the
/// lightweight calls (key validation, model catalog) the registry needs.
///
/// Reference: https://dev.meta.ai/docs/api-reference/
/// Base URL `https://api.meta.ai/v1`, bearer auth with `MODEL_API_KEY`.
/// Endpoints used here:
/// - `GET /v1/models` — list available models (also validates the key)
/// - `POST /v1/chat/completions` — OpenAI-compatible messages-array endpoint (see MetaSession)
/// The Responses API (`POST /v1/responses`) carries reasoning across turns via
/// encrypted replay, but Chat Completions keeps the agent loop OpenAI-compatible
/// and matches the rest of Droppy Code's native providers.
enum MetaAPI {
    static let baseURL = URL(string: "https://api.meta.ai/v1")!
    static let chatURL = URL(string: "https://api.meta.ai/v1/chat/completions")!
    static let modelsURL = URL(string: "https://api.meta.ai/v1/models")!
    static let statusURL = URL(string: "https://api.meta.ai/v1/status")!

    /// Muse Spark context window per https://dev.meta.ai/docs/models
    static let contextWindow = 1_048_576
    static let maxOutputTokens = 131_072

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

    /// Probed live 2026-09-13: only `muse-spark-1.3` accepts "max". Every other
    /// checkpoint, including `muse-spark-1.3-contributor`, rejects it with a bare
    /// "The request contains invalid parameters" 400.
    static let maxEfforts = ["minimal", "low", "medium", "high", "xhigh", "max"]
    static let standardEfforts = ["minimal", "low", "medium", "high", "xhigh"]

    static func efforts(for id: String) -> [String] {
        id == "muse-spark-1.3" ? maxEfforts : standardEfforts
    }

    static func option(for id: String) -> ModelOption? {
        let efforts = efforts(for: id)
        switch id {
        case "muse-spark-1.3":
            return ModelOption(id: id, name: "Spark 1.3", detail: "Latest Muse Spark · best agentic coding · 1M context", efforts: efforts, defaultEffort: "high", isDefault: true)
        case "muse-spark-1.3-contributor":
            return ModelOption(id: id, name: "Spark 1.3 Contributor", detail: "Cheaper 1.3 · Meta may train on your chats · no max effort", efforts: efforts, defaultEffort: "high")
        case "muse-spark-1.2":
            return ModelOption(id: id, name: "Spark 1.2", detail: "Previous checkpoint · Standard tier · 1M context", efforts: efforts, defaultEffort: "high")
        case "muse-spark-1.2-contributor":
            return ModelOption(id: id, name: "Spark 1.2 Contributor", detail: "Cheaper 1.2 · Meta may train on your chats", efforts: efforts, defaultEffort: "high")
        case "muse-spark-1.1":
            return ModelOption(id: id, name: "Spark 1.1", detail: "Original checkpoint · Standard tier · 1M context", efforts: efforts, defaultEffort: "high")
        default:
            // Future Muse Spark checkpoints still show up. The endpoint also lists
            // non-chat models (muse-image-1.0, muse-voice-transcribe-1.0) that
            // fail on chat/completions, so only Spark ids are accepted.
            guard id.lowercased().hasPrefix("muse-spark") else { return nil }
            return ModelOption(id: id, name: id, efforts: efforts, defaultEffort: "high")
        }
    }

    /// All Muse Spark models are multimodal (text, image, video, audio, PDF in).
    static func isVisionModel(_ id: String?) -> Bool {
        guard let id, !id.isEmpty else { return true }
        return id.lowercased().hasPrefix("muse-spark")
    }
}
