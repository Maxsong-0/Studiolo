// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A pair worth having, written as a recipe: which model leads at what effort and
/// which model builds at what effort. A recipe names models, not providers: each
/// side lists the providers that serve its model, and the first one set up on this
/// Mac whose catalog has the model is the one the pair runs on.
struct CrewPairRecipe: Identifiable, Sendable {
    struct Side: Sendable {
        /// Providers that serve this side's model, best first: the model's own house
        /// ahead of aggregators, so a lead on Claude's own CLI wins over one relayed.
        let providers: [ProviderKind]
        /// Names to look for in the provider's catalog, best first: a family name
        /// ("Fable") takes the newest member listed, a model's id ("claude-opus-5-5")
        /// takes that model. A recipe whose names find nothing is not offered.
        let models: [String]
        /// The effort asked for, kept only when the model found offers it.
        let effort: String?
    }

    let id: String
    let title: String
    let tagline: String
    let lead: Side
    let hands: Side
    let maxHands: Int?
}

enum CrewCookbook {
    /// Claude's pairs, strongest first: Fable over Opus 5.5, then one model at two
    /// efforts. Opus goes by its id, not its family: the CLI lists the newest Opus only
    /// by an alias that moves on with every release, and these pairs mean Opus 5.5.
    /// Every side runs on Claude's own CLI, with Command Code, which relays the same
    /// models, behind it for a Mac that has only that one set up. No two recipes share
    /// their models: `isAdded` tells pairs apart by models alone, and efforts and the
    /// cap are the pair editor's to change once a recipe is added.
    static let recipes: [CrewPairRecipe] = [
        CrewPairRecipe(id: "max-fable-opus", title: "Fable 5.1 max leads Opus 5.5 max", tagline: "The deepest lead over the strongest builders, both at full depth: Fable plans and checks at max, and up to four Opus 5.5 hands build at max.", lead: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: "max"), hands: .init(providers: [.claude, .commandcode], models: ["claude-opus-5-5"], effort: "max"), maxHands: 4),
        CrewPairRecipe(id: "claude-opus-opus", title: "Opus 5.5 max, Opus 5.5 medium", tagline: "Opus 5.5 thinks as hard as it can where it counts; Opus 5.5 hands at medium do the building.", lead: .init(providers: [.claude, .commandcode], models: ["claude-opus-5-5"], effort: "max"), hands: .init(providers: [.claude, .commandcode], models: ["claude-opus-5-5"], effort: "medium"), maxHands: nil),
        CrewPairRecipe(id: "claude-fable-fable", title: "Fable 5.1 high, Fable 5.1 medium", tagline: "For the hardest, longest jobs: every seat on the most capable model, the lead at high and the hands at medium.", lead: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: "high"), hands: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: "medium"), maxHands: nil),
    ]

    /// Every provider a recipe runs on, each once, in the order the recipes name them:
    /// the catalogs the cookbook reads to resolve its pairs.
    static var providers: [ProviderKind] {
        var seen: [ProviderKind] = []
        for recipe in recipes {
            for provider in recipe.lead.providers + recipe.hands.providers where !seen.contains(provider) {
                seen.append(provider)
            }
        }
        return seen
    }

    /// The effort asked for when the model offers it; nil otherwise, which keeps the default.
    static func effort(_ wanted: String?, for option: ModelOption?) -> String? {
        guard let wanted, let option, option.efforts.contains(wanted) else { return nil }
        return wanted
    }

    /// Whether a pair like this one is already set up: the same providers and models.
    static func isAdded(_ pair: CrewPair, in pairs: [CrewPair]) -> Bool {
        pairs.contains { $0.provider == pair.provider && $0.handsProvider == pair.handsProvider && $0.orchestratorModel == pair.orchestratorModel && $0.workerModel == pair.workerModel }
    }
}
