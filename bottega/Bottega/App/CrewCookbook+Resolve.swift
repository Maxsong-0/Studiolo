// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A recipe on this Mac: whether its providers are ready and which catalog models it
/// resolves to. This needs the provider registry and the settings, which live in the
/// app layer, so it sits apart from the recipes themselves.
extension CrewCookbook {
    /// Whether a provider is set up and switched on here.
    @MainActor static func isReady(_ provider: ProviderKind, registry: ProviderRegistry, settings: AppSettings) -> Bool {
        settings.isEnabled(provider) && registry.status(provider).isInstalled
    }

    /// The provider a side runs on here: the first of its providers that is ready and
    /// lists the model, or nil while none does.
    @MainActor static func provider(for side: CrewPairRecipe.Side, registry: ProviderRegistry, settings: AppSettings) -> ProviderKind? {
        side.providers.first { provider in
            isReady(provider, registry: registry, settings: settings)
                && (side.models.isEmpty || model(named: side.models, for: provider, registry: registry) != nil)
        }
    }

    /// The provider a recipe still needs, if none of a side's providers is ready: the
    /// side's first choice, as the one to set up.
    @MainActor static func missingProvider(for recipe: CrewPairRecipe, registry: ProviderRegistry, settings: AppSettings) -> ProviderKind? {
        for side in [recipe.lead, recipe.hands] where !side.providers.contains(where: { isReady($0, registry: registry, settings: settings) }) {
            return side.providers.first
        }
        return nil
    }

    /// The pair a recipe makes on this Mac, or nil while a side has no ready provider
    /// that lists its model.
    @MainActor static func resolve(_ recipe: CrewPairRecipe, registry: ProviderRegistry, settings: AppSettings) -> CrewPair? {
        guard let leadProvider = provider(for: recipe.lead, registry: registry, settings: settings),
              let handsProvider = provider(for: recipe.hands, registry: registry, settings: settings) else { return nil }
        var pair = CrewPair(provider: leadProvider)
        let lead = model(named: recipe.lead.models, for: leadProvider, registry: registry)
        pair.orchestratorModel = lead?.id
        pair.orchestratorEffort = effort(recipe.lead.effort, for: lead ?? registry.defaultModel(for: leadProvider))
        if handsProvider != leadProvider { pair.workerProvider = handsProvider }
        let hands = model(named: recipe.hands.models, for: handsProvider, registry: registry)
        pair.workerModel = hands?.id
        pair.workerEffort = effort(recipe.hands.effort, for: hands ?? registry.defaultModel(for: handsProvider))
        pair.maxHands = recipe.maxHands
        return pair
    }

    /// A fragment names a family or one model. A model's id, written exactly, takes that
    /// model when the catalog lists it, ahead of variants whose ids only contain it (a
    /// fast or a thinking row). A family name takes its newest member the catalog lists,
    /// so a new release moves a recipe up on its own; rows of the same version go to the
    /// one listed first. The id match minds case, so a family name ("Fable") never lands
    /// on an alias row that shares it (`fable`).
    @MainActor static func model(named names: [String], for provider: ProviderKind, registry: ProviderRegistry) -> ModelOption? {
        let options = registry.models(for: provider)
        for name in names {
            if let exact = options.first(where: { $0.id == name }) { return exact }
            let matches = options.filter { $0.name.localizedCaseInsensitiveContains(name) || $0.id.localizedCaseInsensitiveContains(name) }
            guard !matches.isEmpty else { continue }
            return matches.max { lhs, rhs in
                let lhsVersion = version(of: lhs) ?? -1
                let rhsVersion = version(of: rhs) ?? -1
                if lhsVersion != rhsVersion { return lhsVersion < rhsVersion }
                return options.firstIndex(where: { $0.id == lhs.id })! > options.firstIndex(where: { $0.id == rhs.id })!
            }
        }
        return nil
    }

    /// How new a catalog row's model is: the version its name gives ('Gemini 3.8 Flash' →
    /// 3.8), else the one in the model id it resolves to, else the one in its own id. Claude
    /// Code names its alias rows plainly ('Opus' for `opus`), so an alias ranks by the
    /// model it stands for today (`claude-opus-5-5`) and the newest Opus wins over a row
    /// pinned to an older one; the seed's 'Fable' row ranks by its id, `claude-fable-5-1[1m]`.
    private static func version(of option: ModelOption) -> Double? {
        version(inName: option.name)
            ?? option.resolvedModel.flatMap(version(inModelID:))
            ?? version(inModelID: option.id)
    }

    /// The first version number in a catalog name ('GLM-5.3' → 5.3, 'DeepSeek V4 Flash' →
    /// 4), if any, to two parts. A number a letter follows straight on is a size, not a
    /// version: the "1M" of 'Opus (1M context)' is a context length, "120B" a parameter
    /// count, and neither makes a row newer.
    private static func version(inName name: String) -> Double? {
        for match in name.matches(of: #/[0-9]+(?:\.[0-9]+)*/#) {
            if let next = name[match.range.upperBound...].first, next.isLetter { continue }
            return Double(match.output.split(separator: ".").prefix(2).joined(separator: "."))
        }
        return nil
    }

    /// The version in a model id ('claude-opus-5-5' → 5.5, 'gpt-5.6-terra' → 5.6), if any.
    /// A dashed id writes the minor number as the next part; a longer run of digits there
    /// is a release date ('claude-sonnet-4-20250514' → 4), and a bracketed suffix such as
    /// `[1m]` or a vendor prefix before a slash is no part of it.
    private static func version(inModelID id: String) -> Double? {
        let unsuffixed = id.prefix { $0 != "[" }
        let name = unsuffixed.split(separator: "/").last ?? unsuffixed
        let parts = name.split(separator: "-")
        func isNumber(_ part: Substring) -> Bool {
            !part.isEmpty && part.allSatisfy { $0.isASCII && $0.isNumber }
        }
        for (index, part) in parts.enumerated() {
            let pieces = part.split(separator: ".", omittingEmptySubsequences: false)
            guard pieces.allSatisfy(isNumber), let major = pieces.first, major.count <= 3 else { continue }
            if pieces.count > 1 { return Double(pieces.prefix(2).joined(separator: ".")) }
            if index + 1 < parts.count, isNumber(parts[index + 1]), parts[index + 1].count <= 2 {
                return Double("\(major).\(parts[index + 1])")
            }
            return Double(major)
        }
        return nil
    }
}
