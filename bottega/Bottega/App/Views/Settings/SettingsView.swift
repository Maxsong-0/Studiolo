// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

enum SettingsPage: String, CaseIterable, Identifiable {
    case general
    // Second in the sidebar: Crew is what the app is about, so it sits right under General.
    case crew
    case providers
    case mcp
    case studiolo
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
        case .mcp: "MCP"
        case .studiolo: "Studiolo"
        case .models: "Models"
        case .crew: "Crew"
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
        case .mcp: "puzzlepiece.extension"
        case .studiolo: "books.vertical"
        case .models: "slider.horizontal.3"
        // Never drawn: the sidebar gives the Crew row the anchor instead. Kept so
        // the switch stays exhaustive and anything else that asks gets a sane symbol.
        case .crew: "point.3.connected.trianglepath.dotted"
        case .sourceControl: "arrow.triangle.branch"
        case .shortcuts: "command"
        case .archive: "archivebox"
        case .about: "info.circle"
        }
    }

    var keywords: [String] {
        switch self {
        case .general: ["permissions", "worktree", "reasoning", "thinking", "notifications", "theme", "appearance", "transparency", "transparent", "opacity", "glass", "dark", "light", "accent", "tint", "bottega", "brass", "custom", "theme.json", "json", "colours", "colors", "wallpaper", "softness", "blur", "resolution", "token", "tokens", "activity", "usage", "panel", "limits", "credits", "text", "size", "font", "zoom", "typeface", "monospace", "terminal", "heatmap", "daily", "weekly", "cumulative", "settle", "settled", "finish", "finished", "done", "sound", "chime", "icon", "icons", "emoji", "symbol", "symbols", "project mark", "compact", "concise", "anti-slop", "slop", "verbose", "comments", "replies"]
        case .models: ["model", "effort", "reasoning", "fast", "slider", "picker"]
        case .crew: ["crew", "hands", "subagents", "sub-agents", "agents", "team", "orchestrator", "worker", "pair", "pairs", "parallel", "delegate", "queue", "profile", "profiles", "purpose", "lead", "instructions", "rules", "sop", "claude.md", "new chats", "folders", "repositories", "vault", "limits", "rounds", "budget", "watchdog", "stalled", "timeout", "checks", "verification", "merge", "push", "branch", "commit", "squash", "permissions", "access"]
        case .providers: ["codex", "claude", "cursor", "opencode", "grok", "deepseek", "meta", "muse", "spark", "zai", "z.ai", "glm", "coding plan", "devin", "cognition", "antigravity", "agy", "google", "gemini", "copilot", "github", "command code", "commandcode", "cmd", "binary", "path", "sign in", "login", "api key", "usage", "limits", "limit", "plan", "quota", "credits", "balance"]
        case .mcp: ["mcp", "model context protocol", "servers", "tools", "connect", "integrations", "github", "playwright", "notion", "linear", "slack", "figma", "supabase", "stripe", "resend", "email", "context7", "sentry", "postgres", "browser", "netlify", "deploy", "hosting"]
        case .studiolo: ["studiolo", "vault", "obsidian", "archive", "archives", "index", "dashboard", "status", "web", "项目", "档案"]
        case .sourceControl: ["git", "commit", "pull request", "titles", "text generation", "checkpoint", "checkpoints", "snapshot", "revert"]
        case .shortcuts: ["keyboard", "keys"]
        case .archive: ["archived", "restore"]
        case .about: ["version", "license"]
        }
    }
}

/// A page asked for from outside the window: `WindowManager.showSettings(page:)`, or a
/// button elsewhere that opens Settings on the page it is about (Providers, for a missing
/// key). Read once by the view, then cleared.
@MainActor
@Observable
final class SettingsNavigation {
    static let shared = SettingsNavigation()
    var requestedPage: SettingsPage?
}

/// The Settings window's size, read by both the window `WindowManager` makes and the view
/// that fills it so the two never drift apart, and the width of the page list down its
/// side. Larger than upstream's 780 × 580: the Crew and Providers pages run long, and a
/// wider detail pane keeps their rows' explanations to fewer lines. The window has no
/// resize handle, so this is the one place to change its size.
enum SettingsWindowLayout {
    static let size = CGSize(width: 920, height: 680)
    static let sidebarWidth: CGFloat = 224
}

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var page: SettingsPage = .general
    @State private var search = ""
    @State private var modelSearch = ""
    @State private var mcpSearch = ""
    @State private var scrollChrome = ChromeScrollModel()
    @State private var scrollTracker = SettingsScrollTracker()

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
                .frame(width: SettingsWindowLayout.sidebarWidth)
            detail
                .padding(.trailing, Chrome.sheetInset)
                .padding(.vertical, Chrome.sheetInset)
        }
        .onDisappear { scrollTracker.end() }
        .background { WindowBackdrop(opacity: model.settings.backdropOpacity) }
        .background { WindowChromeConfigurator() }
        .clipShape(RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous))
        .ignoresSafeArea()
        .frame(width: SettingsWindowLayout.size.width, height: SettingsWindowLayout.size.height)
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
                                if item == .crew {
                                    // The Crew page wears the anchor, the same one as the composer button.
                                    CrewMarkImage()
                                        .frame(width: Chrome.symbolSize * 1.15, height: Chrome.symbolSize * 1.15)
                                } else {
                                    SidebarSymbol(item.symbol, scale: item == .general ? 1.15 : 1)
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
                .allowsHitTesting(!scrollTracker.isFrozen)
            }
            .onScrollGeometryChange(for: CGFloat.self, of: { $0.contentOffset.y }) { _, _ in
                scrollTracker.note()
            }
            .onScrollPhaseChange { _, phase in
                if phase == .interacting { scrollTracker.note() } else { scrollTracker.coast() }
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
            mcpSearch = ""
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
            .allowsHitTesting(!scrollTracker.isFrozen)
        }
        .scrollIndicators(.never)
        .id(page)
        .onScrollGeometryChange(for: CGFloat.self, of: Self.travel) { _, travel in
            scrollTracker.note()
            scrollChrome.update(travel: travel)
        }
        .onScrollPhaseChange { _, phase in
            // Fingers on the content freeze the rows (no click can land then anyway); the
            // moment the scroll coasts or settles they come back, so a click that stops a
            // flick always reaches its target. Wheel scrolling reports no phases and is
            // caught by the movement above instead.
            if phase == .interacting { scrollTracker.note() } else { scrollTracker.coast() }
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
                if page == .mcp {
                    ChromeSearchField(query: $mcpSearch, prompt: "Search servers")
                }
                if page == .providers {
                    ProvidersRefreshButton()
                }
                if page == .archive {
                    ArchiveDeleteAllButton()
                }
            }
            .frame(minHeight: Chrome.capsuleHeight)
            .padding(.horizontal, Chrome.chromeHorizontalPadding)
            .padding(.top, Chrome.chromeTopPadding)
        }
    }

    private var pageHasChromeControls: Bool {
        page == .providers || page == .models || page == .mcp || page == .archive || page == .about
    }

    @ViewBuilder
    private var pageContent: some View {
        switch page {
        case .general: GeneralSettingsPage()
        case .providers: ProvidersSettingsPage()
        case .mcp: MCPSettingsPage(query: mcpSearch)
        case .studiolo: StudioloSettingsPage()
        case .models:
            ModelsSettingsPage(query: modelSearch)
        case .crew: CrewSettingsPage()
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
                ChromeRow(title: "Provider and model", detail: "A new thread follows the one you are in: its provider, model, effort and Crew pair. With no thread open it starts on the provider you last chose.") {
                    EmptyView()
                }
                ChromeRowDivider()
                ChromeRow(title: "Permissions", detail: settings.defaultRuntimeMode.summary) {
                    GlassPickerButton(options: RuntimeMode.allCases.map { ($0, $0.title) }, selection: $settings.defaultRuntimeMode, symbol: { $0.symbol })
                }
                ChromeRowDivider()
                ChromeRow(title: "Workspace", detail: "Where a new thread makes its changes") {
                    ChromeVisualPicker(options: WorkspaceMode.allCases.map { ($0, $0.title) }, selection: $settings.defaultWorkspaceMode) { mode in
                        Image(systemName: mode == .local ? "folder" : "arrow.triangle.branch")
                    }
                }
            }
        }
            ChromeSection(title: "Conversation") {
                ChromeCard {
                    ChromeRow(title: "Show thinking", detail: "The working line opens to the agent's thinking") {
                        SettingsSwitch(isOn: $settings.showReasoning)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Chronological timeline", detail: "Finished turns open on all their steps, in the order they arrived") {
                        SettingsSwitch(isOn: $settings.chronologicalTimeline)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Hide file reads", detail: "Reads leave the steps; the work summary still counts them") {
                        SettingsSwitch(isOn: $settings.hidesReadSteps)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Hide searches", detail: "Likewise for searches") {
                        SettingsSwitch(isOn: $settings.hidesSearchSteps)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Recent steps shown", detail: "How many of a work group's latest steps stay open; the rest fold under one line") {
                        ChromeSegmentedPicker(
                            options: [1, 2, 3, 5, 8].map { ChromeSegmentedOption(value: $0, title: String($0)) },
                            selection: $settings.recentStepsShown
                        )
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Concise AI replies", detail: "Built-in guidance for shorter replies, useful code comments, and focused work, including Crew hands. Changes apply on the next message.") {
                        SettingsSwitch(isOn: $settings.antiSlopEnabled)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Working line", detail: "A card with the turn's progress bar, or a plain line") {
                        ChromeVisualPicker(options: [(true, "Card"), (false, "Line")], selection: $settings.showsWorkingCard) { isCard in
                            Image(systemName: isCard ? "rectangle.fill" : "minus")
                        }
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
                ChromeRowDivider()
                ChromeRow(title: "Marks", detail: "An emoji or symbol for each project, drawn before the thread's name in the activity list and on the project's own row") {
                    ProjectMarksRow()
                }
            }
        }
        ChromeSection(title: "Finished threads") {
            ChromeCard {
                ChromeRow(title: "Done with a thread", detail: settings.threadFinishAction.detail) {
                    ChromeVisualPicker(options: ThreadFinishAction.allCases.map { ($0, $0.title) }, selection: $settings.threadFinishAction) { action in
                        Image(systemName: action == .settle ? "checkmark" : "archivebox")
                    }
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
                CustomThemeFileRow(theme: $settings.theme)
                ChromeRowDivider()
                ChromeRow(title: "Transparency", detail: "How much shows through the window") {
                    BackdropOpacitySlider(value: $settings.backdropOpacity)
                }
                ChromeRowDivider()
                ChromeRow(title: "Sidebar", detail: settings.sidebarMode.detail) {
                    ChromeVisualPicker(options: SidebarMode.allCases.map { ($0, $0.title) }, selection: $settings.sidebarMode) { mode in
                        Image(systemName: mode == .column ? "sidebar.left" : (mode == .floating ? "macwindow.on.rectangle" : "macwindow"))
                    }
                }
                ChromeRowDivider()
                ChromeRow(title: "Activity list", detail: settings.activityThreadStyle.detail) {
                    ChromeVisualPicker(options: ActivityThreadStyle.allCases.map { ($0, $0.title) }, selection: $settings.activityThreadStyle) { style in
                        Image(systemName: style == .icon ? "list.bullet" : "text.alignleft")
                    }
                }
                ChromeRowDivider()
                // Meaningless without a column: dimmed while only the panel is shown.
                ChromeRow(title: "Open the sidebar at launch", detail: "Collapsed otherwise; the list button in the toolbar shows and hides it any time") {
                    SettingsSwitch(isOn: Binding(
                        get: { model.sidebar.startsOpen },
                        set: { model.sidebar.startsOpen = $0 }
                    ))
                }
                .disabled(settings.sidebarMode == .panelOnly)
                .opacity(settings.sidebarMode == .panelOnly ? 0.5 : 1)
            }
            WallpaperCard()
            ChatTextSizeCard(settings: settings)
        }
    }
}

/// The main window's type: a sample of a chat drawn at the chosen size and in the chosen
/// faces, so a change is seen here before any thread is opened, over the rows that set
/// them. The size reaches the whole main window; the faces, the conversation's prose, its
/// code and the terminal.
private struct ChatTextSizeCard: View {
    @Bindable var settings: AppSettings

    var body: some View {
        let index = settings.chatZoom
        let percentage = ChatZoom.percentage(at: index)
        ChromeCard {
            // The sample, at the values the main window reads (this window keeps the stock
            // size and faces for itself).
            ChatTypeSample()
                .environment(\.chatZoom, ChatZoom.scale(at: index))
                .environment(\.chatTypefaces, settings.chatTypefaces)
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .animation(.snappy(duration: 0.2), value: index)
                .accessibilityHidden(true)
            ChromeRowDivider()
            ChromeRow(title: "Text size", detail: percentage == 100 ? "The stock size, for the whole window" : "\(percentage)% of the stock size, for the whole window") {
                ChatTextSizeSlider(index: $settings.chatZoom)
            }
            ChromeRowDivider()
            ChromeRow(title: "Text font", detail: Self.detail(for: settings.textFontFamily, reads: "The conversation's prose: replies and the messages you send")) {
                FontFamilyPickerButton(selection: $settings.textFontFamily, kind: .text)
            }
            ChromeRowDivider()
            ChromeRow(title: "Code font", detail: Self.detail(for: settings.codeFontFamily, reads: "Code blocks, inline code and the terminal")) {
                FontFamilyPickerButton(selection: $settings.codeFontFamily, kind: .code)
            }
            ChromeRowDivider()
            ChromeRow(title: "Terminal text", detail: "The terminal's size at the stock text size; the text size above scales it with the window") {
                TerminalFontSizeStepper(value: $settings.terminalFontSize)
            }
        }
    }

    /// A row's detail, or why the family it names is not what the window shows.
    private static func detail(for family: String?, reads: String) -> String {
        guard let family, Typefaces.installed(family) == nil else { return reads }
        return "\(family) is not installed on this Mac, so the system face stands in"
    }
}

/// A chat in miniature: a message in its bubble and the start of a reply, in the type the
/// card sets (its environment's `chatZoom` and `chatTypefaces`), the reply's inline code
/// in the code face once one is picked.
private struct ChatTypeSample: View {
    @Environment(\.chatZoom) private var zoom
    @Environment(\.chatTypefaces) private var typefaces

    private static let message = "Where does the composer decide how far the draft photo sits from the edges?"
    private static let reply = "In `DraftAttachments`, inside **ComposerView.swift**. The strip pads its leading edge with `attachmentInset` but leaves the top on the stack's default spacing, which is why the photo hugs the top of the pill."

    var body: some View {
        let font = Font.chat(.body, zoom: zoom, family: typefaces.text)
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Spacer(minLength: 48)
                Text(Self.message)
                    .font(font)
                    .foregroundStyle(Chrome.primaryText)
                    .padding(.leading, 14)
                    .padding(.trailing, 14 + UserBubble.tail)
                    .padding(.vertical, 9)
                    .background(.tint.opacity(0.14), in: UserBubble())
            }
            replyText
                .font(font)
                .foregroundStyle(Chrome.primaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The reply as markdown: its code spans in the monospaced cut of the prose's face, as
    /// ever, until a code face is picked, and in that face from then on (see
    /// `InlineText.settingCode`).
    private var replyText: Text {
        guard let code = typefaces.code else { return Text(LocalizedStringKey(Self.reply)) }
        let parsed = (try? AttributedString(markdown: Self.reply, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(Self.reply)
        return Text(InlineText.settingCode(of: parsed, family: code, size: (Font.chatBaseSize(.body) * zoom).rounded()))
    }
}

/// Picks a family for the prose or the code from those installed: a glass capsule naming
/// the choice in its own face, and a popover listing the families, each in its own face,
/// under a search field, with the system's face on top.
private struct FontFamilyPickerButton: View {
    enum Kind {
        case text
        case code

        var systemTitle: String {
            switch self {
            case .text: "System"
            case .code: "System monospaced"
            }
        }

        var design: Font.Design {
            switch self {
            case .text: .default
            case .code: .monospaced
            }
        }
    }

    @Binding var selection: String?
    let kind: Kind

    @State private var isPresented = false
    @State private var isHovering = false

    var body: some View {
        // What the window reads: a family no longer installed shows as the system face.
        let shown = Typefaces.installed(selection)
        Button {
            isPresented.toggle()
        } label: {
            HStack(spacing: 6) {
                Text(verbatim: shown ?? kind.systemTitle)
                    .font(Typefaces.font(family: shown, size: 12.5, weight: .medium, design: kind.design))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 180, alignment: .leading)
                Image(systemName: "chevron.down")
                    .font(Chrome.chevronFont)
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
            FontFamilyList(selection: $selection, kind: kind)
        }
        .background { NoWindowDragArea() }
        .accessibilityLabel(Text(kind == .text ? "Text font" : "Code font"))
        .accessibilityValue(Text(verbatim: shown ?? kind.systemTitle))
    }
}

/// The families to pick from, narrowed as the reader types, each drawn in its own face. The
/// list is read as the popover opens, so a font installed since is on it.
private struct FontFamilyList: View {
    @Binding var selection: String?
    let kind: FontFamilyPickerButton.Kind

    @State private var query = ""
    @State private var families: [String] = []

    var body: some View {
        let trimmed = query.trimmingCharacters(in: .whitespaces)
        let shown = trimmed.isEmpty ? families : families.filter { $0.localizedCaseInsensitiveContains(trimmed) }
        VStack(spacing: 0) {
            SidebarSearchField(text: $query, prompt: kind == .text ? "Search fonts" : "Search monospaced fonts")
                .padding([.horizontal, .top], 8)
                .padding(.bottom, 4)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if trimmed.isEmpty {
                        PopoverItem(kind.systemTitle, text: Text(verbatim: kind.systemTitle).font(.system(size: 13, design: kind.design)), isChecked: Typefaces.installed(selection) == nil) {
                            selection = nil
                        }
                        PopoverDivider()
                    }
                    ForEach(shown, id: \.self) { family in
                        PopoverItem(family, text: Text(verbatim: family).font(Typefaces.font(family: family, size: 13, design: kind.design)), isChecked: family == selection) {
                            selection = family
                        }
                    }
                    if shown.isEmpty, !trimmed.isEmpty {
                        PopoverNote("No installed font matches")
                    }
                }
                .padding(6)
            }
            .scrollIndicators(.automatic)
        }
        .frame(width: 280, height: 360)
        .presentedChrome()
        .onAppear { families = Typefaces.families(fixedPitch: kind == .code) }
    }
}

/// The terminal's size at the stock text size, a point a step.
private struct TerminalFontSizeStepper: View {
    @Binding var value: Double

    var body: some View {
        HStack(spacing: 8) {
            Text(verbatim: "\(Int(value)) pt")
                .font(.system(size: 12).monospacedDigit())
                .foregroundStyle(Chrome.secondaryText)
            Stepper("Terminal text size", value: $value, in: AppSettings.terminalFontSizes, step: 1)
                .labelsHidden()
                .controlSize(.small)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Terminal text size"))
        .accessibilityValue(Text("\(Int(value)) points"))
    }
}

/// Smaller type on the left, larger on the right, the stock size at its own step
/// (`ChatZoom.defaultIndex`, left of the middle: the scale reaches further up than down),
/// with a reset that shows only once the slider has left it (see `BackdropOpacitySlider`).
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
/// A provider switched off is never checked, so it has a row of its own at the foot of the
/// page with nothing on it but the switch that brings it back.
private struct ProvidersSettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let settings = model.settings
        let switchedOff = ProviderKind.allCases.filter { !settings.isEnabled($0) }
        // Eager on purpose: a handful of sections, and lazy ones were rebuilt while scrolling,
        // which reset their fields and re-ran every check they started on appear.
        VStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
            ForEach(ProviderKind.allCases.filter { settings.isEnabled($0) }) { provider in
                ProviderSettingsSection(provider: provider)
            }
            if !switchedOff.isEmpty {
                SwitchedOffProvidersSection(providers: switchedOff)
            }
        }
    }
}

/// The providers switched off in Settings, one small row each: nothing about them was
/// checked (see `ProviderRegistry.refreshAll`), so there is no status to show, only the
/// switch that brings one back and has it checked there and then.
private struct SwitchedOffProvidersSection: View {
    @Environment(AppModel.self) private var model
    let providers: [ProviderKind]

    var body: some View {
        ChromeSection(title: "Switched off") {
            ChromeCard {
                ForEach(Array(providers.enumerated()), id: \.element) { index, provider in
                    if index > 0 { ChromeRowDivider() }
                    HStack(spacing: 12) {
                        ProviderIcon(provider: provider, size: 18)
                            .foregroundStyle(Chrome.secondaryText)
                            .frame(width: 22)
                        Text(verbatim: provider.displayName)
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.secondaryText)
                        Spacer()
                        SettingsSwitch(isOn: Binding(
                            get: { model.settings.isEnabled(provider) },
                            set: { model.setProviderEnabled($0, for: provider) }
                        ))
                    }
                    .padding(.leading, 16)
                    .padding(.trailing, Chrome.rowControlTrailingPadding)
                    .padding(.vertical, 11)
                }
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

    /// Whether the page offers a sign-in row: hidden while the provider is signed in and
    /// working, and while a check runs, so it never flashes in before the answer.
    private var showsSignIn: Bool {
        let status = model.providers.status(provider)
        if case .signedIn = status.auth { return false }
        return !status.isChecking
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
                        set: { model.setProviderEnabled($0, for: provider) }
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
                            Link(destination: provider.installURL) {
                                Text(verbatim: "Get a \(provider.displayName) key")
                                    .frame(height: Chrome.rowButtonLabelHeight)
                            }
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
                            Link(destination: provider.installURL) {
                                Text(verbatim: "Get \(provider.displayName)")
                                    .frame(height: Chrome.rowButtonLabelHeight)
                            }
                            .buttonStyle(.glass)
                        }
                    } else if showsSignIn {
                        ChromeRowDivider()
                        ChromeRow(title: "Sign in", detail: "Click here to login again.") {
                            Button {
                                Workspace.openTerminal(running: provider.loginCommand)
                            } label: {
                                Text(verbatim: "Login").frame(height: Chrome.rowButtonLabelHeight)
                            }
                            .buttonStyle(.glass)
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
        ChromeSection(title: "Checkpoints") {
            ChromeCard {
                ChromeRow(
                    title: "Turns kept",
                    detail: settings.checkpointKeepTurns.map { "A thread's first message and its last \($0) turns can put their files back when you edit them. The turns between, and archived threads, let their snapshots go." }
                        ?? "Every turn can put its files back when you edit its message, until its thread is archived."
                ) {
                    ChromeSegmentedPicker(
                        options: [ChromeSegmentedOption(value: Int?.none, title: "Every turn", symbol: "infinity")]
                            + AppSettings.checkpointKeepTurnsChoices.map { ChromeSegmentedOption(value: Optional($0), title: String($0)) },
                        selection: $settings.checkpointKeepTurns
                    )
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
    /// Droppy Code's own site: the attribution LICENSE section 7(a) asks every modified copy to show here.
    private static let upstreamURL = URL(string: "https://getdroppycode.app")!

    var body: some View {
        VStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
            heroCard
            ChromeSection(title: "This copy") {
                ChromeCard {
                    ChromeRow(title: "Build", detail: Self.buildDescription) { EmptyView() }
                    ChromeRowDivider()
                    ChromeRow(title: "Bundle", detail: Bundle.main.bundleIdentifier ?? "unknown") { EmptyView() }
                    ChromeRowDivider()
                    ChromeRow(title: "Library", detail: Self.tilde(Storage.root)) {
                        RevealButton(url: Storage.root)
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Worktrees and checkpoints", detail: Self.tilde(Storage.worktreesDirectory)) {
                        RevealButton(url: Storage.worktreesDirectory)
                    }
                }
            }
            ChromeSection(title: "Licenses") {
                ChromeCard { LicensesRow() }
            }
        }
    }

    /// The icon, the name, the version and the one line that says what this is. The icon is
    /// read from the running app, so it follows whatever the asset catalog ships.
    private var heroCard: some View {
        ChromeCard {
            HStack(alignment: .top, spacing: 16) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 64, height: 64)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Bottega")
                        .font(.system(size: 20, weight: .semibold))
                    Text("A modified version of Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app.")
                        .font(.system(size: 13))
                        .foregroundStyle(Chrome.secondaryText)
                    Text(verbatim: "Version \(AppInfo.version) (\(AppInfo.build))")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                        .padding(.top, 2)
                }
                Spacer(minLength: 12)
                Link(destination: Self.upstreamURL) {
                    HStack(spacing: 4) {
                        Text(verbatim: "getdroppycode.app")
                        Image(systemName: "arrow.up.right")
                            .font(.system(size: 10, weight: .semibold))
                    }
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Chrome.accent)
                }
                .buttonStyle(.plain)
            }
            .padding(16)
            ChromeRowDivider()
            Text("A native cockpit for the coding agents already on this Mac: their CLIs, one window, and a Crew of helpers that works the way Max works.")
                .font(.system(size: 13))
                .padding(.leading, 16)
                .padding(.trailing, Chrome.rowControlTrailingPadding)
                .padding(.vertical, 11)
        }
    }

    /// Which build this is and how it is signed: the things that decide where its data
    /// lives and why it launches at all (see the Debug config's hardened-runtime note).
    private static var buildDescription: String {
        AppInfo.isDevelopment
            ? "Debug. Self-signed; hardened runtime off so the debug dylib loads."
            : "Release. Self-signed, hardened runtime on."
    }

    /// A path with the home folder folded to `~`, the way people read them.
    private static func tilde(_ url: URL) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let path = url.path
        return path.hasPrefix(home) ? "~" + String(path.dropFirst(home.count)) : path
    }
}

/// Opens Finder on a folder this copy of the app writes to.
private struct RevealButton: View {
    let url: URL

    var body: some View {
        Button("Reveal") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
            .buttonStyle(.glass)
            .controlSize(.small)
    }
}

/// The license as shipped in the bundle, read in a popover.
private struct LicensesRow: View {
    @State private var isPresented = false

    private static let files: [(title: String, resource: String)] = [
        ("License", "LICENSE"),
        ("Third-party notices", "THIRD_PARTY_NOTICES"),
        ("Trademark", "TRADEMARK"),
    ]

    var body: some View {
        ChromeRow(title: "License and notices", detail: "GNU AGPL v3.0 with attribution terms; third-party notices; trademark policy.") {
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

/// The per-project marks behind one button: the Projects card stays two rows long, and the
/// list of projects with their marks opens in a popover off this row.
private struct ProjectMarksRow: View {
    @Environment(AppModel.self) private var model

    @State private var isPresented = false

    var body: some View {
        Button("Edit") { isPresented = true }
            .buttonStyle(.glass)
            .controlSize(.small)
            .disabled(model.projects.isEmpty)
            .help("Choose a mark for each project")
            .popover(isPresented: $isPresented, arrowEdge: .bottom) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(model.projects.enumerated()), id: \.element.id) { index, project in
                        if index > 0 { ChromeRowDivider() }
                        ChromeRow(title: project.name, detail: project.path) {
                            ProjectIconButton(icon: icon(for: project.id))
                        }
                    }
                }
                .frame(width: 400)
            }
    }

    private func icon(for id: UUID) -> Binding<ProjectIcon?> {
        Binding(
            get: { model.project(id)?.icon },
            set: { newIcon in model.updateProject(id) { $0.icon = newIcon } }
        )
    }
}

/// The theme menu: the four built in, then the custom slot under a header of its own, each
/// with a swatch of its surface and accent.
private struct ThemePickerButton: View {
    @Binding var selection: AppTheme

    @State private var isPresented = false
    @State private var isHovering = false

    private static let builtIn = AppTheme.allCases.filter { $0 != .custom }

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
                ForEach(Self.builtIn) { item($0) }
                PopoverSectionHeader("Your own")
                item(.custom)
            }
        }
    }

    /// A theme's row. The custom slot waits for its file: until theme.json has been read it
    /// would only wear the Bottega colours, so it is offered once there is a theme in it.
    private func item(_ theme: AppTheme) -> some View {
        PopoverItem(
            theme.displayName,
            detail: theme.detail,
            leading: AnyView(theme.swatch),
            isChecked: theme == selection,
            isEnabled: AppTheme.available(current: selection).contains(theme)
        ) {
            selection = theme
        }
    }
}

/// Where the custom theme comes from: theme.json, what its last read made of it, and the
/// way to it. Every save of the file applies at once, so there is no Apply here.
private struct CustomThemeFileRow: View {
    @Binding var theme: AppTheme

    private var store: CustomThemeStore { .shared }

    var body: some View {
        let isMissing = store.status == .missing
        ChromeRow(title: "Theme file", detail: detail) {
            HStack(spacing: 8) {
                Button("Show in Finder") { store.reveal() }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                    .help("Shows theme.json, or the folder it goes in")
                Button(isMissing ? "Create" : "Edit") {
                    // A new file starts as the Bottega colours spelled out, and is put on
                    // at once, so the first edit shows as it is saved.
                    if store.edit(), store.status == .loaded { theme = .custom }
                }
                .buttonStyle(.glass)
                .controlSize(.small)
                .help(isMissing ? "Writes a theme.json to start from, switches to it and opens it" : "Opens theme.json in your editor")
            }
        }
    }

    private var detail: String {
        switch store.status {
        case .missing:
            "Your own theme, from a theme.json you edit in any editor; Create writes one to start from, and each save applies at once"
        case .loaded:
            "“\(AppTheme.custom.displayName)”, from theme.json; each save applies at once"
        case .failed(let reason):
            reason
        }
    }
}
