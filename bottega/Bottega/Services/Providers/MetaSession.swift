// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Native Meta agent (Muse Spark): talks to https://api.meta.ai/v1/chat/completions
/// directly (OpenAI-compatible Chat Completions) and executes coding tools locally,
/// so no CLI install is needed — just a MODEL_API_KEY in Settings → Providers → Meta.
///
/// API reference: https://dev.meta.ai/docs/api-reference/
/// Auth is `Authorization: Bearer $MODEL_API_KEY`. Models are the Muse Spark
/// family (`muse-spark-1.3` default, plus contributor-tier variants) with a
/// 1,048,576-token context window. Chat Completions does not replay reasoning
/// across turns (that needs the Responses API's encrypted reasoning items);
/// each turn still reasons fresh, which is fine for the local tool loop here.
@MainActor
final class MetaSession: OpenAICompatibleSession {
    private static let config = Config(
        kind: .meta,
        title: "Meta",
        agentIdentity: { model in "Meta coding agent (Muse Spark \(model))" },
        defaultModel: "muse-spark-1.3",
        chatURL: MetaAPI.chatURL,
        contextWindow: MetaAPI.contextWindow,
        maxOutputTokens: nil,
        replaysReasoning: false,
        isVisionModel: MetaAPI.isVisionModel
    )

    init(configuration: SessionConfiguration) {
        super.init(config: Self.config, configuration: configuration)
    }

    /// Meta Chat Completions takes top-level `reasoning_effort`, mapped per model, and
    /// streams the usage block only when asked for.
    override func applyProviderPayload(_ payload: inout [String: JSONValue], model: String, effort: String?, withTools: Bool) {
        payload["stream_options"] = ["include_usage": true]
        if let mapped = Self.reasoningEffort(effort, model: model) { payload["reasoning_effort"] = .string(mapped) }
    }

    static func reasoningEffort(_ effort: String?, model: String? = nil) -> String? {
        guard let effort, !effort.isEmpty else { return nil }
        switch effort {
        case "minimal", "low", "medium", "high", "xhigh": return effort
        case "max":
            // Only muse-spark-1.3 itself accepts "max"; a thread saved at max
            // that moves to any other model must not send it.
            return MetaAPI.efforts(for: model ?? "muse-spark-1.3").contains("max") ? "max" : "xhigh"
        case "extra-high": return "xhigh"
        default: return nil
        }
    }

    override func friendlyError(_ message: String, statusCode: Int) -> String {
        let lower = message.lowercased()
        if statusCode == 401 || (lower.contains("invalid") && lower.contains("key")) || lower.contains("invalid_api_key") || lower.contains("authentication_error") {
            return "Meta rejected the API key. Check it in Settings → Providers → Meta (MODEL_API_KEY from dev.meta.ai)."
        }
        if statusCode == 404 || lower.contains("model_not_found") || (lower.contains("model") && lower.contains("not exist")) {
            return "Meta no longer offers that model. Pick Muse Spark 1.3 in the model picker."
        }
        if statusCode == 429 || lower.contains("rate limit") || lower.contains("rate_limit_exceeded") {
            return "Meta is rate-limited right now. Wait a moment and try again."
        }
        if statusCode == 504 || lower.contains("gateway_timeout") {
            return "Meta timed out before finishing. Try again; long generations already stream."
        }
        if lower.contains("reasoning_effort") && lower.contains("none") {
            return "Muse Spark always reasons; \"none\" is rejected. Pick Minimal or higher in the model picker."
        }
        if lower.contains("balance") || lower.contains("insufficient") || lower.contains("quota") || lower.contains("billing") {
            return "Meta reported a billing/quota problem. Check your Model API balance at dev.meta.ai, then try again."
        }
        if lower.contains("context") && (lower.contains("length") || lower.contains("window") || lower.contains("token")) {
            return "The conversation no longer fits Meta's 1M-token context window. Start a new thread or compact first."
        }
        return message.isEmpty ? "Meta stopped before finishing." : message
    }
}
