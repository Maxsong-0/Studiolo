// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Native Z.ai agent: talks to https://api.z.ai/api/coding/paas/v4/chat/completions
/// directly (OpenAI-compatible, the GLM Coding Plan endpoint) and executes coding
/// tools locally, so no CLI install is needed — just an API key in Settings → Providers → Z.ai.
@MainActor
final class ZaiSession: OpenAICompatibleSession {
    private static let config = Config(
        kind: .zai,
        title: "Z.ai",
        agentIdentity: { _ in "Z.ai GLM coding agent" },
        defaultModel: "glm-5.3",
        chatURL: ZaiAPI.chatURL,
        contextWindow: ZaiAPI.contextWindow,
        maxOutputTokens: ZaiAPI.maxOutputTokens,
        replaysReasoning: true,
        isVisionModel: ZaiAPI.isVisionModel
    )

    init(configuration: SessionConfiguration) {
        super.init(config: Self.config, configuration: configuration)
    }

    /// GLM asks for a `max_tokens` cap, an always-on thinking mode that keeps
    /// prior thinking, and `tool_stream` alongside the tools.
    override func applyProviderPayload(_ payload: inout [String: JSONValue], model: String, effort: String?, withTools: Bool) {
        if withTools { payload["tool_stream"] = true }
        payload["thinking"] = ["type": "enabled", "clear_thinking": false]
        payload["max_tokens"] = .int(ZaiAPI.maxOutputTokens)
        if let mapped = Self.reasoningEffort(effort) { payload["reasoning_effort"] = .string(mapped) }
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

    /// Z.ai answers every one of these refusals with HTTP 429, so the status alone says
    /// nothing and the body has to be read: a rate limit, an overloaded service, a model
    /// the plan does not carry and a spent window are all 429s (the business codes and
    /// their messages are docs.z.ai/api-reference/api-code). A 429 that is not a spent
    /// window is never reported as one: an account with its windows untouched must not be
    /// told its limit is reached.
    override func friendlyError(_ message: String, statusCode: Int) -> String {
        let lower = message.lowercased()
        // A context-length message also says "limit", so it is read before the plan window.
        if (lower.contains("context") && (lower.contains("length") || lower.contains("window") || lower.contains("token"))) || lower.contains("prompt too long") {
            return "The conversation no longer fits GLM-5.3's context window. Start a new thread or compact first."
        }
        if statusCode == 401 || lower.contains("token expired") || lower.contains("incorrect") || lower.contains("authentication") {
            return "Z.ai rejected the API key. Check it in Settings → Providers → Z.ai."
        }
        // 1311: the plan does not carry the model asked for. Nothing was spent, no window
        // is involved, and the model is named back so the picker can be set right.
        if lower.contains("does not yet include access") {
            return "Your Z.ai Coding Plan does not include \(Self.planRefusedModel(in: message)). Pick GLM-5.3 or GLM-5.3 Flash for the model, or move up a plan tier."
        }
        // 1302 and 1305: the account's request rate, or Z.ai's own load. Both pass on their own.
        if lower.contains("rate limit reached") || lower.contains("too many requests") {
            return "Z.ai is throttling requests right now — its plans cap how many run at once. Wait a moment and send again; fewer at a time get through."
        }
        if lower.contains("temporarily overloaded") {
            return "Z.ai is overloaded right now, which says nothing about your plan. Send the message again in a moment."
        }
        // 1313: the Fair Usage Policy, which lifts on its own like a throttle.
        if lower.contains("fair usage") {
            return "Z.ai has paused this account's requests under its Fair Usage Policy. It lifts on its own; details at z.ai/devpack/usage-policy."
        }
        // 1113: no package on this key at all.
        if lower.contains("insufficient balance") || lower.contains("no resource package") || lower.contains("1113") {
            return "This Z.ai key has no active Coding Plan. Subscribe at z.ai/subscribe, then try again."
        }
        // 1309: the subscription lapsed.
        if lower.contains("package has expired") {
            return "Your GLM Coding Plan has expired. Renew it at z.ai/subscribe, then try again."
        }
        // 1314 and 1315: enterprise packages and their keys.
        if lower.contains("enterprise") {
            return "This Z.ai key belongs to an enterprise coding package. Use the personal Coding Plan key from z.ai/manage-apikey/subscription."
        }
        // A spent window (1308, 1310, 1316–1321): "usage limit" is what auto-continue
        // listens for; it then reads the reset time from Z.ai's quota API.
        if lower.contains("usage limit") || lower.contains("limit exhausted") || lower.contains("extra usage") {
            return "Your GLM Coding Plan usage limit is reached for this window. It refreshes on its own; the usage ring shows when. Details at z.ai/manage-apikey/subscription."
        }
        // 1211, and any other wording for a model Z.ai does not have.
        if lower.contains("unknown model") || (lower.contains("model") && lower.contains("not exist")) {
            return "Z.ai does not offer that model. Pick GLM-5.3 or GLM-5.3 Flash in the model picker."
        }
        // Anything else is Z.ai saying no for a reason this build does not know, and it is
        // passed on as Z.ai worded it rather than guessed at.
        return message.isEmpty ? "Z.ai stopped before finishing." : message
    }

    /// The model named in a 1311 message, quoted back so it can be changed.
    private static func planRefusedModel(in message: String) -> String {
        guard let range = message.range(of: "access to ", options: .caseInsensitive) else { return "that model" }
        let named = message[range.upperBound...].trimmingCharacters(in: CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: ".`\"'")))
        return named.isEmpty ? "that model" : named
    }
}
