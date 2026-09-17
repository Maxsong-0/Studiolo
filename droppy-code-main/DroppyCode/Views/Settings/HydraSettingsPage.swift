import SwiftUI

/// Switches Hydra on for the app and sets up its heads: how they work, which messages go
/// to them, what happens when the team is done, and which model leads which, per provider.
struct HydraSettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        VStack(alignment: .leading, spacing: Chrome.sectionHeaderSpacing) {
            ChromeCard {
                HStack(spacing: 14) {
                    HStack(spacing: -9) {
                        ForEach(0..<5, id: \.self) { index in
                            HydraGlyph(persona: HydraRoster.persona(at: index), size: 26)
                        }
                    }
                    .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 4) {
                            Text("Hydra")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(Chrome.primaryText)
                            InfoHoverButton(help: "How Hydra works") { HydraInfoPopover(topic: .hydra) }
                        }
                        Text("One chat leads a team of heads on big jobs.")
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 12)
                    SettingsSwitch(isOn: $settings.hydraEnabled)
                }
                .padding(.leading, 16)
                .padding(.trailing, Chrome.rowControlTrailingPadding)
                .padding(.vertical, 14)
            }
            // One note for the whole page: the sections below dim together rather than
            // each row repeating why it is off.
            if !settings.hydraEnabled {
                Text("Switch Hydra on to set up its heads.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .padding(.horizontal, 4)
            }
        }
        Group {
            ChromeSection(title: "Heads", accessory: AnyView(InfoHoverButton(help: "How heads work") { HydraInfoPopover(topic: .heads) })) {
                ChromeCard {
                    toggleRow("Own copy of the checkout", detail: "Changes land in the chat's checkout when the head reports.", isOn: $settings.hydraIsolateHeads)
                    ChromeRowDivider()
                    toggleRow("Clear finished heads", detail: "A finished head moves from the panel to the sidebar.", isOn: $settings.hydraAutoClearFinished)
                    ChromeRowDivider()
                    toggleRow("Show every step", detail: "Each step in the head's panel instead of its progress bar.", isOn: $settings.hydraShowsHeadDetails)
                    ChromeRowDivider()
                    toggleRow("Pop heads out automatically", detail: "While there is room beside the chat, each head after the first gets a panel of its own, on the left and the right, until each side is full; the usage panel keeps its spot.", isOn: $settings.hydraAutoPopsHeads)
                    ChromeRowDivider()
                    toggleRow("Heads work at a working effort", detail: "A lead thinking above medium sends its heads out at medium on the same model — on a scale without a medium (Z.ai), at the scale's own middle rung; a pair that names the heads' effort keeps it.", isOn: $settings.hydraTempersHeadEffort)
                }
            }
            ChromeSection(title: "Messages") {
                ChromeCard {
                    toggleRow("Queued follow-ups start on heads", detail: "A prompt queued behind a running turn goes out at once.", isOn: $settings.hydraQueueHeads, blockedBy: settings.hydraAlwaysHeads ? "Every message already goes to a head." : nil)
                    ChromeRowDivider()
                    toggleRow("Every message goes to a head", detail: "The lead only hears the reports.", isOn: $settings.hydraAlwaysHeads)
                }
            }
            ChromeSection(title: "When the team is done") {
                ChromeCard {
                    toggleRow("Lead checks the heads' work", detail: "Reads every changed file and corrects it. Slower, more careful.", isOn: $settings.hydraReviewHeads)
                    ChromeRowDivider()
                    toggleRow("Merge automatically", detail: "The team's changes go out as a merge request and are merged.", isOn: $settings.hydraAutoMerge)
                }
            }
            ChromeSection(title: "Pairs") {
                ChromeCard {
                    HStack(alignment: .center, spacing: 14) {
                        // A lead and the heads it sends out: the pair, drawn with the roster.
                        HStack(spacing: 6) {
                            HydraGlyph(persona: HydraRoster.persona(at: 0), size: 30)
                            Image(systemName: "arrow.right")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(Chrome.secondaryText)
                            HStack(spacing: -8) {
                                ForEach(1..<4, id: \.self) { index in
                                    HydraGlyph(persona: HydraRoster.persona(at: index), size: 22)
                                }
                            }
                        }
                        .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 3) {
                            Text("One leads, the others build")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(Chrome.primaryText)
                            Text("A pair says which model leads and which runs the heads. Put a deep thinker over quick hands on one provider, or lead on one provider and send the heads out on another: the strongest lead with the fastest team, every time.")
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 12)
                        HydraCookbookButton()
                    }
                    .padding(.leading, 16)
                    .padding(.trailing, Chrome.rowControlTrailingPadding)
                    .padding(.vertical, 14)
                }
                ChromeCard {
                    if settings.hydraPairs.isEmpty {
                        ChromeRow(title: "No pairs yet", detail: "Heads run on the chat's own model until a chat joins a pair.") {
                            HydraAddPairButton()
                        }
                    } else {
                        ForEach(Array(settings.hydraPairs.enumerated()), id: \.element.id) { index, pair in
                            VStack(spacing: 0) {
                                if index > 0 { ChromeRowDivider() }
                                HydraPairRow(pair: pair)
                            }
                        }
                        ChromeRowDivider()
                        ChromeRow(title: "Another pair", detail: "One per provider is enough.") {
                            HydraAddPairButton()
                        }
                    }
                }
                Text("Pairs sit at the top of the model picker: a tap puts the chat in one.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .padding(.horizontal, 4)
            }
        }
        .disabled(!settings.hydraEnabled)
        .opacity(settings.hydraEnabled ? 1 : 0.5)
        .animation(.smooth(duration: 0.2), value: settings.hydraEnabled)
        ChromeSection(title: "Where heads run") {
            ChromeCard {
                ChromeRow(title: "Claude, Codex and Copilot", detail: "In the provider's own session, through the lead's own agent tools.") {
                    HStack(alignment: .center, spacing: 6) {
                        ProviderIcon(provider: .claude, size: 14)
                        ProviderIcon(provider: .codex, size: 14)
                        // Copilot's mark is wider than tall, so it fills less of a
                        // square frame; size it up to share the same optical height.
                        ProviderIcon(provider: .copilot, size: 16.75)
                    }
                    .foregroundStyle(Chrome.secondaryText)
                }
                ChromeRowDivider()
                ChromeRow(title: "Other providers", detail: "As threads of their own, asked for at the end of the lead's reply.") {
                    EmptyView()
                }
                ChromeRowDivider()
                ChromeRow(title: "Across providers", detail: "A pair can lead on one provider and run its heads on another.") {
                    HydraPairMark(lead: .claude, heads: .antigravity, leadSize: 14)
                    .foregroundStyle(Chrome.secondaryText)
                }
            }
        }
    }

    /// A toggle row. A reason it is blocked stands in for its detail, and the row is dimmed
    /// and inert while there is one.
    private func toggleRow(_ title: String, detail: String, isOn: Binding<Bool>, blockedBy reason: String? = nil) -> some View {
        ChromeRow(title: title, detail: reason ?? detail) {
            SettingsSwitch(isOn: isOn)
        }
        .disabled(reason != nil)
        .opacity(reason == nil ? 1 : 0.5)
    }
}

/// Opens the cookbook: the best pairs for the providers set up here, added with a click.
private struct HydraCookbookButton: View {
    @State private var isOpen = false

    var body: some View {
        Button {
            isOpen = true
        } label: {
            Label("Open cookbook", systemImage: "book")
        }
        .buttonStyle(.glass)
        .popover(isPresented: $isOpen, arrowEdge: .trailing) {
            HydraCookbookPanel()
                .presentedChrome()
        }
    }
}

private struct HydraAddPairButton: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Button {
            let installed = ProviderKind.allCases.filter { model.providers.status($0).isInstalled && model.settings.isEnabled($0) }
            let taken = Set(model.settings.hydraPairs.map(\.provider))
            let provider = installed.first { !taken.contains($0) } ?? installed.first ?? model.settings.defaultProvider
            withAnimation(Chrome.panelSlide) { model.settings.addHydraPair(HydraPair(provider: provider)) }
        } label: {
            Label("Add pair", systemImage: "plus")
        }
        .buttonStyle(.glass)
    }
}

/// One pair: who leads whom, on what effort, with the editor behind a click.
private struct HydraPairRow: View {
    @Environment(AppModel.self) private var model
    let pair: HydraPair

    @State private var isEditing = false
    @State private var isHovering = false

    var body: some View {
        let registry = model.providers
        HStack(spacing: 10) {
            Button {
                isEditing = true
            } label: {
                HStack(spacing: 12) {
                    HydraPairIcons(pair: pair)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: HydraPairSummary.title(pair, registry: registry))
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.primaryText)
                        Text(verbatim: summary(registry: registry))
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                    }
                    Spacer(minLength: 8)
                    Image(systemName: "slider.horizontal.3")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                        .opacity(isHovering ? 1 : 0)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .onHover { isHovering = $0 }
            .popover(isPresented: $isEditing, arrowEdge: .trailing) {
                HydraPairEditor(pairID: pair.id)
                    .frame(width: 360)
                    .presentedChrome()
            }
            Button {
                withAnimation(Chrome.panelSlide) { model.removeHydraPair(pair.id) }
            } label: {
                Image(systemName: "minus.circle")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 24, height: 24)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Remove this pair")
            .accessibilityLabel(Text("Remove this pair"))
        }
        .padding(.leading, 16)
        .padding(.trailing, Chrome.rowControlTrailingPadding)
        .padding(.vertical, 9)
        .task(id: pair.headsProvider) {
            await registry.loadCatalog(pair.provider)
            await registry.loadCatalog(pair.headsProvider)
        }
    }

    /// The row's second line: only what the title and the icons do not already say. The
    /// title names the models, the icons the providers, so this reads the efforts, a head
    /// cap when one is set, and a warning when the heads' provider is not set up. Defaults
    /// say nothing: a pair with no effort chosen and no cap has one short line.
    private func summary(registry: ProviderRegistry) -> String {
        var parts = [HydraPairSummary.providers(pair)]
        if let efforts = HydraPairSummary.efforts(pair) { parts.append(efforts) }
        if let cap = pair.maxHeads {
            parts.append(cap == 1 ? "One head at a time" : "Up to \(cap) heads")
        }
        // Heads on a provider this Mac cannot run stay on the lead's, on the chat's own
        // model (see `AppModel.hydraHeadsProvider`); the row says so rather than promising
        // a team that would fail.
        if pair.sendsHeadsElsewhere, model.hydraHeadsProvider(of: pair) == pair.provider {
            parts.append("\(pair.headsProvider.displayName) is not set up, heads stay on \(pair.provider.displayName)")
        }
        return parts.joined(separator: " · ")
    }
}

/// A pair's provider marks: the lead's larger and the heads' smaller after an arrow,
/// the same mark wherever a pair is drawn, so a same-provider pair shows
/// provider → provider too and a row reads the same everywhere.
private struct HydraPairIcons: View {
    let pair: HydraPair

    var body: some View {
        HydraPairMark(lead: pair.provider, heads: pair.headsProvider)
        .frame(minWidth: 22, alignment: .leading)
    }
}

/// The pair's settings: provider, lead model and effort, the heads' provider, model and
/// effort, and how many heads go out at once. Every choice writes straight to the pair.
private struct HydraPairEditor: View {
    @Environment(AppModel.self) private var model
    let pairID: UUID

    var body: some View {
        if let pair = model.settings.hydraPair(pairID) {
            let registry = model.providers
            let providers = ProviderKind.allCases.filter { model.settings.isEnabled($0) && (registry.status($0).isInstalled || $0 == pair.provider) }
            let headsProviders = ProviderKind.allCases.filter { model.settings.isEnabled($0) && (registry.status($0).isInstalled || $0 == pair.headsProvider) }
            let options = registry.models(for: pair.provider)
            let headsOptions = registry.models(for: pair.headsProvider)
            let leadOption = registry.model(pair.orchestratorModel, for: pair.provider)
            let workerOption = registry.model(pair.workerModel, for: pair.headsProvider)
            // A lead that may be any model offers every effort the provider's models know.
            let leadEfforts = leadOption?.efforts ?? Self.allEfforts(options)
            // Heads with no model chosen inherit the chat's, and its efforts; on another
            // provider they run its default model, so its efforts are the ones on offer.
            let workerEfforts = workerOption?.efforts ?? (pair.sendsHeadsElsewhere ? (registry.defaultModel(for: pair.headsProvider)?.efforts ?? Self.allEfforts(headsOptions)) : leadEfforts)
            VStack(alignment: .leading, spacing: 0) {
                Text("Pair")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .padding(.horizontal, 14)
                    .padding(.top, 14)
                    .padding(.bottom, 6)
                editorRow("Lead provider") {
                    GlassPickerButton(
                        options: providers.map { ($0, $0.displayName) },
                        selection: Binding(
                            get: { pair.provider },
                            set: { provider in
                                model.updateHydraPair(pairID) {
                                    guard $0.provider != provider else { return }
                                    $0.provider = provider
                                    $0.orchestratorModel = nil
                                    $0.orchestratorEffort = nil
                                    // Heads on the old lead provider go with it; heads on a
                                    // provider of their own keep their model and effort, and
                                    // when that provider is the new lead's, they are simply
                                    // on the lead's provider again.
                                    if $0.workerProvider == nil {
                                        $0.workerModel = nil
                                        $0.workerEffort = nil
                                    } else if $0.workerProvider == provider {
                                        $0.workerProvider = nil
                                    }
                                }
                                Task { await registry.loadCatalog(provider) }
                            }
                        ),
                        asset: { $0.iconName }
                    )
                }
                Divider().padding(.horizontal, 14).padding(.vertical, 6)
                editorRow("Lead model", detail: "The chat's model while it leads") {
                    GlassPickerButton(
                        options: [(String?.none, "Any model")] + options.map { (Optional($0.id), $0.shortName) },
                        selection: Binding(
                            get: { pair.orchestratorModel },
                            set: { id in
                                model.updateHydraPair(pairID) {
                                    $0.orchestratorModel = id
                                    if let effort = $0.orchestratorEffort, let option = registry.model(id, for: $0.provider), !option.efforts.contains(effort) {
                                        $0.orchestratorEffort = nil
                                    }
                                }
                            }
                        ),
                        maxWidth: 190
                    )
                }
                if !leadEfforts.isEmpty {
                    editorRow("Lead effort") {
                        effortPicker(
                            efforts: leadEfforts,
                            inherit: "Chat's effort",
                            selection: Binding(
                                get: { pair.orchestratorEffort },
                                set: { effort in model.updateHydraPair(pairID) { $0.orchestratorEffort = effort } }
                            )
                        )
                    }
                }
                Divider().padding(.horizontal, 14).padding(.vertical, 6)
                editorRow("Heads' provider", detail: "Another provider puts quick heads under a strong lead") {
                    GlassPickerButton(
                        options: [(ProviderKind?.none, "Same as the lead")] + headsProviders.filter { $0 != pair.provider }.map { (Optional($0), $0.displayName) },
                        selection: Binding(
                            get: { pair.sendsHeadsElsewhere ? pair.workerProvider : nil },
                            set: { provider in
                                model.updateHydraPair(pairID) {
                                    let chosen = provider == $0.provider ? nil : provider
                                    guard $0.workerProvider != chosen else { return }
                                    // The heads' model and effort were the old provider's.
                                    $0.workerProvider = chosen
                                    $0.workerModel = nil
                                    $0.workerEffort = nil
                                }
                                if let provider { Task { await registry.loadCatalog(provider) } }
                            }
                        ),
                        asset: { ($0 ?? pair.provider).iconName }
                    )
                }
                editorRow("Heads' model", detail: pair.sendsHeadsElsewhere ? "What the heads run on, from \(pair.headsProvider.displayName)'s models" : "What the heads run on") {
                    GlassPickerButton(
                        options: [(String?.none, pair.sendsHeadsElsewhere ? "\(pair.headsProvider.displayName)'s default" : "Same as the chat")] + headsOptions.map { (Optional($0.id), $0.shortName) },
                        selection: Binding(
                            get: { pair.workerModel },
                            set: { id in
                                model.updateHydraPair(pairID) {
                                    $0.workerModel = id
                                    if let effort = $0.workerEffort, let option = registry.model(id, for: $0.headsProvider), !option.efforts.contains(effort) {
                                        $0.workerEffort = nil
                                    }
                                }
                            }
                        ),
                        maxWidth: 190
                    )
                }
                if !workerEfforts.isEmpty {
                    editorRow("Heads' effort", detail: "Lower effort keeps heads quick and cheap") {
                        effortPicker(
                            efforts: workerEfforts,
                            inherit: pair.sendsHeadsElsewhere ? "\(pair.headsProvider.displayName)'s default" : "Same as the chat",
                            selection: Binding(
                                get: { pair.workerEffort },
                                set: { effort in model.updateHydraPair(pairID) { $0.workerEffort = effort } }
                            )
                        )
                    }
                }
                Divider().padding(.horizontal, 14).padding(.vertical, 6)
                editorRow("Heads at once", detail: "How many work in parallel; with no cap, as many as the work takes") {
                    GlassPickerButton(
                        options: [(Int?.none, "No cap")] + HydraPair.maxHeadsRange.map { (Optional($0), String($0)) },
                        selection: Binding(
                            get: { pair.maxHeads },
                            set: { count in model.updateHydraPair(pairID) { $0.maxHeads = count } }
                        )
                    )
                }
                ForEach(pair.sendsHeadsElsewhere ? [pair.provider, pair.headsProvider] : [pair.provider], id: \.self) { provider in
                    if registry.models(for: provider).isEmpty {
                        Text(registry.loadingCatalogs.contains(provider) ? "Loading \(provider.displayName)'s models…" : "No models loaded for \(provider.displayName) yet.")
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(.horizontal, 14)
                            .padding(.top, 8)
                    }
                }
                Spacer(minLength: 12)
            }
            .padding(.bottom, 4)
            .task(id: [pair.provider, pair.headsProvider]) {
                await registry.loadCatalog(pair.provider)
                await registry.loadCatalog(pair.headsProvider)
            }
        }
    }

    private func editorRow<Control: View>(_ title: String, detail: String? = nil, @ViewBuilder control: () -> Control) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: title)
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let detail {
                    Text(verbatim: detail)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            control()
                .layoutPriority(1)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
    }

    private func effortPicker(efforts: [String], inherit: String, selection: Binding<String?>) -> some View {
        GlassPickerButton(
            options: [(String?.none, inherit)] + efforts.map { (Optional($0), ModelOption.effortTitle($0)) },
            selection: selection
        )
    }

    /// Every effort any of the provider's models offers, in the order the first lists them.
    private static func allEfforts(_ options: [ModelOption]) -> [String] {
        var seen: [String] = []
        for option in options {
            for effort in option.efforts where !seen.contains(effort) { seen.append(effort) }
        }
        return seen
    }
}
