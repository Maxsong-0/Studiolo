// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The pairs cookbook: Claude's pairs as they resolve on this Mac, each added with one
/// click. A recipe whose providers are not set up here waits, dimmed, below.
struct CrewCookbookPanel: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let registry = model.providers
        let settings = model.settings
        let ready = CrewCookbook.recipes.compactMap { recipe -> (recipe: CrewPairRecipe, pair: CrewPair)? in
            CrewCookbook.resolve(recipe, registry: registry, settings: settings).map { (recipe, $0) }
        }
        let waiting = CrewCookbook.recipes.compactMap { recipe -> (recipe: CrewPairRecipe, provider: ProviderKind)? in
            CrewCookbook.missingProvider(for: recipe, registry: registry, settings: settings).map { (recipe, $0) }
        }
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Cookbook")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text("Claude pairs, ready to add: which model leads, at what effort, and which ones build. Add one and it is in the model picker.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 16)
            .padding(.top, 16)
            .padding(.bottom, 12)
            Divider().opacity(0.5)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if ready.isEmpty {
                        Text("Set up Claude in Settings › Providers, and its models bring these pairs here.")
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(16)
                    }
                    ForEach(ready, id: \.recipe.id) { entry in
                        CrewCookbookRow(recipe: entry.recipe, pair: entry.pair, missing: nil)
                        Divider().opacity(0.35).padding(.leading, 16)
                    }
                    if !waiting.isEmpty {
                        Text("Needs a provider")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(.horizontal, 16)
                            .padding(.top, 14)
                            .padding(.bottom, 4)
                        ForEach(waiting, id: \.recipe.id) { entry in
                            CrewCookbookRow(recipe: entry.recipe, pair: nil, missing: entry.provider)
                                .opacity(0.55)
                            Divider().opacity(0.35).padding(.leading, 16)
                        }
                    }
                }
                .padding(.bottom, 8)
            }
        }
        .frame(width: 460, height: 540)
        .task {
            // The recipes pick their models from the catalogs of the providers they name,
            // so those that are ready are loaded; no other provider's CLI starts for this.
            for provider in CrewCookbook.providers where CrewCookbook.isReady(provider, registry: registry, settings: settings) {
                await registry.loadCatalog(provider)
            }
        }
    }
}

/// One recipe: the provider marks, the name and the pitch, the models it resolved to,
/// and the button that adds it. A recipe still missing a provider says which.
private struct CrewCookbookRow: View {
    @Environment(AppModel.self) private var model
    let recipe: CrewPairRecipe
    let pair: CrewPair?
    let missing: ProviderKind?

    var body: some View {
        let registry = model.providers
        let isAdded = pair.map { CrewCookbook.isAdded($0, in: model.settings.crewPairs) } ?? false
        HStack(alignment: .center, spacing: 12) {
            // The marks show the providers the pair resolved to here; a waiting recipe shows its first choices.
            CrewPairMark(
                lead: pair?.provider ?? recipe.lead.providers.first ?? .claude,
                hands: pair?.handsProvider ?? recipe.hands.providers.first ?? .claude
            )
            .frame(width: 52, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: pair.map { CrewPairSummary.title($0, registry: registry) } ?? recipe.title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                Text(verbatim: recipe.tagline)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                if let pair {
                    // A same-provider pair with no efforts or cap set has nothing to say here; an
                    // empty Text would still take a line and leave the row with blank space below it.
                    // The cap is part of what a recipe sets, and the title, which names the models, does not say it.
                    let providersLine: String? = pair.handsProvider == pair.provider ? nil : "\(pair.provider.displayName) lead, \(pair.handsProvider.displayName) hands"
                    let capLine: String? = pair.maxHands.map { $0 == 1 ? "One hand at a time" : "Up to \($0) hands" }
                    let detail = [providersLine, CrewPairSummary.efforts(pair), capLine].compactMap { $0 }.joined(separator: " · ")
                    if !detail.isEmpty {
                        Text(verbatim: detail)
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText.opacity(0.8))
                            .lineLimit(2)
                    }
                } else if let missing {
                    Text(verbatim: "Set up \(missing.displayName) to add this pair.")
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText.opacity(0.8))
                }
            }
            Spacer(minLength: 8)
            if let pair {
                if isAdded {
                    Label("Added", systemImage: "checkmark")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Chrome.secondaryText)
                        .padding(.horizontal, 10)
                } else {
                    Button {
                        withAnimation(Chrome.panelSlide) { model.settings.addCrewPair(pair) }
                    } label: {
                        Label("Add", systemImage: "plus")
                    }
                    .buttonStyle(.glassProminent)
                    .controlSize(.small)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}
