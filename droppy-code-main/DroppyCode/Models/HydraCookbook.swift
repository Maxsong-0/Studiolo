import Foundation

/// A pair worth having, written as a recipe: which model leads at what effort and
/// which model builds at what effort. A recipe names models, not providers: each
/// side lists the providers that serve its model, and the first one set up on this
/// Mac whose catalog has the model is the one the pair runs on.
struct HydraPairRecipe: Identifiable, Sendable {
    struct Side: Sendable {
        /// Providers that serve this side's model, best first: the model's own house
        /// ahead of aggregators, so a lead on Claude's own CLI wins over one relayed.
        let providers: [ProviderKind]
        /// Names to look for in the provider's catalog, best first; a recipe whose
        /// names find nothing is not offered.
        let models: [String]
        /// The effort asked for, kept only when the model found offers it.
        let effort: String?
    }

    let id: String
    let title: String
    let tagline: String
    let lead: Side
    let heads: Side
    let maxHeads: Int?
}

enum HydraCookbook {
    /// The best pairs, strongest first: cross-model pairs, then one model at two efforts.
    static let recipes: [HydraPairRecipe] = [
        HydraPairRecipe(id: "claude-fable-opus", title: "Fable 5.1 leads Opus 5", tagline: "The deepest lead over the strongest builders: Fable plans and checks, Opus heads build. Set the thinking effort of each side yourself.", lead: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: nil), heads: .init(providers: [.claude, .commandcode], models: ["Opus"], effort: nil), maxHeads: nil),
        HydraPairRecipe(id: "claude-opus-gemini-flash", title: "Opus 5 leads Gemini 3.8 Flash", tagline: "A careful lead and a wide, fast team: Opus writes the briefs, six Gemini Flash heads sprint through them.", lead: .init(providers: [.claude, .commandcode], models: ["Opus"], effort: "high"), heads: .init(providers: [.antigravity, .commandcode], models: ["Gemini 3.8 Flash", "Gemini", "Flash"], effort: "medium"), maxHeads: 6),
        HydraPairRecipe(id: "claude-opus-deepseek", title: "Opus 5 leads DeepSeek V4.1", tagline: "Opus keeps the plan tight; DeepSeek V4.1 Flash heads keep the bill small on the routine parts.", lead: .init(providers: [.claude, .commandcode], models: ["Opus"], effort: "high"), heads: .init(providers: [.deepseek, .commandcode], models: ["V4.1 Flash", "V4.1", "DeepSeek"], effort: "high"), maxHeads: nil),
        HydraPairRecipe(id: "claude-opus-glm", title: "Opus 5 leads GLM-5.3 Flash", tagline: "Opus keeps the plan tight; GLM-5.3 Flash heads on a Z.ai Coding Plan keep the bill flat.", lead: .init(providers: [.claude, .commandcode], models: ["Opus"], effort: "high"), heads: .init(providers: [.zai, .commandcode], models: ["5.3 Flash", "GLM"], effort: "high"), maxHeads: nil),
        HydraPairRecipe(id: "claude-fable-composer", title: "Fable 5.1 leads Composer 2.5", tagline: "Fable thinks; Cursor's Composer heads type at the speed Composer is known for.", lead: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: "high"), heads: .init(providers: [.cursor], models: ["Composer"], effort: nil), maxHeads: nil),
        HydraPairRecipe(id: "astra-spark", title: "Astra 6 leads Spark 1.3 Contributor", tagline: "GPT-6 Astra orchestrates at high; Muse Spark Contributor heads build at medium.", lead: .init(providers: [.codex, .commandcode], models: ["Astra"], effort: "high"), heads: .init(providers: [.meta, .commandcode], models: ["Contributor"], effort: "medium"), maxHeads: nil),
        HydraPairRecipe(id: "astra-terra", title: "Astra 6 leads Terra 5.6", tagline: "OpenAI top to bottom: Astra plans and reviews, Terra does the work at medium.", lead: .init(providers: [.codex, .commandcode], models: ["Astra"], effort: "high"), heads: .init(providers: [.codex, .commandcode], models: ["Terra"], effort: "medium"), maxHeads: nil),
        HydraPairRecipe(id: "astra-astra", title: "Astra 6 high, Astra 6 medium", tagline: "One model, two efforts: Astra thinks hard about the plan and the review, and Astra heads build at a working effort.", lead: .init(providers: [.codex, .commandcode], models: ["Astra"], effort: "high"), heads: .init(providers: [.codex, .commandcode], models: ["Astra"], effort: "medium"), maxHeads: nil),
        HydraPairRecipe(id: "claude-opus-opus", title: "Opus 5 max, Opus 5 medium", tagline: "Opus thinks as hard as it can where it counts; Opus heads at medium do the building.", lead: .init(providers: [.claude, .commandcode], models: ["Opus"], effort: "max"), heads: .init(providers: [.claude, .commandcode], models: ["Opus"], effort: "medium"), maxHeads: nil),
        HydraPairRecipe(id: "claude-fable-fable", title: "Fable 5.1 high, Fable 5.1 medium", tagline: "For the hardest, longest jobs: every seat on the most capable model, the lead at high and the heads at medium.", lead: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: "high"), heads: .init(providers: [.claude, .commandcode], models: ["Fable"], effort: "medium"), maxHeads: nil),
    ]

    /// Whether a provider is set up and switched on here.
    @MainActor static func isReady(_ provider: ProviderKind, registry: ProviderRegistry, settings: AppSettings) -> Bool {
        settings.isEnabled(provider) && registry.status(provider).isInstalled
    }

    /// The provider a side runs on here: the first of its providers that is ready and
    /// lists the model, or nil while none does.
    @MainActor static func provider(for side: HydraPairRecipe.Side, registry: ProviderRegistry, settings: AppSettings) -> ProviderKind? {
        side.providers.first { provider in
            isReady(provider, registry: registry, settings: settings)
                && (side.models.isEmpty || model(named: side.models, for: provider, registry: registry) != nil)
        }
    }

    /// The provider a recipe still needs, if none of a side's providers is ready: the
    /// side's first choice, as the one to set up.
    @MainActor static func missingProvider(for recipe: HydraPairRecipe, registry: ProviderRegistry, settings: AppSettings) -> ProviderKind? {
        for side in [recipe.lead, recipe.heads] where !side.providers.contains(where: { isReady($0, registry: registry, settings: settings) }) {
            return side.providers.first
        }
        return nil
    }

    /// Whether a side has a ready provider but none of them lists its model.
    @MainActor static func missingModel(for recipe: HydraPairRecipe, registry: ProviderRegistry, settings: AppSettings) -> Bool {
        guard missingProvider(for: recipe, registry: registry, settings: settings) == nil else { return false }
        return provider(for: recipe.lead, registry: registry, settings: settings) == nil
            || provider(for: recipe.heads, registry: registry, settings: settings) == nil
    }

    /// The pair a recipe makes on this Mac, or nil while a side has no ready provider
    /// that lists its model.
    @MainActor static func resolve(_ recipe: HydraPairRecipe, registry: ProviderRegistry, settings: AppSettings) -> HydraPair? {
        guard let leadProvider = provider(for: recipe.lead, registry: registry, settings: settings),
              let headsProvider = provider(for: recipe.heads, registry: registry, settings: settings) else { return nil }
        var pair = HydraPair(provider: leadProvider)
        let lead = model(named: recipe.lead.models, for: leadProvider, registry: registry)
        pair.orchestratorModel = lead?.id
        pair.orchestratorEffort = effort(recipe.lead.effort, for: lead ?? registry.defaultModel(for: leadProvider))
        if headsProvider != leadProvider { pair.workerProvider = headsProvider }
        let heads = model(named: recipe.heads.models, for: headsProvider, registry: registry)
        pair.workerModel = heads?.id
        pair.workerEffort = effort(recipe.heads.effort, for: heads ?? registry.defaultModel(for: headsProvider))
        pair.maxHeads = recipe.maxHeads
        return pair
    }

    /// A fragment names a family; the newest member the catalog lists wins, so a new
    /// release moves a recipe up on its own.
    @MainActor static func model(named names: [String], for provider: ProviderKind, registry: ProviderRegistry) -> ModelOption? {
        let options = registry.models(for: provider)
        for name in names {
            let matches = options.filter { $0.name.localizedCaseInsensitiveContains(name) || $0.id.localizedCaseInsensitiveContains(name) }
            guard !matches.isEmpty else { continue }
            return matches.max { lhs, rhs in
                let lhsVersion = version(of: lhs.name) ?? -1
                let rhsVersion = version(of: rhs.name) ?? -1
                if lhsVersion != rhsVersion { return lhsVersion < rhsVersion }
                return options.firstIndex(where: { $0.id == lhs.id })! > options.firstIndex(where: { $0.id == rhs.id })!
            }
        }
        return nil
    }

    /// The first version number in a catalog name ('Gemini 3.8 Flash' → 3.8), if any.
    private static func version(of name: String) -> Double? {
        guard let match = name.firstMatch(of: #/[0-9]+(?:\.[0-9]+)?/#) else { return nil }
        return Double(match.output)
    }

    /// The effort asked for when the model offers it; nil otherwise, which keeps the default.
    static func effort(_ wanted: String?, for option: ModelOption?) -> String? {
        guard let wanted, let option, option.efforts.contains(wanted) else { return nil }
        return wanted
    }

    /// Whether a pair like this one is already set up: the same providers and models.
    static func isAdded(_ pair: HydraPair, in pairs: [HydraPair]) -> Bool {
        pairs.contains { $0.provider == pair.provider && $0.headsProvider == pair.headsProvider && $0.orchestratorModel == pair.orchestratorModel && $0.workerModel == pair.workerModel }
    }
}
