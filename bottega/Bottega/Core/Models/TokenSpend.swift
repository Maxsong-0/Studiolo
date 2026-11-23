// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// One observation of newly consumed tokens, as a provider reported them.
///
/// The amount is what this event is responsible for — the usage one request or
/// one turn just consumed — never the occupancy of a context window, which
/// `ContextUsage` carries separately. `cachedInputTokens` is a subset of
/// `totalTokens` (input the provider served from its cache), so it is never
/// added on top of input tokens; only `totalTokens` is authoritative.
///
/// `agentID` names the hand a provider ran inside the session when the usage
/// belongs to that hand. A hand's spend is the account's spend and belongs to
/// the hand alone: a consumer must not attribute an event carrying an `agentID`
/// to the session's lead as well.
struct TokenSpend: Codable, Hashable, Sendable {
    var provider: ProviderKind
    var totalTokens: Int
    var inputTokens: Int? = nil
    var outputTokens: Int? = nil
    var cachedInputTokens: Int? = nil
    var model: String? = nil
    var agentID: String? = nil

    init(
        provider: ProviderKind,
        totalTokens: Int,
        inputTokens: Int? = nil,
        outputTokens: Int? = nil,
        cachedInputTokens: Int? = nil,
        model: String? = nil,
        agentID: String? = nil
    ) {
        self.provider = provider
        self.totalTokens = totalTokens
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.cachedInputTokens = cachedInputTokens
        self.model = model
        self.agentID = agentID
    }

    mutating func merge(_ other: TokenSpend) {
        totalTokens += other.totalTokens
        inputTokens = Self.sum(inputTokens, other.inputTokens)
        outputTokens = Self.sum(outputTokens, other.outputTokens)
        cachedInputTokens = Self.sum(cachedInputTokens, other.cachedInputTokens)
    }

    private static func sum(_ lhs: Int?, _ rhs: Int?) -> Int? {
        guard let lhs, let rhs else { return nil }
        return lhs + rhs
    }

    /// Read field by field, so a record written by a build that knew fewer of
    /// these fields still reads as the observation it was.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        provider = try container.decode(ProviderKind.self, forKey: .provider)
        totalTokens = container.value(.totalTokens, default: 0)
        inputTokens = container.value(.inputTokens, default: nil)
        outputTokens = container.value(.outputTokens, default: nil)
        cachedInputTokens = container.value(.cachedInputTokens, default: nil)
        model = container.value(.model, default: nil)
        agentID = container.value(.agentID, default: nil)
    }
}
