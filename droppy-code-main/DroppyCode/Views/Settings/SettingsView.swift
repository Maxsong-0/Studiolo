import AppKit
import SwiftUI

enum SettingsPage: String, CaseIterable, Identifiable {
    case general
    // Second in the sidebar: Hydra is what the app is about, so it sits right under General.
    case hydra
    case providers
    case models
    case sourceControl
    case shortcuts
    case archive
    case about

    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: "General"
        case .providers: "Providers"
        case .models: "Models"
        case .hydra: "Hydra"
        case .sourceControl: "Source control"
        case .shortcuts: "Shortcuts"
        case .archive: "Archive"
        case .about: "About"
        }
    }

    var symbol: String {
        switch self {
        case .general: "gear"
        case .providers: "cpu"
        case .models: "slider.horizontal.3"
        // Never drawn: the sidebar gives the Hydra row the dragon mark instead. Kept so
        // the switch stays exhaustive and anything else that asks gets a sane symbol.
        case .hydra: "point.3.connected.trianglepath.dotted"
        case .sourceControl: "arrow.triangle.branch"
        case .shortcuts: "command"
        case .archive: "archivebox"
        case .about: "info.circle"
        }
    }

    var tint: Color {
        switch self {
        case .general: Chrome.gray
        case .providers: Chrome.blue
        case .models: Color(red: 0.686, green: 0.322, blue: 0.871)
        case .hydra: Color(red: 0.188, green: 0.690, blue: 0.780)
        case .sourceControl: Color(red: 0.345, green: 0.337, blue: 0.839)
        case .shortcuts: Color(red: 0.32, green: 0.48, blue: 0.93)
        case .archive: Chrome.orange
        case .about: Color(red: 0.204, green: 0.780, blue: 0.349)
        }
    }

    var keywords: [String] {
        switch self {
        case .general: ["permissions", "worktree", "reasoning", "thinking", "notifications", "theme", "appearance", "transparency", "transparent", "opacity", "glass", "dark", "light", "accent", "tint", "catppuccin", "dracula", "tokyo", "nord", "gruvbox", "solarized", "github", "claude", "codex", "cursor", "matrix", "token", "tokens", "activity", "usage", "panel", "limits", "credits", "text", "size", "font", "zoom", "heatmap", "daily", "weekly", "cumulative", "settle", "settled", "finish", "finished", "done", "sound", "chime"]
        case .models: ["model", "effort", "reasoning", "fast", "slider", "picker"]
        case .hydra: ["hydra", "heads", "subagents", "sub-agents", "agents", "team", "orchestrator", "worker", "pair", "pairs", "parallel", "delegate", "queue"]
        case .providers: ["codex", "claude", "cursor", "opencode", "grok", "deepseek", "meta", "muse", "spark", "zai", "z.ai", "glm", "coding plan", "devin", "cognition", "antigravity", "agy", "google", "gemini", "copilot", "github", "command code", "commandcode", "cmd", "binary", "path", "sign in", "login", "api key", "usage", "limits", "limit", "plan", "quota", "credits", "balance"]
        case .sourceControl: ["git", "commit", "pull request", "titles", "text generation"]
        case .shortcuts: ["keyboard", "keys"]
        case .archive: ["archived", "restore"]
        case .about: ["version", "license"]
        }
    }
}

/// A page asked for from outside the window: the update notification and the relaunch after
/// an install both open About. Read once by the view, then cleared.
@MainActor
@Observable
final class SettingsNavigation {
    static let shared = SettingsNavigation()
    var requestedPage: SettingsPage?
}

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var page: SettingsPage = .general
    @State private var search = ""
    @State private var modelSearch = ""
    @State private var scrollChrome = ChromeScrollModel()

    private var visiblePages: [SettingsPage] {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return SettingsPage.allCases }
        return SettingsPage.allCases.filter { page in
            page.title.localizedCaseInsensitiveContains(query)
                || page.keywords.contains { $0.localizedCaseInsensitiveContains(query) }
        }
    }

    var body: some View {
        HStack(spacing: 0) {
            sidebar
                .frame(width: 212)
            detail
                .padding(.trailing, Chrome.sheetInset)
                .padding(.vertical, Chrome.sheetInset)
        }
        .background { WindowBackdrop() }
        .background { WindowChromeConfigurator() }
        .clipShape(RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous))
        .ignoresSafeArea()
        .frame(width: 780, height: 580)
        .onChange(of: SettingsNavigation.shared.requestedPage, initial: true) { _, requested in
            guard let requested else { return }
            page = requested
            SettingsNavigation.shared.requestedPage = nil
        }
        // A search that filters the selected page out of the list left the detail pane on a
        // page no row was selected for; the first match takes over instead.
        .onChange(of: visiblePages) { _, pages in
            guard !pages.isEmpty, !pages.contains(page), let first = pages.first else { return }
            page = first
        }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            Color.clear
                .frame(height: Chrome.trafficLightDiameter)
                .padding(.top, Chrome.trafficLightTop)
            SidebarSearchField(text: $search) {
                if let first = visiblePages.first { page = first }
            }
            .padding(.horizontal, Chrome.listInset)
            .padding(.top, 14)
            ScrollView {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(visiblePages) { item in
                        SidebarRow(title: item.title, isSelected: page == item, action: { page = item }) {
                            SidebarIconBadge {
                                if item == .hydra {
                                    // The Hydra page wears the dragon mark, the same one as the composer button.
                                    HydraMarkImage()
                                        .frame(width: Chrome.symbolSize * 1.15, height: Chrome.symbolSize * 1.15)
                                } else {
                                    SidebarSymbol(item.symbol, scale: item == .general ? 1.15 : 1)
                                }
                            }
                            .overlay(alignment: .topTrailing) {
                                if item == .about, UpdateChecker.shared.updateAvailable {
                                    UpdateAvailableDot()
                                }
                            }
                        }
                    }
                    if visiblePages.isEmpty {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("No results")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(Chrome.primaryText.opacity(0.92))
                            Text("Try a setting or a provider name.")
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                        }
                        .padding(.horizontal, 8)
                        .padding(.top, 14)
                    }
                }
                .padding(.horizontal, Chrome.listInset)
                .padding(.top, 12)
                .padding(.bottom, 8)
            }
            .scrollIndicators(.never)
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .background { WindowDragArea() }
    }

    private var detail: some View {
        Group {
            if page == .archive, model.archivedThreads.isEmpty {
                // An empty archive is one symbol in the middle of the pane: no sentence, no chrome,
                // no button with nothing to delete.
                Image(systemName: "archivebox")
                    .font(.system(size: 44, weight: .light))
                    .foregroundStyle(Chrome.secondaryText.opacity(0.6))
                    .accessibilityLabel(Text("No archived threads"))
            } else {
                pageScroll
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .detailSheet()
        .onChange(of: page, initial: true) {
            scrollChrome.update(travel: 0)
            modelSearch = ""
        }
    }

    private var pageScroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
                pageContent
            }
            .padding(.horizontal, Chrome.contentHorizontalPadding + 8)
            // Pages without chrome controls start just below the sheet edge; the others clear their capsules.
            .padding(.top, pageHasChromeControls ? Chrome.chromeTopPadding + Chrome.capsuleHeight + 16 : 22)
            .padding(.bottom, 24)
        }
        .scrollIndicators(.never)
        .id(page)
        .onScrollGeometryChange(for: CGFloat.self, of: Self.travel) { _, travel in
            scrollChrome.update(travel: travel)
        }
        .overlay(alignment: .top) {
            PaneTopVeil(model: scrollChrome)
        }
        .overlay(alignment: .top) {
            HStack(spacing: 10) {
                ChromeCompactTitle(title: page.title, model: scrollChrome)
                if page == .models {
                    ChromeSearchField(query: $modelSearch, prompt: "Search models")
                }
                if page == .providers {
                    ProvidersRefreshButton()
                }
                if page == .archive {
                    ArchiveDeleteAllButton()
                }
                if page == .about {
                    AboutUpdateChromeAccessory()
                }
            }
            .frame(minHeight: Chrome.capsuleHeight)
            .padding(.horizontal, Chrome.chromeHorizontalPadding)
            .padding(.top, Chrome.chromeTopPadding)
        }
    }

    private var pageHasChromeControls: Bool {
        page == .providers || page == .models || page == .archive || page == .about
    }

    @ViewBuilder
    private var pageContent: some View {
        switch page {
        case .general: GeneralSettingsPage()
        case .providers: ProvidersSettingsPage()
        case .models:
            ModelsSettingsPage(query: modelSearch)
        case .hydra: HydraSettingsPage()
        case .sourceControl: SourceControlSettingsPage()
        case .shortcuts: ShortcutsSettingsPage()
        case .archive: ArchiveSettingsPage()
        case .about: AboutSettingsPage()
        }
    }

    private nonisolated static func travel(_ geometry: ScrollGeometry) -> CGFloat {
        geometry.contentOffset.y + geometry.contentInsets.top
    }
}

struct SettingsSwitch: View {
    @Binding var isOn: Bool

    var body: some View {
        Toggle("", isOn: $isOn)
            .labelsHidden()
            .toggleStyle(.switch)
            .controlSize(.small)
    }
}

private struct GeneralSettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        // The Token activity heatmap leads the page: it summarizes what the
        // settings below configure, rather than configuring anything itself.
        TokenActivitySection()
        ChromeSection(title: "New threads") {
            ChromeCard {
                // No picker: a new thread follows the one it is made from, and the composer's
                // model picker is where the provider is chosen (see `AppModel.newThread`).
                ChromeRow(title: "Provider and model", detail: "A new thread follows the one you are in: its provider, model, effort and Hydra pair. With no thread open it starts on the provider you last chose.") {
                    EmptyView()
                }
                ChromeRowDivider()
                ChromeRow(title: "Permissions", detail: settings.defaultRuntimeMode.summary) {
                    GlassPickerButton(options: RuntimeMode.allCases.map { ($0, $0.title) }, selection: $settings.defaultRuntimeMode, symbol: { $0.symbol })
                }
                ChromeRowDivider()
                ChromeRow(title: "Workspace", detail: "Where a new thread makes its changes") {
                    GlassPickerButton(options: WorkspaceMode.allCases.map { ($0, $0.title) }, selection: $settings.defaultWorkspaceMode)
                }
            }
        }
            ChromeSection(title: "Conversation") {
                ChromeCard {
                    ChromeRow(title: "Show thinking", detail: "The working line opens to the agent's thinking") {
                        SettingsSwitch(isOn: $settings.showReasoning)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Show activity while working", detail: "The working line is a card with the turn's progress bar; off, a plain line") {
                        SettingsSwitch(isOn: $settings.showsWorkingCard)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Recent downloads", detail: "The attach button offers recent downloads first") {
                        SettingsSwitch(isOn: $settings.recentDownloadsPicker)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Notify when a turn finishes") {
                    SettingsSwitch(isOn: $settings.notifyWhenFinished)
                }
                ChromeRowDivider()
                ChromeRow(title: "Chime when a turn finishes", detail: "A soft chime, whether or not the thread is in view") {
                    SettingsSwitch(isOn: $settings.chimeWhenFinished)
                }
                // Switching it on plays the chime once, so you hear what you are getting.
                .onChange(of: settings.chimeWhenFinished) { _, isOn in
                    if isOn { FinishChime.play() }
                }
                ChromeRowDivider()
                ChromeRow(title: "Confirm before deleting threads") {
                    SettingsSwitch(isOn: $settings.confirmBeforeDeleting)
                }
                ChromeRowDivider()
                ChromeRow(title: "Continue after a usage limit", detail: "When the provider's limit is spent, the chat waits for the reset and then tells the agent to carry on") {
                    SettingsSwitch(isOn: $settings.autoContinueAfterLimit)
                }
                ChromeRowDivider()
                ChromeRow(
                    title: "Show a usage panel",
                    detail: "The plan's limits and credits in a floating panel beside every chat, for the model in use or a pair's two; the usage popover's pop-out button switches it on as well, and it keeps the corner it was last dragged to"
                ) {
                    SettingsSwitch(isOn: $settings.showsUsagePanel)
                }
            }
        }
        ChromeSection(title: "Projects") {
            ChromeCard {
                ChromeRow(title: "Projects", detail: "Activates every project at once unless explicitly overridden") {
                    SettingsSwitch(isOn: $settings.projectsEnabled)
                }
            }
        }
        ChromeSection(title: "Finished threads") {
            ChromeCard {
                ChromeRow(title: "Done with a thread", detail: settings.threadFinishAction.detail) {
                    GlassPickerButton(
                        options: ThreadFinishAction.allCases.map { ($0, $0.title) },
                        selection: $settings.threadFinishAction
                    )
                }
                ChromeRowDivider()
                ChromeRow(title: "Sound when settling", detail: "A small note as the thread settles") {
                    SettingsSwitch(isOn: $settings.settleSound)
                }
                // Switching it on plays the note once, so you hear what you are getting.
                .onChange(of: settings.settleSound) { _, isOn in
                    if isOn { SettleChime.play() }
                }
                .disabled(settings.threadFinishAction != .settle)
                .opacity(settings.threadFinishAction == .settle ? 1 : 0.5)
            }
        }
        ChromeSection(title: "Appearance") {
            ChromeCard {
                ChromeRow(title: "Theme", detail: settings.theme.detail) {
                    ThemePickerButton(selection: $settings.theme)
                }
                ChromeRowDivider()
                ChromeRow(title: "Transparency", detail: "How much shows through the window") {
                    BackdropOpacitySlider(value: $settings.backdropOpacity)
                }
                ChromeRowDivider()
                ChromeRow(title: "Open the sidebar at launch", detail: "Collapsed otherwise; the list button in the toolbar shows and hides it any time") {
                    SettingsSwitch(isOn: Binding(
                        get: { model.sidebar.startsOpen },
                        set: { model.sidebar.startsOpen = $0 }
                    ))
                }
                ChromeRowDivider()
                ChromeRow(title: "Float the sidebar over the chat", detail: "With the column collapsed, the thread list floats as a panel over the chat instead of opening from the toolbar button, and stays where you leave it") {
                    SettingsSwitch(isOn: $settings.sidebarFloats)
                }
                ChromeRowDivider()
                ChromeRow(title: "Only the floating panel", detail: "No column and no toolbar button; the panel is the sidebar") {
                    SettingsSwitch(isOn: $settings.sidebarOnlyFloats)
                        .disabled(!settings.sidebarFloats)
                }
            }
            ChatTextSizeCard(index: $settings.chatZoom)
        }
    }
}

/// The conversation's text size: a sample of a chat drawn at the chosen size, so the
/// change is seen here before any thread is opened, over the row that sets it.
private struct ChatTextSizeCard: View {
    @Binding var index: Int

    var body: some View {
        let zoom = ChatZoom.scale(at: index)
        let percentage = ChatZoom.percentage(at: index)
        ChromeCard {
            // The sample: a message in its bubble and the start of a reply, in the
            // conversation's own type at the chosen size.
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Spacer(minLength: 48)
                    Text("Where does the composer decide how far the draft photo sits from the edges?")
                        .font(.chat(.body, zoom: zoom))
                        .foregroundStyle(Chrome.primaryText)
                        .padding(.leading, 14)
                        .padding(.trailing, 14 + UserBubble.tail)
                        .padding(.vertical, 9)
                        .background(.tint.opacity(0.14), in: UserBubble())
                }
                Text("In `DraftAttachments`, inside **ComposerView.swift**. The strip pads its leading edge with `attachmentInset` but leaves the top on the stack's default spacing, which is why the photo hugs the top of the pill.")
                    .font(.chat(.body, zoom: zoom))
                    .foregroundStyle(Chrome.primaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .animation(.snappy(duration: 0.2), value: index)
            .accessibilityHidden(true)
            ChromeRowDivider()
            ChromeRow(title: "Text size", detail: percentage == 100 ? "The stock size" : "\(percentage)% of the stock size") {
                ChatTextSizeSlider(index: $index)
            }
        }
    }
}

/// Smaller type on the left, larger on the right, the stock size in the middle, with a
/// reset that shows only once the slider has left it (see `BackdropOpacitySlider`).
private struct ChatTextSizeSlider: View {
    @Binding var index: Int

    private var isOffStock: Bool { index != ChatZoom.defaultIndex }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "textformat.size.smaller")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .help("Smaller")
            Slider(
                value: Binding(
                    get: { Double(index) },
                    set: { index = ChatZoom.clamped(Int($0.rounded())) }
                ),
                in: 0...Double(ChatZoom.stepCount - 1),
                step: 1
            )
            .controlSize(.small)
            .frame(width: 140)
            Image(systemName: "textformat.size.larger")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .help("Larger")
        }
        .overlay(alignment: .leading) {
            if isOffStock {
                Button {
                    withAnimation(.snappy(duration: 0.2)) { index = ChatZoom.defaultIndex }
                } label: {
                    Image(systemName: "arrow.counterclockwise")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 22, height: 22)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Back to the stock size")
                .offset(x: -28)
                .transition(.opacity.combined(with: .offset(x: 6)))
            }
        }
        .animation(.snappy(duration: 0.2), value: isOffStock)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Text size"))
        .accessibilityValue(Text("\(ChatZoom.percentage(at: index)) percent"))
    }
}

/// Clear glass on the left, a solid base on the right, the stock look in the middle,
/// with a reset that shows only once the slider has left the stock look.
private struct BackdropOpacitySlider: View {
    @Binding var value: Double

    private var isOffStock: Bool { abs(value - AppSettings.defaultBackdropOpacity) >= 0.005 }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "circle.dotted")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .help("Clear")
            Slider(value: $value, in: 0...1)
                .controlSize(.small)
                .frame(width: 140)
            Image(systemName: "circle.fill")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .help("Solid")
        }
        // The reset takes no room in the row, so the slider ends flush with the card's
        // other controls whether or not it shows; it fades in to the slider's left once
        // the value has left the stock look.
        .overlay(alignment: .leading) {
            if isOffStock {
                Button {
                    withAnimation(.snappy(duration: 0.2)) { value = AppSettings.defaultBackdropOpacity }
                } label: {
                    Image(systemName: "arrow.counterclockwise")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 22, height: 22)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Back to the stock look")
                .offset(x: -28)
                .transition(.opacity.combined(with: .offset(x: 6)))
            }
        }
        .animation(.snappy(duration: 0.2), value: isOffStock)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Window transparency"))
    }
}

private struct ProvidersRefreshButton: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ChromeCapsule {
            ChromeIconButton(
                symbol: "arrow.clockwise",
                isEnabled: !model.providers.isRefreshing,
                help: model.providers.isRefreshing ? "Checking providers…" : "Check providers again"
            ) {
                Task { await model.providers.refreshEverything() }
            }
        }
    }
}

/// Shows what launch (or the refresh button) last found: opening or scrolling the page checks no provider.
/// The one read it starts on its own is each signed-in account's usage limits, which the registry
/// throttles to once a minute, so the numbers are on screen without a trip through a chat.
private struct ProvidersSettingsPage: View {
    var body: some View {
        // Eager on purpose: a handful of sections, and lazy ones were rebuilt while scrolling,
        // which reset their fields and re-ran every check they started on appear.
        VStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
            ForEach(ProviderKind.allCases) { provider in
                ProviderSettingsSection(provider: provider)
            }
        }
    }
}

private struct ProviderSettingsSection: View {
    @Environment(AppModel.self) private var model
    let provider: ProviderKind

    @State private var binaryPath = ""
    @State private var apiKey = ""
    @State private var showsAPIKey = false
    @State private var keyCheck: Task<Void, Never>?

    /// Limits and credits belong to a signed-in account, so a missing or signed-out provider shows none.
    private var showsUsage: Bool {
        guard PlanLimitsReader.exposesLimits(provider) || CreditsReader.exposesCredits(provider) else { return false }
        let status = model.providers.status(provider)
        return status.isInstalled && status.auth != .signedOut
    }

    var body: some View {
        let status = model.providers.status(provider)
        ChromeSection(title: provider.displayName) {
            ChromeCard {
                HStack(spacing: 12) {
                    ProviderIcon(provider: provider, size: 18)
                        .foregroundStyle(Chrome.primaryText)
                        .frame(width: 22)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: status.isChecking ? "Checking…" : status.summary)
                            .font(.system(size: 13))
                        if let version = status.version, !provider.isAPIKeyBased {
                            Text(verbatim: "Version \(version)")
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                        } else if provider.isAPIKeyBased, status.apiKeyConfigured, let host = provider.apiHost {
                            Text(verbatim: "Native API · \(host)")
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                        }
                    }
                    Spacer()
                    SettingsSwitch(isOn: Binding(
                        get: { model.settings.isEnabled(provider) },
                        set: { model.settings.setEnabled($0, for: provider) }
                    ))
                }
                .padding(.leading, 16)
                .padding(.trailing, Chrome.rowControlTrailingPadding)
                .padding(.vertical, 11)
                if showsUsage {
                    ChromeRowDivider()
                    VStack(alignment: .leading, spacing: 14) {
                        if PlanLimitsReader.exposesLimits(provider) {
                            PlanLimitsView(provider: provider)
                        }
                        if CreditsReader.exposesCredits(provider) {
                            CreditsView(provider: provider)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                }
                if provider.acceptsAPIKey {
                    ChromeRowDivider()
                    ChromeRow(
                        title: provider.isAPIKeyBased ? "API key" : "API key (optional)",
                        detail: provider.isAPIKeyBased
                            ? "Stored in your Keychain. Get one at \(provider.apiKeySource ?? "the provider dashboard")."
                            : "Instead of `\(provider.loginCommand)`: a Studio key from \(provider.apiKeySource ?? "the provider dashboard"), stored in your Keychain and handed to the CLI."
                    ) {
                        HStack(spacing: 8) {
                            Group {
                                if showsAPIKey {
                                    TextField("", text: $apiKey, prompt: Text("sk-…"))
                                } else {
                                    SecureField("", text: $apiKey, prompt: Text("sk-…"))
                                }
                            }
                            .textFieldStyle(.roundedBorder)
                            .font(.system(size: 12, design: .monospaced))
                            .frame(width: 200)
                            Button {
                                showsAPIKey.toggle()
                            } label: {
                                Image(systemName: showsAPIKey ? "eye.slash" : "eye")
                                    .font(.system(size: 12))
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(Chrome.secondaryText)
                            .help(showsAPIKey ? "Hide the API key" : "Show the API key")
                        }
                    }
                    if provider.isAPIKeyBased, !status.isInstalled {
                        ChromeRowDivider()
                        ChromeRow(title: "Get a key", detail: "\(provider.displayName) needs an API key. Nothing to install.") {
                            Link("Get a \(provider.displayName) key", destination: provider.installURL)
                                .buttonStyle(.glass)
                        }
                    }
                }
                if !provider.isAPIKeyBased {
                    ChromeRowDivider()
                    ChromeRow(title: "Binary path", detail: status.executable?.path) {
                        TextField("", text: $binaryPath, prompt: Text(provider.executableName))
                            .textFieldStyle(.roundedBorder)
                            .font(.system(size: 12, design: .monospaced))
                            .frame(width: 200)
                    }
                    if !status.isInstalled {
                        ChromeRowDivider()
                        ChromeRow(title: "Install", detail: "\(provider.displayName) was not found on your PATH.") {
                            Link("Get \(provider.displayName)", destination: provider.installURL)
                                .buttonStyle(.glass)
                        }
                    } else if status.auth == .signedOut {
                        ChromeRowDivider()
                        ChromeRow(title: "Sign in", detail: "Run this command in Terminal.") {
                            CopyCommandButton(command: provider.loginCommand)
                        }
                    }
                }
            }
        }
        .onAppear {
            binaryPath = model.settings.binaryPath(for: provider)
            if provider.acceptsAPIKey { apiKey = model.settings.apiKeyInput(for: provider) }
        }
        .onChange(of: showsUsage, initial: true) { _, shows in
            // Fires once when the page opens on a signed-in account and again when a check signs
            // one in. Sections are eager, so scrolling never re-fires it, and the registry hands
            // back what it read within the last minute instead of asking the provider again.
            guard shows else { return }
            model.providers.refreshPlanLimits(provider)
            model.providers.refreshCredits(provider)
        }
        .onChange(of: binaryPath) { _, value in
            guard value != model.settings.binaryPath(for: provider) else { return }
            model.settings.setBinaryPath(value, for: provider)
        }
        .onChange(of: apiKey) { _, value in
            // Only an edit counts: loading the stored key into the field must not save or check it.
            guard provider.acceptsAPIKey, value != model.settings.apiKeyInput(for: provider) else { return }
            model.settings.setAPIKeyInput(value, for: provider)
            keyCheck?.cancel()
            keyCheck = Task {
                try? await Task.sleep(for: .milliseconds(600))
                guard !Task.isCancelled else { return }
                await model.providers.refresh(provider)
                await model.providers.loadCatalog(provider, force: true)
            }
        }
        .onChange(of: model.settings.apiKeyInput(for: provider)) { _, value in
            guard provider.acceptsAPIKey, value != apiKey else { return }
            apiKey = value
        }
    }
}

private struct SourceControlSettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        ChromeSection(title: "Text generation") {
            ChromeCard {
                ChromeRow(
                    title: "Thread titles and commit messages",
                    detail: "Automatic uses the thread's own provider. Claude uses Haiku, Codex your default model."
                ) {
                    GlassPickerButton(options: TextGenerationChoice.allCases.map { ($0, $0.title) }, selection: $settings.textGeneration, asset: { ProviderKind(rawValue: $0.rawValue)?.iconName })
                }
            }
        }
        ChromeSection(title: "Commit message instructions") {
            ChromeCard {
                ZStack(alignment: .topLeading) {
                    TextEditor(text: $settings.commitInstructions)
                        .font(.system(size: 13))
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: 90)
                    if settings.commitInstructions.isEmpty {
                        Text("For example: use conventional commit prefixes")
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(.horizontal, 5)
                            .allowsHitTesting(false)
                    }
                }
                .padding(12)
            }
        }
        ChromeSection(title: "Worktrees and pull requests") {
            ChromeCard {
                ChromeRow(title: "Worktrees", detail: Storage.worktreesDirectory.path) {
                    Button("Reveal") { Workspace.revealInFinder(Storage.worktreesDirectory.path) }
                        .buttonStyle(.glass)
                }
                ChromeRowDivider()
                ChromeRow(title: "Pull requests", detail: "Opened with the GitHub CLI (gh) or GitLab CLI (glab) from your PATH.") {
                    EmptyView()
                }
            }
        }
    }
}

/// Clears the whole archive from the page's chrome row, asking first the way a single
/// delete does when the setting is on. The question hangs off the button as a popover,
/// like the sidebar's delete menu, rather than a sheet over the whole window.
private struct ArchiveDeleteAllButton: View {
    @Environment(AppModel.self) private var model
    @State private var isConfirming = false

    var body: some View {
        let count = model.archivedThreads.count
        ChromeTextButton(
            symbol: "trash",
            title: "Delete all",
            help: "Delete every archived thread",
            isEnabled: count > 0,
            isDestructive: true
        ) {
            if model.settings.confirmBeforeDeleting {
                isConfirming = true
            } else {
                model.deleteArchivedThreads()
            }
        }
        .popover(isPresented: $isConfirming, arrowEdge: .bottom) {
            PopoverMenu {
                PopoverSectionHeader(count == 1 ? "Delete the archived thread?" : "Delete all archived threads?")
                PopoverNote(count == 1
                    ? "Its history will be removed. Files in your project stay as they are."
                    : "The histories of all \(count) threads will be removed. Files in your projects stay as they are.")
                PopoverDivider()
                PopoverItem(count == 1 ? "Delete thread" : "Delete \(count) threads", symbol: "trash", isDestructive: true) {
                    model.deleteArchivedThreads()
                }
            }
        }
    }
}

/// The archived threads. The empty archive never reaches this page: the pane shows its symbol instead.
private struct ArchiveSettingsPage: View {
    @Environment(AppModel.self) private var model

    /// The row waiting on its confirmation, or nil. One deletion is ever in flight, so one
    /// dialog on the card serves every row.
    @State private var pendingDeletion: ChatThread?

    var body: some View {
        let archived = model.archivedThreads
        ChromeCard {
            ForEach(Array(archived.enumerated()), id: \.element.id) { index, thread in
                if index > 0 { ChromeRowDivider() }
                ChromeRow(title: thread.title, detail: model.project(thread.projectID)?.name) {
                    HStack(spacing: 8) {
                        Button("Restore") { model.unarchive(thread.id) }
                            .buttonStyle(.glass)
                        // The same check the sidebar and Delete all honour: a row here
                        // used to delete a history on the first click.
                        Button("Delete", role: .destructive) {
                            if model.settings.confirmBeforeDeleting {
                                pendingDeletion = thread
                            } else {
                                model.delete(thread.id)
                            }
                        }
                        .buttonStyle(.glass)
                    }
                    .controlSize(.small)
                }
            }
        }
        .confirmationDialog(
            pendingDeletion.map { "Delete \"\($0.title)\"?" } ?? "Delete the archived thread?",
            isPresented: Binding(
                get: { pendingDeletion != nil },
                set: { if !$0 { pendingDeletion = nil } }
            ),
            presenting: pendingDeletion
        ) { thread in
            Button("Delete thread", role: .destructive) {
                model.delete(thread.id)
                pendingDeletion = nil
            }
        } message: { _ in
            Text("Its history will be removed. Files in your project stay as they are.")
        }
    }
}

private struct AboutSettingsPage: View {
    @Environment(AppModel.self) private var model
    private static let droppyURL = URL(string: "https://getdroppy.app")!

    var body: some View {
        VStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
            AboutSoftwareUpdateSection()
            appCard
            ChromeSection(title: "Credits") {
                ChromeCard {
                    MadeByDroppyRow(url: Self.droppyURL)
                    ChromeRowDivider()
                    ChromeRow(title: "Working indicators", detail: "Ported from Zeron by Wing, MIT License.") {
                        CreditLink(title: "zeronsh/zeron", url: URL(string: "https://github.com/zeronsh/zeron")!)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Terminal", detail: "SwiftTerm by Miguel de Icaza, MIT License.") {
                        CreditLink(title: "SwiftTerm", url: URL(string: "https://github.com/migueldeicaza/SwiftTerm")!)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Welcome tour", detail: "Ported from TourKit by Ram Patra, MIT License.") {
                        CreditLink(title: "rampatra/TourKit", url: URL(string: "https://github.com/rampatra/TourKit")!)
                    }
                    ChromeRowDivider()
                    LicensesRow()
                }
            }
        }
    }

    private var appCard: some View {
        ChromeCard {
            HStack(spacing: 14) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 56, height: 56)
                VStack(alignment: .leading, spacing: 3) {
                    Text("Droppy Code")
                        .font(.system(size: 15, weight: .semibold))
                    Text(verbatim: "Version \(AppInfo.version) (\(AppInfo.build))")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                }
                Spacer()
            }
            .padding(16)
            ChromeRowDivider()
            ChromeRow(title: "Welcome tour", detail: "The slideshow from the first launch: Hydra, the effort slider, panels and themes.") {
                Button("Show") { Tour.present(model: model) }.buttonStyle(.glass).controlSize(.small)
            }
            ChromeRowDivider()
            AboutUpdateCheckRow()
            ChromeRowDivider()
            Text("The coding app by Droppy: a native home for your coding agents, built in Swift with Liquid Glass.")
                .font(.system(size: 13))
                .padding(.leading, 16)
                .padding(.trailing, Chrome.rowControlTrailingPadding)
                .padding(.vertical, 11)
            ChromeRowDivider()
            Text("Droppy Code is licensed under the GNU AGPL v3.0 with attribution terms; SwiftTerm is MIT licensed.")
                .font(.system(size: 12))
                .foregroundStyle(Chrome.secondaryText)
                .padding(.leading, 16)
                .padding(.trailing, Chrome.rowControlTrailingPadding)
                .padding(.vertical, 11)
        }
    }
}

/// The license, the third-party notices and the trademark policy, as shipped in the bundle,
/// read in a popover: every copy of the app carries the notices for what it builds on.
private struct LicensesRow: View {
    @State private var isPresented = false

    private static let files: [(title: String, resource: String)] = [
        ("Droppy Code", "LICENSE"),
        ("Third-party notices", "THIRD_PARTY_NOTICES"),
        ("Trademarks", "TRADEMARK"),
    ]

    var body: some View {
        ChromeRow(title: "Licenses", detail: "AGPL-3.0 with attribution terms, and the notices for what the app builds on.") {
            Button("Show") { isPresented = true }
                .buttonStyle(.glass)
                .controlSize(.small)
                .popover(isPresented: $isPresented, arrowEdge: .bottom) {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 18) {
                            ForEach(Self.files, id: \.resource) { file in
                                VStack(alignment: .leading, spacing: 8) {
                                    Text(verbatim: file.title)
                                        .font(.system(size: 13, weight: .semibold))
                                    MarkdownView(text: Self.text(of: file.resource)).equatable()
                                        .environment(\.markdownPointSize, 12)
                                        .foregroundStyle(Chrome.secondaryText)
                                        .textSelection(.enabled)
                                }
                            }
                        }
                        .padding(18)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(width: 480, height: 440)
                    .presentedChrome()
                }
        }
    }

    /// Read from the bundle once: the popover's body runs on every hover and scroll, and
    /// the markdown it now renders is parsed from this text each time as it is.
    private static let texts: [String: String] = Dictionary(uniqueKeysWithValues: files.map { file in
        let url = Bundle.main.url(forResource: file.resource, withExtension: file.resource == "LICENSE" ? nil : "md")
        let text = url.flatMap { try? String(contentsOf: $0, encoding: .utf8) } ?? "The \(file.resource) file is missing from this copy of the app."
        return (file.resource, text)
    })

    private static func text(of resource: String) -> String {
        texts[resource] ?? "The \(resource) file is missing from this copy of the app."
    }
}

/// Droppy's logo and name, linking to getdroppy.app, so it is clear the app is made by Droppy.
private struct MadeByDroppyRow: View {
    let url: URL
    @State private var isHovering = false

    var body: some View {
        Link(destination: url) {
            HStack(spacing: 12) {
                Image("droppy-logo")
                    .resizable()
                    .interpolation(.high)
                    .frame(width: 40, height: 40)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Made by Droppy")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Chrome.primaryText)
                    Text("Droppy Code is a coding app by Droppy for Mac.")
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                }
                Spacer(minLength: 12)
                HStack(spacing: 4) {
                    Text(verbatim: "getdroppy.app")
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 10, weight: .semibold))
                }
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.accent)
                .opacity(isHovering ? 1 : 0.9)
            }
            .padding(.leading, 16)
            .padding(.trailing, Chrome.rowControlTrailingPadding + 4)
            .padding(.vertical, 12)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help("Open getdroppy.app")
        .accessibilityLabel(Text("Made by Droppy. Open getdroppy.app"))
    }
}

private struct CreditLink: View {
    let title: String
    let url: URL

    var body: some View {
        Link(destination: url) {
            HStack(spacing: 4) {
                Text(verbatim: title)
                Image(systemName: "arrow.up.right")
                    .font(.system(size: 10, weight: .semibold))
            }
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(Chrome.accent)
        }
        .buttonStyle(.plain)
        .help(url.absoluteString)
    }
}

/// The theme menu: System on top, then the dark and the light themes in their
/// own sections, each with a swatch of its surface and accent.
private struct ThemePickerButton: View {
    @Binding var selection: AppTheme

    @State private var isPresented = false
    @State private var isHovering = false

    private static let dark = AppTheme.allCases.filter { $0.scheme == .dark }
    private static let light = AppTheme.allCases.filter { $0.scheme == .light }

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            HStack(spacing: 6) {
                selection.swatch
                    .frame(width: 14, height: 14)
                Text(verbatim: selection.displayName)
                    .font(.system(size: 12.5, weight: .medium))
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(Chrome.secondaryText)
            }
            .foregroundStyle(Chrome.primaryText.opacity(isHovering || isPresented ? 1 : 0.92))
            .padding(.horizontal, Chrome.capsuleHorizontalPadding)
            .frame(height: Chrome.capsuleContentHeight)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .fixedSize()
        .chromeGlassCapsule()
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu {
                item(.system)
                PopoverSectionHeader("Dark")
                ForEach(Self.dark) { item($0) }
                PopoverSectionHeader("Light")
                ForEach(Self.light) { item($0) }
            }
        }
    }

    private func item(_ theme: AppTheme) -> some View {
        PopoverItem(theme.displayName, leading: AnyView(theme.swatch), isChecked: theme == selection) {
            selection = theme
        }
    }
}
