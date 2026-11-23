// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Native DeepSeek agent: talks to https://api.deepseek.com/chat/completions
/// directly (OpenAI-compatible) and executes coding tools locally, so no CLI
/// install is needed — just an API key in Settings → Providers → DeepSeek.
@MainActor
final class DeepSeekSession: OpenAICompatibleSession {
    private static let config = Config(
        kind: .deepseek,
        title: "DeepSeek",
        agentIdentity: { _ in "DeepSeek coding agent" },
        defaultModel: "deepseek-v4-pro",
        chatURL: DeepSeekAPI.chatURL,
        contextWindow: DeepSeekAPI.contextWindow,
        maxOutputTokens: nil,
        replaysReasoning: true,
        isVisionModel: DeepSeekAPI.isVisionModel
    )

    init(configuration: SessionConfiguration) {
        super.init(config: Self.config, configuration: configuration)
    }

    /// Thinking mode needs the effort mapping on top of the shared payload, and the
    /// usage block only streams when asked for: without it the ledger and the context
    /// window would never hear from DeepSeek.
    override func applyProviderPayload(_ payload: inout [String: JSONValue], model: String, effort: String?, withTools: Bool) {
        payload["stream_options"] = ["include_usage": true]
        payload["thinking"] = ["type": "enabled"]
        if let mapped = Self.reasoningEffort(effort) { payload["reasoning_effort"] = .string(mapped) }
    }

    static func reasoningEffort(_ effort: String?) -> String? {
        guard let effort, !effort.isEmpty else { return nil }
        switch effort {
        case "low", "high", "max": return effort
        case "medium", "xhigh", "extra-high": return "high"
        default: return nil
        }
    }

    override func friendlyError(_ message: String, statusCode: Int) -> String {
        let lower = message.lowercased()
        if lower.contains("balance") || lower.contains("insufficient") {
            return "DeepSeek has no balance left. Top up at platform.deepseek.com, then try again."
        }
        if statusCode == 429 || lower.contains("rate limit") {
            return "DeepSeek is rate-limited right now. Wait a moment and try again."
        }
        if lower.contains("context") && (lower.contains("length") || lower.contains("window") || lower.contains("token")) {
            return "The conversation no longer fits DeepSeek's context window. Start a new thread or compact first."
        }
        if lower.contains("model") && lower.contains("not exist") {
            return "DeepSeek no longer offers that model. Pick V4 Pro or V4.1 Flash in the model picker."
        }
        return message.isEmpty ? "DeepSeek stopped before finishing." : message
    }
}
