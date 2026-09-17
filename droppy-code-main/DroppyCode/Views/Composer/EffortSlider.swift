import AppKit
import SwiftUI

/// The models the composer offers: the list chosen in Settings, or every ready provider's models until one is set up.
@MainActor
enum ModelCatalog {
    struct Entry: Identifiable, Hashable {
        let provider: ProviderKind
        let option: ModelOption

        var id: String { "\(provider.rawValue)/\(option.id)" }
    }

    static func entries(_ model: AppModel, including thread: ChatThread?) -> [Entry] {
        let registry = model.providers
        var result: [Entry] = if model.settings.modelList.isEmpty {
            registry.availableProviders.flatMap { provider in
                registry.models(for: provider).map { Entry(provider: provider, option: $0) }
            }
        } else {
            model.settings.modelList.compactMap { pin in
                registry.model(pin.modelID, for: pin.provider).map { Entry(provider: pin.provider, option: $0) }
            }
        }
        if let thread, let current = registry.model(thread.model, for: thread.provider),
           !result.contains(where: { $0.provider == thread.provider && $0.option.id == current.id }) {
            result.insert(Entry(provider: thread.provider, option: current), at: 0)
        }
        return result
    }
}

// MARK: - Composer chip

/// The composer's model chip. It opens the effort slider, and the slider's title opens the model list.
struct ModelEffortButton: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread
    let hasHistory: Bool
    /// Floating panels are narrow: the chip collapses to the provider's icon so the
    /// text field keeps its room. The full name stays in the tooltip and VoiceOver.
    var compact: Bool = false

    @State private var isPresented = false

    /// The chip's content width, in points, as it lays out on its own: a short name and
    /// effort make a narrow chip, a long one a wide chip. Read only while the chip is
    /// free to size itself, so the value is always the content's own width and never the
    /// locked box's.
    @State private var naturalWidth: CGFloat?
    /// The width the chip is held at while its popover is open. The popover hangs from
    /// the chip's center, so a chip that grew or shrank under it, as the slider changes
    /// the effort's title or the list picks a model with a longer name, would drag the
    /// popover along. Held at the width the chip had when it opened; a name that no
    /// longer fits is cut with an ellipsis until the popover closes and the chip is free
    /// again.
    @State private var lockedWidth: CGFloat?

    var body: some View {
        let registry = model.providers
        let current = registry.model(thread.model, for: thread.provider)
        // The pair this chat leads with, while Hydra is on: the chip wears its mark, so a
        // chat that leads a team says so without opening anything.
        let pair = model.hydraIsOn(thread) ? model.hydraPair(for: thread) : nil
        Button {
            isPresented.toggle()
        } label: {
            HStack(spacing: 5) {
                ProviderIcon(provider: thread.provider, size: 14)
                if pair != nil {
                    HydraMarkImage()
                        .foregroundStyle(Chrome.primaryText.opacity(0.85))
                        .frame(width: 12, height: 12)
                }
                if thread.fastMode, current?.supportsFast == true {
                    Image(systemName: "bolt.fill")
                        .font(Chrome.inlineIconFont)
                        .foregroundStyle(.yellow)
                }
                if !compact {
                    // The chip keeps the model and effort while its popover is open: they
                    // are what the popover edits, and a label that swapped for "Select
                    // effort" resized the chip under the popover's own arrow. The open
                    // state reads from the chip style's active look instead. The name is
                    // the one flexible part of the row, so inside the fixed width below it
                    // is what gets cut; the effort and the marks keep their size.
                    Text(verbatim: current?.chipName ?? thread.model ?? thread.provider.displayName)
                        .foregroundStyle(Chrome.primaryText)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    if let current, !current.efforts.isEmpty {
                        Text(verbatim: ModelOption.effortTitle(thread.effort ?? current.defaultEffort ?? ""))
                            .foregroundStyle(Chrome.primaryText.opacity(0.72))
                            .lineLimit(1)
                            .fixedSize()
                    }
                    Image(systemName: "chevron.down")
                        .font(Chrome.chevronFont)
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            // The chip takes its content's width, and holds it while the popover is open
            // so the popover hanging from its center never moves; the content stays
            // centered in the held box while the effort's title or the name changes.
            .frame(width: compact ? nil : lockedWidth)
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in
                if lockedWidth == nil { naturalWidth = width }
            }
        }
        .buttonStyle(.chip(active: isPresented))
        .focusable(false)
        .onChange(of: isPresented) { _, isOpen in
            lockedWidth = isOpen ? naturalWidth : nil
        }
        .help(helpText(current: current, pair: pair))
        .accessibilityLabel(Text(helpText(current: current, pair: pair)))
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            ModelEffortPanel(threadID: thread.id, hasHistory: hasHistory)
                .presentedChrome()
        }
        .background {
            // The website captures hang the slider and the switcher from the real chip: the
            // composer's, never a panel's icon-only one, which would leave the popover with
            // nothing to hang from once the panel is gone.
            if WebsiteCaptures.isEnabled, !compact {
                AttachmentAnchorCapture { WebsiteCaptures.modelChipAnchor = WeakView($0) }
            }
        }
        // And where the chip is, in the window, by the thread it belongs to: a popover
        // hung from that rect on the window itself outlives any remaking of the chip's view.
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
            if WebsiteCaptures.isEnabled, !compact { WebsiteCaptures.modelChipFrames[thread.id] = frame }
        }
        .task(id: thread.provider) { await registry.loadCatalog(thread.provider) }
    }

    /// The full model and effort, plus the pair the chat leads with, for the tooltip and
    /// VoiceOver when the chip shows only the icon.
    private func helpText(current: ModelOption?, pair: HydraPair?) -> String {
        var parts = [current?.shortName ?? thread.model ?? thread.provider.displayName]
        if let current, !current.efforts.isEmpty {
            parts.append(ModelOption.effortTitle(thread.effort ?? current.defaultEffort ?? ""))
        }
        if let pair { parts.append(HydraPairSummary.title(pair, registry: model.providers)) }
        return parts.joined(separator: " · ")
    }
}

/// The popover behind the chip: the slider, or the model list when its title is tapped.
private struct ModelEffortPanel: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let threadID: UUID
    let hasHistory: Bool

    @State private var showsModels = false

    var body: some View {
        if let thread = model.thread(threadID) {
            Group {
                if showsModels {
                    ModelList(thread: thread, hasHistory: hasHistory) { entry in
                        choose(entry, for: thread)
                        showsModels = false
                    } onEnterPair: { pair in
                        // Entering a pair settles the chat's model, effort and Hydra in one
                        // go, so there is nothing left to choose here: the popover closes.
                        // A tap on the pair the chat already leads with settles them again:
                        // it is the way back onto the pair's lead model and effort after the
                        // slider or a pair edit moved the chat off them.
                        model.enterHydraPair(pair, for: thread.id)
                        showsModels = false
                        dismiss()
                    } onBack: {
                        showsModels = false
                    }
                    .transition(.opacity)
                } else {
                    slider(for: thread)
                        .transition(.opacity)
                }
            }
            .frame(width: 330)
            .animation(.easeOut(duration: 0.12), value: showsModels)
        }
    }

    private func slider(for thread: ChatThread) -> some View {
        let option = model.providers.model(thread.model, for: thread.provider)
        // In a pair the slider is the lead's: it wears the pair's fusion, says whose effort
        // it sets, and resets to the pair's lead effort rather than the model's default.
        let pair = model.hydraIsOn(thread) ? model.hydraPair(for: thread) : nil
        let pairLook = pair.map { EffortPairLook($0, registry: model.providers, resolvedHeadsEffort: model.hydraLaunch(for: thread)?.workerEffort) }
        return EffortSliderCard(
            modelName: option?.shortName ?? thread.model ?? thread.provider.displayName,
            provider: thread.provider,
            pair: pairLook,
            efforts: option?.efforts ?? [],
            defaultEffort: option?.defaultEffort,
            supportsFast: option?.supportsFast ?? false,
            effort: Binding(
                get: { thread.effort },
                set: { effort in
                    model.updateThread(thread.id) { $0.effort = effort }
                    remember(thread.id)
                }
            ),
            fastMode: Binding(
                get: { thread.fastMode },
                set: { isOn in
                    model.updateThread(thread.id) { $0.fastMode = isOn }
                    remember(thread.id)
                }
            ),
            onTitleTap: { showsModels = true },
            onReset: {
                let leadEffort = pair?.orchestratorEffort.flatMap { option?.efforts.contains($0) == true ? $0 : nil }
                model.updateThread(thread.id) {
                    $0.effort = leadEffort
                    $0.fastMode = false
                }
                remember(thread.id)
            }
        )
    }

    /// A choice made in a chat becomes that model's setting for every future chat.
    private func remember(_ id: UUID) {
        guard let thread = model.thread(id), let modelID = thread.model else { return }
        model.settings.setPreference(ModelPreference(effort: thread.effort, fastMode: thread.fastMode), for: thread.provider, model: modelID)
        model.settings.remember(model: modelID, effort: thread.effort, for: thread.provider)
    }

    private func choose(_ entry: ModelCatalog.Entry, for thread: ChatThread) {
        let provider = entry.provider
        let option = entry.option
        let switchesProvider = provider != thread.provider
        let preference = model.settings.preference(for: provider, model: option.id)
        let effort = preference.effort.flatMap { option.efforts.contains($0) ? $0 : nil }
        model.updateThread(thread.id) { thread in
            if switchesProvider {
                thread.provider = provider
                thread.providerSessionID = nil
                thread.providerResumeAt = nil
            }
            thread.model = option.id
            thread.effort = effort
            thread.fastMode = option.supportsFast && preference.fastMode
        }
        // A model row is the choice of a model alone: the chat leaves any pair it led with.
        model.leaveHydraPair(for: thread.id)
        if switchesProvider { model.existingRuntime(for: thread.id)?.stopSession() }
        model.settings.remember(model: option.id, effort: effort, for: provider)
        model.settings.defaultProvider = provider
    }
}

struct ModelList: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread
    let hasHistory: Bool
    let onChoose: (ModelCatalog.Entry) -> Void
    /// Left out where the list is only shown, as in the website captures.
    var onEnterPair: (HydraPair) -> Void = { _ in }
    let onBack: () -> Void

    /// A pair row carries two lines, like a model row with its provider under it.
    private static let pairRowHeight: CGFloat = 46
    /// What `PopoverSectionHeader` takes: an 11 pt line with its own padding above and below.
    private static let sectionHeaderHeight: CGFloat = 22

    var body: some View {
        let entries = ModelCatalog.entries(model, including: thread)
        // The pairs are Hydra's: on offer while it is on, out of sight otherwise.
        let pairs = model.hydraIsOn(thread) ? model.hydraPickerPairs : []
        // A chat leads with one pair or runs one model, never both: the checkmark sits on
        // the pair while the chat is in one, even though the pair's lead model is a row too.
        let leadsWithPair = pairs.contains { model.leadsWithHydraPair($0, thread: thread) }
        let showsProviders = Set(entries.map(\.provider)).count > 1
        // Fixed row heights let the popover size itself in one pass instead of measuring and resizing.
        let rowHeight: CGFloat = showsProviders || (hasHistory && entries.contains { $0.provider != thread.provider }) ? 46 : 34
        // The pairs section is part of that one pass: its two headers and its rows are
        // counted here, so the popover opens at the size it keeps.
        let pairsHeight = pairs.isEmpty ? 0 : Self.sectionHeaderHeight * 2 + CGFloat(pairs.count) * (Self.pairRowHeight + 1)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 20, height: 20)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .foregroundStyle(Chrome.secondaryText)
                .help("Back")
                Text("Select model")
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.secondaryText)
                Spacer()
            }
            .padding(.horizontal, 8)
            .padding(.top, 8)
            .padding(.bottom, 4)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    if !pairs.isEmpty {
                        PopoverSectionHeader("Pairs")
                        ForEach(pairs) { pair in
                            let locked = hasHistory && pair.provider != thread.provider
                            HydraPairListRow(
                                pair: pair,
                                title: HydraPairSummary.title(pair, registry: model.providers),
                                detail: locked ? "New chats only" : detail(for: pair),
                                rowHeight: Self.pairRowHeight,
                                isSelected: model.leadsWithHydraPair(pair, thread: thread),
                                isEnabled: !locked
                            ) {
                                onEnterPair(pair)
                            }
                        }
                        PopoverSectionHeader("Models")
                    }
                    ForEach(entries) { entry in
                        let locked = hasHistory && entry.provider != thread.provider
                        ModelListRow(
                            entry: entry,
                            detail: locked ? "New chats only" : (showsProviders ? entry.provider.displayName : nil),
                            showsIcon: showsProviders,
                            rowHeight: rowHeight,
                            isSelected: !leadsWithPair && entry.provider == thread.provider && entry.option.id == thread.model,
                            isEnabled: !locked
                        ) {
                            onChoose(entry)
                        }
                    }
                    if entries.isEmpty {
                        Text("Loading models…")
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(10)
                    }
                }
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(height: min(pairsHeight + CGFloat(max(entries.count, 1)) * (rowHeight + 1), 440))
        }
        .padding(6)
        .task {
            // A pair on another provider, or with its heads on one, names its models from
            // that provider's catalogue, which this chat may never have loaded.
            for provider in Set(pairs.flatMap { [$0.provider, $0.headsProvider] }) where provider != thread.provider {
                await model.providers.loadCatalog(provider)
            }
        }
    }

    /// The line under a pair's title: the providers, and the efforts when any is set. The
    /// title already names the models, so this never repeats them.
    private func detail(for pair: HydraPair) -> String {
        [HydraPairSummary.providers(pair), HydraPairSummary.efforts(pair)].compactMap { $0 }.joined(separator: " · ")
    }
}

/// One Hydra pair in the model picker: the Hydra mark, who leads whom, and the provider
/// with the heads' model under it; a pair whose heads run on another provider shows both
/// providers' marks, lead then heads. Tapping it puts the chat in the pair.
private struct HydraPairListRow: View {
    let pair: HydraPair
    let title: String
    let detail: String
    let rowHeight: CGFloat
    let isSelected: Bool
    let isEnabled: Bool
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                HydraMarkImage()
                    .foregroundStyle(Chrome.primaryText)
                    .frame(width: 14, height: 14)
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: title)
                        .font(.system(size: 14))
                        .foregroundStyle(Chrome.primaryText)
                        .lineLimit(1)
                    Text(verbatim: detail)
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(1)
                }
                Spacer(minLength: 12)
                if pair.sendsHeadsElsewhere {
                    HydraPairMark(lead: pair.provider, heads: pair.headsProvider, leadSize: 13)
                    .foregroundStyle(Chrome.secondaryText)
                    .accessibilityHidden(true)
                }
                Image(systemName: "checkmark")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .opacity(isSelected ? 1 : 0)
            }
            .padding(.horizontal, 10)
            .frame(height: rowHeight)
            .background {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(isHovering && isEnabled ? Chrome.overlay(0.1) : Color.clear)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.45)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
    }
}

private struct ModelListRow: View {
    let entry: ModelCatalog.Entry
    let detail: String?
    let showsIcon: Bool
    let rowHeight: CGFloat
    let isSelected: Bool
    let isEnabled: Bool
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                if showsIcon {
                    ProviderIcon(provider: entry.provider, size: 14)
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 18)
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: entry.option.shortName)
                        .font(.system(size: 14))
                        .foregroundStyle(Chrome.primaryText)
                    if let detail {
                        Text(verbatim: detail)
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                    }
                }
                Spacer(minLength: 12)
                if entry.option.supportsFast {
                    Image(systemName: "bolt")
                        .font(.system(size: 10))
                        .foregroundStyle(Chrome.secondaryText)
                }
                Image(systemName: "checkmark")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .opacity(isSelected ? 1 : 0)
            }
            .padding(.horizontal, 10)
            .frame(height: rowHeight)
            .background {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(isHovering && isEnabled ? Chrome.overlay(0.1) : Color.clear)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.45)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
    }
}

// MARK: - Slider card

/// Fast mode, the effort title and model name, reset, and the effort slider.
/// What the slider shows while the chat leads a pair: the pair in words, the colours its
/// fusion runs between, and what the heads' effort is, for the line that says the slider
/// sets the lead's.
struct EffortPairLook: Hashable {
    /// "Fable + Opus (1M context)".
    var title: String
    var brand: EffortPairBrand
    /// The heads' effort by name, when the pair sets one.
    var headsEffort: String?

    @MainActor
    init(_ pair: HydraPair, registry: ProviderRegistry, resolvedHeadsEffort: String?) {
        title = HydraPairSummary.title(pair, registry: registry)
        brand = EffortPairBrand(pair)
        headsEffort = (pair.workerEffort ?? resolvedHeadsEffort).map { ModelOption.effortTitle($0).lowercased() }
    }

    /// Whose effort the slider sets, and what the heads run at.
    var caption: String {
        if let headsEffort { return "The lead's effort · heads run at \(headsEffort) effort" }
        return "The lead's effort · heads keep their own"
    }
}

struct EffortSliderCard: View {
    let modelName: String
    let provider: ProviderKind?
    /// The pair the chat leads with, if one: the card is the lead's then.
    var pair: EffortPairLook? = nil
    let efforts: [String]
    let defaultEffort: String?
    let supportsFast: Bool
    @Binding var effort: String?
    @Binding var fastMode: Bool
    var onTitleTap: (() -> Void)?
    let onReset: () -> Void

    @State private var isTitleHovering = false

    private var resolvedIndex: Int {
        if let effort, let index = efforts.firstIndex(of: effort) { return index }
        if let defaultEffort, let index = efforts.firstIndex(of: defaultEffort) { return index }
        return efforts.firstIndex(of: "medium") ?? 0
    }

    private var isMaxEffort: Bool {
        efforts.count > 1 && resolvedIndex == efforts.count - 1
    }

    private var title: String {
        guard !efforts.isEmpty else { return "Standard" }
        return ModelOption.effortTitle(efforts[resolvedIndex])
    }

    private var isFast: Bool { supportsFast && fastMode }

    private var brand: EffortBrand { EffortBrand(provider: provider) }

    /// The title takes the slider's colour: the provider's brand at maximum, gold in
    /// fast mode, and both blended when the two are on together. In a pair it runs from
    /// the lead's colour into the heads', like the track, gold at the end in fast mode.
    private var titleStyle: AnyShapeStyle {
        if let pair {
            var colors = [brand.titleColor] + pair.brand.titleColors
            if isFast { colors.append(EffortPalette.fast) }
            return AnyShapeStyle(LinearGradient(colors: colors, startPoint: .leading, endPoint: .trailing))
        }
        switch (isMaxEffort, isFast) {
        case (true, true): return AnyShapeStyle(LinearGradient(colors: [brand.titleColor, EffortPalette.fast], startPoint: .leading, endPoint: .trailing))
        case (true, false): return brand.title
        case (false, true): return AnyShapeStyle(EffortPalette.fast)
        case (false, false): return AnyShapeStyle(EffortPalette.title)
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            ZStack {
                HStack {
                    if supportsFast {
                        FastModeButton(isOn: $fastMode)
                    }
                    Spacer()
                    RoundIconButton(symbol: "arrow.counterclockwise", help: "Reset to default", action: onReset)
                }
                VStack(spacing: 2) {
                    Button {
                        onTitleTap?()
                    } label: {
                        HStack(spacing: 4) {
                            Text(verbatim: title)
                                .font(.system(size: 17, weight: .medium))
                                .contentTransition(.numericText())
                            if onTitleTap != nil {
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 12, weight: .semibold))
                                    .foregroundStyle(Chrome.secondaryText)
                            }
                        }
                        .foregroundStyle(titleStyle)
                        .animation(.smooth(duration: 0.3), value: isMaxEffort)
                        .animation(.smooth(duration: 0.3), value: isFast)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background {
                            RoundedRectangle(cornerRadius: 7, style: .continuous)
                                .fill(isTitleHovering && onTitleTap != nil ? Chrome.overlay(0.08) : Color.clear)
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .disabled(onTitleTap == nil)
                    .onHover { hovering in
                        withAnimation(Chrome.hover) { isTitleHovering = hovering }
                    }
                    .help(onTitleTap == nil ? "" : "Choose a model")

                    // In a pair the line under the effort is the pair itself, behind the
                    // Hydra mark: the slider is the lead's, and the lead is named first.
                    HStack(spacing: 5) {
                        if pair != nil {
                            HydraMarkImage()
                                .foregroundStyle(Chrome.primaryText.opacity(0.75))
                                .frame(width: 12, height: 12)
                        } else if let provider {
                            ProviderIcon(provider: provider, size: 12)
                        }
                        Text(verbatim: pair?.title ?? modelName)
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.primaryText.opacity(0.7))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .minimumScaleFactor(0.85)
                    }
                }
                .padding(.horizontal, 44)
            }

            if efforts.isEmpty {
                Text("This model has one reasoning level.")
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(height: EffortSlider.thumbSize)
            } else {
                EffortSlider(
                    count: efforts.count,
                    index: Binding(
                        get: { resolvedIndex },
                        set: { index in
                            withAnimation(.snappy(duration: 0.2)) { effort = efforts[index] }
                        }
                    ),
                    fastMode: isFast,
                    brand: brand,
                    pair: pair?.brand,
                    accessibilityTitle: title
                )
            }

            // Whose effort this is, so a pair's slider is never taken for the heads'.
            if let pair, !efforts.isEmpty {
                Text(verbatim: pair.caption)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .minimumScaleFactor(0.9)
                    .padding(.top, -6)
                    .transition(.softAppear)
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 16)
    }
}

private struct FastModeButton: View {
    @Binding var isOn: Bool
    @State private var isHovering = false

    var body: some View {
        Button {
            withAnimation(.spring(response: 0.3, dampingFraction: 0.7)) { isOn.toggle() }
        } label: {
            Image(systemName: isOn ? "bolt.fill" : "bolt")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(isOn ? Color.yellow : Chrome.secondaryText)
                .symbolEffect(.bounce, value: isOn)
                .frame(width: 30, height: 30)
                .background {
                    Circle().fill(isOn ? Color.yellow.opacity(0.18) : Chrome.overlay(isHovering ? 0.1 : 0.06))
                }
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .help(isOn ? "Fast mode is on" : "Fast mode")
        .accessibilityLabel(Text("Fast mode"))
        .accessibilityValue(Text(isOn ? "On" : "Off"))
    }
}

private struct RoundIconButton: View {
    let symbol: String
    let help: String
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Chrome.secondaryText)
                .frame(width: 30, height: 30)
                .background { Circle().fill(isHovering ? Chrome.overlay(0.1) : Color.clear) }
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .accessibilityLabel(Text(help))
    }
}

/// A thick capsule track with a dot per level, filled up to a large glass knob that snaps to each level.
struct EffortSlider: View {
    static let trackHeight: CGFloat = 34
    static let thumbSize: CGFloat = 42

    let count: Int
    @Binding var index: Int
    /// Fast mode on, for a model that has it. Colours the fill gold with
    /// speed streaks; at maximum effort it fuses with the brand's particles.
    var fastMode = false
    /// The colours maximum effort wears: the provider's brand.
    var brand: EffortBrand = .purple
    /// The pair the chat leads with, if one: the fill runs from the lead's colour into the
    /// heads' and the two fuse at a seam, whatever the effort.
    var pair: EffortPairBrand? = nil
    let accessibilityTitle: String

    @State private var dragX: CGFloat?

    private var look: TrackLook {
        let isMax = count > 1 && index == count - 1
        let kind: TrackLook.Kind = switch (isMax, fastMode) {
        case (true, true): .fusion
        case (true, false): .supercharged
        case (false, true): .fast
        case (false, false): .plain
        }
        return TrackLook(kind: kind, brand: brand, pair: pair)
    }

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            // The lens is wider than the track: stop centres sit a lens radius in
            // from each end so the whole knob stays visible at the extremes and
            // the fill ends flush with its far edge.
            let inset = Self.thumbSize / 2
            let midY = proxy.size.height / 2
            let step = count > 1 ? (width - inset * 2) / CGFloat(count - 1) : 0
            let restingX = inset + CGFloat(index) * step
            let x = min(max(dragX ?? restingX, inset), width - inset)
            let look = look

            ZStack(alignment: .leading) {
                Capsule(style: .continuous)
                    .fill(Chrome.overlay(0.1))
                    .frame(width: width, height: Self.trackHeight)
                Capsule(style: .continuous)
                    .fill(look.fill)
                    .frame(width: x + inset, height: Self.trackHeight)
                    .overlay(alignment: .leading) {
                        if look.kind != .plain || look.pair != nil {
                            TrackEffect(look: look)
                                .frame(width: x + inset, height: Self.trackHeight)
                                .clipShape(Capsule(style: .continuous))
                                .transition(.opacity)
                                .id(look.effectIdentity)
                        }
                    }
                ForEach(0..<count, id: \.self) { stop in
                    let stopX = inset + CGFloat(stop) * step
                    Circle()
                        .fill(stopX <= x ? look.stopColor : Chrome.overlay(0.32))
                        .frame(width: 5, height: 5)
                        .position(x: stopX, y: midY)
                }
                // The knob is a Liquid Glass lens: it refracts the fill's edge, the stops and
                // the particles as it slides over them, lit white just enough to read as the
                // knob on the dark track. The charged fills keep their glow under it: a blurred
                // disc beneath the lens rather than a shadow on it, because a shadow that
                // changes colour and radius on glass makes the lens flash as the look changes.
                ZStack {
                    Circle()
                        .fill(look.glow)
                        .blur(radius: look.kind == .plain ? 4 : 8)
                        .offset(y: 1)
                    Circle()
                        .fill(.clear)
                        .glassEffect(.regular.tint(Color.white.opacity(look.isLightFill ? 0.55 : 0.32)).interactive(), in: Circle())
                        // On a white fill the lens needs an edge to read against it.
                        .overlay {
                            Circle().strokeBorder(Color.black.opacity(look.isLightFill ? 0.14 : 0), lineWidth: 1)
                        }
                }
                .frame(width: Self.thumbSize, height: Self.thumbSize)
                .scaleEffect(dragX == nil ? 1 : 1.06)
                .position(x: x, y: midY)
            }
            .frame(width: width, height: proxy.size.height, alignment: .leading)
            .animation(.smooth(duration: 0.3), value: look)
            .contentShape(.rect)
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        if dragX == nil {
                            withAnimation(.spring(response: 0.22, dampingFraction: 0.8)) { dragX = value.location.x }
                        } else {
                            dragX = value.location.x
                        }
                        let nearest = nearestStop(to: value.location.x, inset: inset, step: step)
                        if nearest != index {
                            index = nearest
                            NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
                        }
                    }
                    .onEnded { _ in
                        withAnimation(.spring(response: 0.32, dampingFraction: 0.78)) { dragX = nil }
                    }
            )
        }
        .frame(height: Self.thumbSize)
        .accessibilityElement()
        .accessibilityLabel(Text(pair == nil ? "Reasoning effort" : "The lead's reasoning effort"))
        .accessibilityValue(Text(verbatim: accessibilityTitle))
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: index = min(count - 1, index + 1)
            case .decrement: index = max(0, index - 1)
            @unknown default: break
            }
        }
    }

    private func nearestStop(to x: CGFloat, inset: CGFloat, step: CGFloat) -> Int {
        guard count > 1, step > 0 else { return 0 }
        return min(count - 1, max(0, Int(((x - inset) / step).rounded())))
    }
}

enum EffortPalette {
    /// The accent lifted a little, so the effort name reads clearly on glass.
    static var title: Color { Chrome.accent.mix(with: .white, by: 0.22) }
    static let supercharged = Color(red: 0.74, green: 0.55, blue: 1.0)
    static let superchargedFill = Color(red: 0.55, green: 0.34, blue: 0.97)
    /// Fast mode's gold: the bolt's yellow, deepened for a fill.
    static let fast = Color(red: 1.0, green: 0.84, blue: 0.36)
    static let fastFill = Color(red: 0.93, green: 0.66, blue: 0.13)
}

/// The colours a provider's maximum effort wears: its brand. Claude's terracotta,
/// DeepSeek's blue, Meta's blue gradient, Google's Gemini gradient for Antigravity.
/// Brands whose marks are black and white (Codex, Cursor, Grok, OpenCode, Devin, Copilot, Command Code) get
/// a white fill with dark sparks, and everything in the track flips dark to read on it.
enum EffortBrand: Hashable {
    case claude
    case deepseek
    case meta
    case antigravity
    case silver
    /// The stock purple, for a slider with no provider behind it.
    case purple

    init(provider: ProviderKind?) {
        self = switch provider {
        case .claude: .claude
        case .deepseek: .deepseek
        case .meta: .meta
        case .antigravity: .antigravity
        case .codex, .cursor, .grok, .opencode, .devin, .copilot, .commandcode, .pi, .zai: .silver
        case nil: .purple
        }
    }

    /// The colour the effort name takes at maximum, also the near end of the fast blend.
    var titleColor: Color {
        switch self {
        case .claude: Color(red: 0.93, green: 0.58, blue: 0.44)
        case .deepseek: Color(red: 0.49, green: 0.58, blue: 1.0)
        case .meta: Color(red: 0.31, green: 0.64, blue: 1.0)
        case .antigravity: Color(red: 0.61, green: 0.45, blue: 0.80)
        case .silver: Chrome.primaryText
        case .purple: EffortPalette.supercharged
        }
    }

    var title: AnyShapeStyle {
        switch self {
        case .antigravity: AnyShapeStyle(LinearGradient(colors: Self.gemini, startPoint: .leading, endPoint: .trailing))
        default: AnyShapeStyle(titleColor)
        }
    }

    /// The fill behind the particles.
    var fill: AnyShapeStyle {
        switch self {
        case .claude: AnyShapeStyle(Color(red: 0.80, green: 0.42, blue: 0.28))
        case .deepseek: AnyShapeStyle(Color(red: 0.24, green: 0.36, blue: 0.98))
        case .meta: AnyShapeStyle(LinearGradient(
            colors: [Color(red: 0.0, green: 0.39, blue: 0.88), Color(red: 0.0, green: 0.51, blue: 0.98)],
            startPoint: .leading, endPoint: .trailing
        ))
        case .antigravity: AnyShapeStyle(LinearGradient(colors: Self.gemini, startPoint: .leading, endPoint: .trailing))
        case .silver: AnyShapeStyle(LinearGradient(
            colors: [Color.white, Color(red: 0.88, green: 0.88, blue: 0.91)],
            startPoint: .leading, endPoint: .trailing
        ))
        case .purple: AnyShapeStyle(EffortPalette.superchargedFill)
        }
    }

    /// The fill's darkest colour, for the fusion blend's near end and the knob's glow.
    var fillColor: Color {
        switch self {
        case .claude: Color(red: 0.80, green: 0.42, blue: 0.28)
        case .deepseek: Color(red: 0.24, green: 0.36, blue: 0.98)
        case .meta: Color(red: 0.0, green: 0.45, blue: 0.93)
        case .antigravity: Color(red: 0.56, green: 0.45, blue: 0.80)
        case .silver: Color(red: 0.92, green: 0.92, blue: 0.94)
        case .purple: EffortPalette.superchargedFill
        }
    }

    /// The particles and sheen: white on colour, dark on white.
    var spark: Color { self == .silver ? .black : .white }

    /// Whether the fill is light enough that the knob and stops must read dark against it.
    var isLight: Bool { self == .silver }

    var glow: Color { self == .silver ? .black.opacity(0.35) : fillColor.opacity(0.6) }

    /// Google's Gemini sweep: blue into violet into coral.
    static let gemini = [
        Color(red: 0.26, green: 0.52, blue: 0.96),
        Color(red: 0.61, green: 0.45, blue: 0.80),
        Color(red: 0.85, green: 0.40, blue: 0.44),
    ]
}

/// The colours a pair's fusion runs between: the lead's brand at the near end, and at the
/// far end the heads' provider's brand when they run on another provider than the lead
/// (Claude's terracotta into the Gemini sweep, say), else the roster's colours, for one
/// lead over many heads.
struct EffortPairBrand: Hashable {
    /// The heads' colours, near end first.
    var colors: [Color]
    /// Whether the far end is light, so the knob needs its edge there.
    var isLight: Bool

    init(_ pair: HydraPair) {
        if pair.sendsHeadsElsewhere {
            let brand = EffortBrand(provider: pair.headsProvider)
            switch brand {
            case .antigravity: colors = EffortBrand.gemini
            case .silver: colors = [Color(white: 0.8), Color(white: 0.95)]
            default: colors = [brand.fillColor, brand.titleColor]
            }
            isLight = brand.isLight
        } else {
            colors = HydraRoster.personas.prefix(4).map(\.color)
            isLight = false
        }
    }

    /// The heads' colours for the effort's name: lifted a little, like the lead's.
    var titleColors: [Color] { colors.map { $0.mix(with: .white, by: 0.18) } }
}

/// How the filled part of the effort track looks.
struct TrackLook: Hashable {
    enum Kind: Hashable {
        case plain
        /// Maximum effort: the brand's fill with particles drifting toward the knob.
        case supercharged
        /// Fast mode: gold with speed streaks and lightning.
        case fast
        /// Both: the brand running into gold, particles, streaks, lightning and a sweeping sheen.
        case fusion
    }

    var kind: Kind
    var brand: EffortBrand
    /// The pair the chat leads with: its colours take the fill over, whatever the kind, and
    /// the kind's own effects play on top.
    var pair: EffortPairBrand? = nil

    var fill: AnyShapeStyle {
        if let pair {
            return AnyShapeStyle(LinearGradient(colors: [brand.fillColor] + pair.colors, startPoint: .leading, endPoint: .trailing))
        }
        switch kind {
        case .plain: return AnyShapeStyle(Chrome.accent)
        case .supercharged: return brand.fill
        case .fast: return AnyShapeStyle(EffortPalette.fastFill)
        case .fusion: return AnyShapeStyle(LinearGradient(
            colors: [brand.fillColor, brand.fillColor, EffortPalette.fastFill],
            startPoint: .leading, endPoint: .trailing
        ))
        }
    }

    var glow: Color {
        if let pair, let last = pair.colors.last { return pair.isLight ? .black.opacity(0.35) : last.opacity(0.7) }
        switch kind {
        case .plain: return .black.opacity(0.28)
        case .supercharged: return brand.glow
        case .fast: return EffortPalette.fastFill.opacity(0.7)
        case .fusion: return EffortPalette.fast.opacity(0.75)
        }
    }

    /// Whether the fill under the knob is light (a white brand at maximum, not in fusion,
    /// whose far end is gold; or a pair whose heads' colours are white).
    var isLightFill: Bool { pair.map(\.isLight) ?? (kind == .supercharged && brand.isLight) }

    /// The passed stops: white on colour, dark on a white fill.
    var stopColor: Color { isLightFill ? .black.opacity(0.35) : .white.opacity(0.6) }

    /// The particles' and sheen's colour.
    var spark: Color { pair == nil && (kind == .supercharged || kind == .fusion) ? brand.spark : .white }

    /// What one run of the track's animation stands for. A look without a pair starts its
    /// effect over on every change, crossfading. A pair's fusion is the whole track's
    /// animation, so it keeps its time through the kind's changes: the seam does not jump
    /// because the knob reached maximum, and the kind's effects join it in place.
    var effectIdentity: AnyHashable {
        pair.map { AnyHashable(TrackLook(kind: .plain, brand: brand, pair: $0)) } ?? AnyHashable(self)
    }
}

/// The track's animation, one small Canvas drawn only while the slider needs it.
///
/// Maximum effort: particles drifting through the fill toward the knob, each with its own height,
/// size, speed and shimmer, fading at the ends. Fast mode: speed streaks zipping toward the knob
/// and a lightning bolt flashing now and then. Both together layer the two, over a fill that runs
/// from purple into gold, with a glossy sheen sweeping across. 30fps, 60 when streaks move.
///
/// Each particle's and streak's traits are fixed by its index, so they are worked out once
/// per fill width (`Traits`) and every frame is arithmetic and fills; six `sin` per element
/// per frame was most of what a frame cost.
struct TrackEffect: View {
    let look: TrackLook
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var startedAt = Date.now
    @State private var traits = TraitCache()

    var body: some View {
        let kind = look.kind
        let spark = look.spark
        let pair = look.pair
        let paused = reduceMotion || !FrontMonitor.shared.isVisible
        TimelineView(.animation(minimumInterval: 1.0 / (kind == .supercharged && pair == nil ? 30 : 60), paused: paused)) { timeline in
            let time = timeline.date.timeIntervalSince(startedAt)
            Canvas { context, size in
                guard size.width > 8 else { return }
                if let pair {
                    Self.drawFusion(context, size: size, time: time, lead: look.brand.fillColor, heads: pair.colors)
                }
                let traits = traits.traits(width: Double(size.width))
                if kind == .supercharged || kind == .fusion {
                    Self.drawParticles(context, size: size, time: time, color: spark, particles: traits.particles)
                }
                if kind == .fast || kind == .fusion {
                    Self.drawStreaks(context, size: size, time: time, streaks: traits.streaks)
                    Self.drawBolts(context, size: size, time: time)
                }
                if kind == .fusion {
                    Self.drawSheen(context, size: size, time: time, color: spark)
                }
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    struct Particle: Equatable {
        var height: Double
        var speed: Double
        var radius: Double
        /// Where along the span the particle starts, 0..<1.
        var start: Double
        var brightness: Double
        var shimmerRate: Double
    }

    struct Streak: Equatable {
        var length: Double
        var speed: Double
        var thickness: Double
        var start: Double
        var height: Double
        var brightness: Double
    }

    struct Traits: Equatable {
        var particles: [Particle]
        var streaks: [Streak]

        /// The traits for a fill of this width: as many particles as one per five points and
        /// streaks as one per fourteen, each element's traits the same at every width.
        static func build(width: Double) -> Traits {
            let particles = (0..<max(10, Int(width / 5))).map { index -> Particle in
                let seed = Double(index)
                return Particle(
                    height: random(seed, 1), speed: 12 + 28 * random(seed, 2), radius: 0.7 + 1.1 * random(seed, 3),
                    start: random(seed, 4), brightness: 0.22 + 0.5 * random(seed, 5), shimmerRate: 2 + 3 * random(seed, 6)
                )
            }
            let streaks = (0..<max(5, Int(width / 14))).map { index -> Streak in
                let seed = Double(index) + 100
                return Streak(
                    length: 10 + 20 * random(seed, 1), speed: 110 + 190 * random(seed, 2), thickness: 1.1 + 1.1 * random(seed, 3),
                    start: random(seed, 4), height: 0.2 + 0.6 * random(seed, 5), brightness: 0.35 + 0.55 * random(seed, 6)
                )
            }
            return Traits(particles: particles, streaks: streaks)
        }
    }

    /// The traits of the width last drawn. A reference the canvas closure updates in
    /// place: the width changes only while the knob is dragged, and a frame at the same
    /// width reuses what the last one built.
    final class TraitCache {
        private var width = -1.0
        private var built = Traits(particles: [], streaks: [])

        func traits(width: Double) -> Traits {
            if width != self.width {
                self.width = width
                built = Traits.build(width: width)
            }
            return built
        }
    }

    private static func drawParticles(_ context: GraphicsContext, size: CGSize, time: TimeInterval, color: Color, particles: [Particle]) {
        let span = Double(size.width) + 8
        for (index, particle) in particles.enumerated() {
            let x = (particle.start * span + time * particle.speed).truncatingRemainder(dividingBy: span) - 4
            let y = Double(size.height) * (0.16 + 0.68 * particle.height)
            let shimmer = 0.6 + 0.4 * sin(time * particle.shimmerRate + Double(index))
            let opacity = particle.brightness * shimmer * edgeFade(x, width: Double(size.width))
            guard opacity > 0.01 else { continue }
            let radius = particle.radius
            context.fill(
                Path(ellipseIn: CGRect(x: x - radius, y: y - radius, width: radius * 2, height: radius * 2)),
                with: .color(color.opacity(opacity))
            )
        }
    }

    /// Thin streaks racing toward the knob, bright at the head and fading down the tail.
    private static func drawStreaks(_ context: GraphicsContext, size: CGSize, time: TimeInterval, streaks: [Streak]) {
        let width = Double(size.width)
        for streak in streaks {
            let length = streak.length
            let thickness = streak.thickness
            let span = width + length + 12
            let x = (streak.start * span + time * streak.speed).truncatingRemainder(dividingBy: span) - length - 6
            let y = Double(size.height) * streak.height
            let brightness = streak.brightness * edgeFade(x + length, width: width)
            guard brightness > 0.02 else { continue }
            let rect = CGRect(x: x, y: y - thickness / 2, width: length, height: thickness)
            context.fill(
                Path(roundedRect: rect, cornerRadius: thickness / 2),
                with: .linearGradient(
                    Gradient(colors: [.white.opacity(0), .white.opacity(brightness)]),
                    startPoint: CGPoint(x: rect.minX, y: rect.midY),
                    endPoint: CGPoint(x: rect.maxX, y: rect.midY)
                )
            )
        }
    }

    /// Two lightning bolts on their own beats: each flashes for a quarter second every second
    /// or two, somewhere new along the fill, as a glowing zigzag.
    private static func drawBolts(_ context: GraphicsContext, size: CGSize, time: TimeInterval) {
        let width = Double(size.width)
        let height = Double(size.height)
        for slot in 0..<2 {
            let seed = Double(slot) + 200
            let period = 1.4 + 0.9 * random(seed, 1)
            let offset = random(seed, 2) * period
            let cycle = ((time + offset) / period).rounded(.down)
            let phase = (time + offset) - cycle * period
            let flash = 0.26
            guard phase < flash else { continue }
            let intensity = sin(phase / flash * .pi)
            let cycleSeed = seed + cycle * 7.31
            let x = 12 + (width - 24) * random(cycleSeed, 3)
            guard x > 6, x < width - 6 else { continue }
            let lean = 3.0 + 3.0 * random(cycleSeed, 4)
            var bolt = Path()
            bolt.move(to: CGPoint(x: x + lean, y: 3))
            bolt.addLine(to: CGPoint(x: x - lean * 0.4, y: height * 0.42))
            bolt.addLine(to: CGPoint(x: x + lean * 0.5, y: height * 0.5))
            bolt.addLine(to: CGPoint(x: x - lean, y: height - 3))
            var glow = context
            glow.addFilter(.blur(radius: 3))
            glow.stroke(bolt, with: .color(EffortPalette.fast.opacity(0.9 * intensity)), style: StrokeStyle(lineWidth: 4, lineCap: .round, lineJoin: .round))
            context.stroke(bolt, with: .color(.white.opacity(intensity)), style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
        }
    }

    /// A slanted band of light sweeping the whole fill every few seconds.
    private static func drawSheen(_ context: GraphicsContext, size: CGSize, time: TimeInterval, color: Color) {
        let width = Double(size.width)
        let height = Double(size.height)
        let band = 46.0
        let period = 2.8
        let progress = (time / period).truncatingRemainder(dividingBy: 1)
        let x = -band + (width + band * 2) * progress
        var path = Path()
        path.move(to: CGPoint(x: x + 10, y: 0))
        path.addLine(to: CGPoint(x: x + band + 10, y: 0))
        path.addLine(to: CGPoint(x: x + band, y: height))
        path.addLine(to: CGPoint(x: x, y: height))
        path.closeSubpath()
        context.fill(
            path,
            with: .linearGradient(
                Gradient(colors: [color.opacity(0), color.opacity(0.22), color.opacity(0)]),
                startPoint: CGPoint(x: x, y: 0),
                endPoint: CGPoint(x: x + band + 10, y: 0)
            )
        )
    }

    /// A pair's fusion: the lead's colour streaming in from the near end and the heads'
    /// from the far end, meeting at a white-hot seam that drifts and breathes. Bands of
    /// each colour flow on into the other's side and fade there, sparks fly off the seam
    /// to either side, cooling from white to the colour they land in, and the sheen sweeps
    /// over it all. The gradient itself is drawn here, over the fill, so the seam can move.
    private static func drawFusion(_ context: GraphicsContext, size: CGSize, time: TimeInterval, lead: Color, heads: [Color]) {
        let width = Double(size.width)
        let height = Double(size.height)
        let seam = width * (0.5 + 0.13 * sin(time * 0.7) + 0.05 * sin(time * 1.9 + 1))
        let pulse = 0.5 + 0.5 * sin(time * 2.4)
        let headsNear = heads.first ?? lead

        // The two colours meeting: each holds its own side and they fuse over a short run
        // around the seam, whose heart is white.
        var stops: [Gradient.Stop] = [.init(color: lead, location: 0)]
        stops.append(.init(color: lead, location: max(0, seam / width - 0.22)))
        stops.append(.init(color: .white.opacity(0.92), location: seam / width))
        let farStart = min(1, seam / width + 0.16)
        if heads.count == 1 {
            stops.append(.init(color: headsNear, location: farStart))
            stops.append(.init(color: headsNear, location: 1))
        } else {
            for (index, color) in heads.enumerated() {
                stops.append(.init(color: color, location: min(1, farStart + (1 - farStart) * Double(index) / Double(heads.count - 1))))
            }
        }
        context.fill(
            Path(CGRect(origin: .zero, size: size)),
            with: .linearGradient(Gradient(stops: stops), startPoint: CGPoint(x: 0, y: height / 2), endPoint: CGPoint(x: width, y: height / 2))
        )

        // Plasma: bands of the far side's colour flowing into each side, soft and additive,
        // brightest at the seam and gone by the fill's end, so each colour keeps reaching
        // into the other.
        for band in 0..<3 {
            let seed = Double(band) + 300
            let amplitude = height * (0.12 + 0.16 * random(seed, 1))
            let wavelength = 30 + 34 * random(seed, 2)
            let speed = 22 + 34 * random(seed, 3)
            let thickness = 1.8 + 2.2 * random(seed, 4)
            let phase = time * speed / wavelength * 2 * .pi
            for stream in 0..<2 {
                let toNear = stream == 0
                var path = Path()
                let steps = Int(width / 3) + 1
                for step in 0...steps {
                    let x = min(width, Double(step) * 3)
                    let y = height / 2 + amplitude * sin(x / wavelength * 2 * .pi + (toNear ? phase : -phase) + seed * (toNear ? 1 : 1.7))
                    if step == 0 { path.move(to: CGPoint(x: x, y: y)) } else { path.addLine(to: CGPoint(x: x, y: y)) }
                }
                var plasma = context
                plasma.blendMode = .plusLighter
                plasma.addFilter(.blur(radius: 2.5))
                let reach = width * 0.55
                let side = toNear ? CGRect(x: 0, y: 0, width: seam, height: height) : CGRect(x: seam, y: 0, width: width - seam, height: height)
                plasma.clip(to: Path(side))
                let color = toNear ? headsNear : lead
                let brightness = 0.32 + 0.16 * random(seed, 5)
                plasma.stroke(
                    path,
                    with: .linearGradient(
                        Gradient(colors: toNear ? [color.opacity(0), color.opacity(brightness)] : [color.opacity(brightness), color.opacity(0)]),
                        startPoint: CGPoint(x: toNear ? seam - reach : seam, y: 0),
                        endPoint: CGPoint(x: toNear ? seam : seam + reach, y: 0)
                    ),
                    style: StrokeStyle(lineWidth: thickness, lineCap: .round, lineJoin: .round)
                )
            }
        }

        // Sparks off the seam, flung to either side on beats of their own, white as they
        // leave and the colour of the side they land in as they cool.
        for index in 0..<22 {
            let seed = Double(index) + 400
            let life = 0.7 + 0.8 * random(seed, 1)
            let age = (time + random(seed, 2) * life).truncatingRemainder(dividingBy: life)
            let progress = age / life
            let toNear = random(seed, 3) < 0.5
            let speed = 28 + 74 * random(seed, 4)
            let x = seam + (toNear ? -1 : 1) * speed * age
            let y = height * (0.18 + 0.64 * random(seed, 5)) + 5 * sin(age * 8 + seed)
            let radius = 0.7 + 1.5 * (1 - progress)
            let opacity = (1 - progress) * (0.5 + 0.5 * random(seed, 6)) * edgeFade(x, width: width)
            guard opacity > 0.01 else { continue }
            let color = Color.white.mix(with: toNear ? lead : headsNear, by: progress * 0.8)
            context.fill(
                Path(ellipseIn: CGRect(x: x - radius, y: y - radius, width: radius * 2, height: radius * 2)),
                with: .color(color.opacity(opacity))
            )
        }

        // The seam: a white-hot core that breathes, its glow spilling to both sides.
        var glow = context
        glow.blendMode = .plusLighter
        glow.addFilter(.blur(radius: 7))
        let coreWidth = 9 + 8 * pulse
        glow.fill(
            Path(ellipseIn: CGRect(x: seam - coreWidth, y: -height * 0.2, width: coreWidth * 2, height: height * 1.4)),
            with: .color(.white.opacity(0.32 + 0.3 * pulse))
        )
        var core = context
        core.addFilter(.blur(radius: 0.8))
        core.fill(
            Path(roundedRect: CGRect(x: seam - 1.1, y: 2, width: 2.2, height: height - 4), cornerRadius: 1.1),
            with: .color(.white.opacity(0.5 + 0.4 * pulse))
        )

        drawSheen(context, size: size, time: time, color: .white)
    }

    /// Fades a point out over the last 14 points at either end of the fill.
    private static func edgeFade(_ x: Double, width: Double) -> Double {
        min(1, max(0, x / 14), max(0, (width - x) / 14))
    }

    /// A stable pseudo-random value in 0..<1 for an element and one of its traits.
    nonisolated static func random(_ index: Double, _ trait: Double) -> Double {
        let value = sin(index * 12.9898 + trait * 78.233) * 43758.5453
        return value - value.rounded(.down)
    }
}
