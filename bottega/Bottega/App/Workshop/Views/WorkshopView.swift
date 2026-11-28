// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import SwiftUI

/// The window (Vetro, ADR-047): a photo at the back, and over it two floating panes of glass, the sidebar and the
/// main pane with the inspector inside it.
struct WorkshopView: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        @Bindable var model = model
        // The window's own width, which this is handed whatever is in it. The width of what is in it would not
        // do: panes that have not given way yet keep that from getting any narrower, so they never would.
        GeometryReader { proxy in
            let window = Double(proxy.size.width)
            let drawn = WindowMeasures.drawn(
                sidebar: model.showsSidebar ? model.sidebarWidth : nil,
                panel: model.showsInspector ? model.panelWidth : nil, window: window)
            HStack(spacing: 0) {
                if let sidebar = drawn.sidebar {
                    HStack(spacing: 0) {
                        VetroSidebar(width: sidebar)
                        // The gap between the two panes is what drags the sidebar's edge.
                        PaneHandle(pane: .sidebar, width: sidebar, window: window)
                    }
                    .transition(reduceMotion ? .opacity : .move(edge: .leading).combined(with: .opacity))
                }
                MainPane(panel: drawn.panel, window: window)
            }
            .animation(reduceMotion ? .easeOut(duration: 0.2) : .smooth(duration: 0.36), value: model.showsSidebar)
            .padding(WindowMeasures.margin)
        }
        #if DEBUG
        .overlay(alignment: .top) { if PanelsForAScreenshot.isAsked { PanelsForAScreenshot() } }
        .task { await WindowForAScreenshot.resize() }
        #endif
        .background { BackdropView() }
        .ignoresSafeArea()
        .toolbar {
            ToolbarItem(placement: .navigation) { Color.clear.frame(width: 1, height: 1).accessibilityHidden(true) }
                .sharedBackgroundVisibility(.hidden)
        }
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        .preferredColorScheme(look.colorScheme)
        .sheet(item: $model.askBeingRead) { item in
            AskSheet(id: item.id)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.cameToFront()
        }
        // Wide enough for whichever side panes are shown, so that none is pushed past the window's edge.
        .frame(
            minWidth: WindowMeasures.narrowestWindow(sidebar: model.showsSidebar, panel: model.showsInspector),
            minHeight: WindowMeasures.windowShortest)
    }
}

/// The main pane: the page's title and tools along the top, the page, and the inspector on the right.
struct MainPane: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// How wide the panel is drawn; nil when it is put away.
    let panel: Double?
    let window: Double

    var body: some View {
        VStack(spacing: 0) {
            PageHeader()
            HStack(spacing: 0) {
                PageView()
                    .frame(minWidth: WindowMeasures.pageNarrowest)
                if let panel {
                    SidePanel()
                        .frame(width: panel)
                        .vetroSurface(cornerRadius: 14)
                        .clipShape(.rect(cornerRadius: 14, style: .continuous))
                        // Astride the panel's left edge.
                        .overlay(alignment: .leading) {
                            PaneHandle(pane: .panel, width: panel, window: window).offset(x: -WindowMeasures.handle / 2)
                        }
                        .padding(EdgeInsets(top: 0, leading: 0, bottom: 10, trailing: WindowMeasures.panelInset))
                        .transition(reduceMotion ? .opacity : .move(edge: .trailing).combined(with: .opacity))
                }
            }
            .animation(reduceMotion ? .easeOut(duration: 0.2) : .smooth(duration: 0.36), value: model.showsInspector)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .vetroPane(cornerRadius: 20)
        // Over the main pane only, as in the canvas: the sidebar stays as it is.
        .overlay {
            if model.showsPalette { WorkshopPalette() }
        }
        .animation(reduceMotion ? .easeOut(duration: 0.16) : .easeOut(duration: 0.2), value: model.showsPalette)
    }
}

/// The page's name and what it is, and on the right a small capsule of tools.
private struct PageHeader: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 10) {
            if !model.showsSidebar {
                // The window's own buttons are over this end of the header now; next to them, what brings the
                // sidebar back.
                SidebarButton().padding(.leading, WindowMeasures.windowButtons)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: PageView.title(model))
                    .font(.system(size: 13.5, weight: .semibold))
                    .lineLimit(1)
                let subtitle = PageView.subtitle(model)
                if !subtitle.isEmpty {
                    Text(verbatim: subtitle).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            tools
        }
        .padding(.horizontal, 14)
        .frame(height: 52)
        .navigationTitle(PageView.title(model))
    }

    private var tools: some View {
        HStack(spacing: 2) {
            if case .line = model.page {
                VetroSegmented<Bool>(
                    label: "View",
                    items: [
                        .init(value: false, title: "Timeline", symbol: "text.alignleft", help: "Show this line as a timeline"),
                        .init(
                            value: true, title: "Lanes", symbol: "arrow.left.arrow.right",
                            help: "Show this line as letters between agents"),
                    ],
                    selected: model.showsLanes
                ) { model.showsLanes = $0 }
            }
            if case .line(let id) = model.page, let line = model.state.threads[id] {
                ArrangeButton(line: line)
            }
            if model.page == .board {
                // The same kind of control as Chat / Sequence in the canvas.
                VetroSegmented<WorkshopModel.CardLayout>(
                    label: "Layout",
                    items: [
                        .init(value: .board, title: "Board", help: "Show the cards as a board"),
                        .init(value: .list, title: "List", help: "Show the cards as a list, most recent first"),
                    ],
                    selected: model.cardLayout
                ) { layout in
                    withAnimation(CardsMotion.switching(reduceMotion: reduceMotion)) { model.cardLayout = layout }
                }
            }
            Button { model.showsInspector.toggle() } label: {
                Image(systemName: "sidebar.trailing")
                    .font(.system(size: 13))
                    .frame(width: 28, height: 28)
                    .background(model.showsInspector ? look.selection : .clear, in: .circle)
                    .contentShape(.circle)
            }
            .buttonStyle(.plain)
            .help(model.showsInspector ? "Hide the panel" : "Show the panel")
            .accessibilityLabel(model.showsInspector ? "Hide the panel" : "Show the panel")
        }
        .padding(3)
        .glassBehind(.clear.tint(look.surface), in: .capsule)
        .vetroEdge(in: Capsule())
    }
}

/// Opens the line's arrange panel; opening it signs nothing.
private struct ArrangeButton: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    let line: BottegaKit.Line

    var body: some View {
        let here = WorkshopModel.Arranging(line: line.id, inSidebar: false)
        let isOpen = model.arrangePanel(at: here)
        Button { model.arranging = isOpen.wrappedValue ? nil : here } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 13))
                .frame(width: 28, height: 28)
                .background(isOpen.wrappedValue ? look.selection : .clear, in: .circle)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .help("Rename, pin or archive this line")
        .accessibilityLabel("Arrange line")
        .popover(isPresented: isOpen, arrowEdge: .bottom) { ArrangePanel(line: line) }
    }
}

/// What the middle shows: the page picked, or what has to be done before there is one.
struct PageView: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        FloatingTop(makesRoom: isReadOnly) {
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
        } over: {
            StatusBanner()
            NoticeBanner()
        }
    }

    /// Nothing on it is signed or sent. (What lanes have to send with is at the bottom, out of the way.)
    private var isReadOnly: Bool {
        guard model.source != .none, !model.needsRegistration else { return false }
        return switch model.page {
        case .board, .activity, .rooms: true
        case .line: model.showsLanes
        case .dispatcher, .waiting, .devices, .projects, .bot: false
        }
    }

    @ViewBuilder private var content: some View {
        if model.source == .none {
            ChooseRunDirectory()
        } else if model.needsRegistration {
            RegistrationView()
        } else {
            switch model.page {
            case .rooms:
                ContentUnavailableView(
                    "No Rooms in LMAS Yet", systemImage: SidebarSection.rooms.symbol,
                    description: Text("When LMAS lets agents open rooms to talk things over, they show up here."))
            case .dispatcher:
                TimelinePage(rows: model.state.dispatcherRows, key: .dispatcher, empty: Self.emptyDispatcher)
            case .waiting: WaitingView()
            case .board:
                switch model.cardLayout {
                case .board: BoardView(risesIn: model.cardsRise.isDue).transition(.opacity)
                case .list: CardListView().transition(.opacity)
                }
            case .activity: ActivityView()
            case .devices: DevicesView()
            case .projects: ProjectsView()
            case .bot(let vendor): BotHomeView(vendor: vendor).id(vendor)
            case .line(let id):
                if let line = model.state.threads[id] {
                    LinePage(line: line).id(id)
                } else {
                    ContentUnavailableView("No Such Line", systemImage: "text.bubble", description: Text(verbatim: id))
                }
            }
        }
    }

    private static let emptyDispatcher = ContentUnavailableView(
        "Nothing Yet", systemImage: Page.dispatcher.symbol,
        description: Text("What you say to the dispatcher, and what comes of it, shows up here."))

    static func title(_ model: WorkshopModel) -> String {
        if case .line(let id) = model.page, let line = model.state.threads[id] { return model.state.title(of: line) }
        return model.page.fixedTitle
    }

    static func subtitle(_ model: WorkshopModel) -> String {
        switch model.page {
        case .line(let id):
            guard let line = model.state.threads[id] else { return "" }
            return "\(Wording.lineKind(model.state.kind(of: line))) · \(UntrustedText.line(id, limit: 40))"
        case .bot(let vendor):
            return Wording.roles(model.state.catalogs[vendor]?.listed?.roles ?? [])
        case .rooms, .dispatcher, .waiting, .board, .activity, .devices, .projects:
            if case .recording(let url) = model.source { return "Recording · \(url.lastPathComponent)" }
            return ""
        }
    }
}

struct ChooseRunDirectory: View {
    @Environment(WorkshopModel.self) private var model
    @State private var isChoosing = false

    var body: some View {
        ContentUnavailableView {
            Label("Not Connected to LMAS", systemImage: "bolt.horizontal.circle")
        } description: {
            Text("Choose the folder LMAS runs in, the one with ui.sock inside.")
        } actions: {
            Button("Choose Folder…") { isChoosing = true }
                .buttonStyle(.glassProminent)
        }
        .fileImporter(isPresented: $isChoosing, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result { model.choose(runDirectory: url) }
        }
    }
}

/// Sync problems, and not being connected. Not red: that is for what waits on the user.
struct StatusBanner: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        switch model.status {
        case .stopped(let why):
            Banner(symbol: "exclamationmark.triangle.fill", tint: .orange, text: why.text) {
                if model.canRegisterAgain {
                    RegisterAgainButton()
                } else {
                    Button("Reload All Events") { model.reload() }
                }
                Button("Reconnect") { model.reconnect() }
            }
        case .retrying(let why):
            Banner(symbol: "bolt.horizontal.circle", tint: .secondary, text: "Reconnecting. \(why.text)") {
                Button("Try Now") { model.reconnect() }
            }
        case nil, .connecting, .needsRegistration, .registering, .catchingUp, .live:
            EmptyView()
        }
    }
}

struct NoticeBanner: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        if let notice = model.notice {
            Banner(symbol: "info.circle", tint: .secondary, text: notice.text) {
                Button("Dismiss") { model.notice = nil }
            }
        }
    }
}

/// Over the page, not part of it (`FloatingTop`).
struct Banner<Actions: View>: View {
    let symbol: String
    let tint: Color
    let text: String
    @ViewBuilder let actions: Actions

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol).foregroundStyle(tint)
            Text(verbatim: UntrustedText.line(text, limit: 400))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            actions.controlSize(.small)
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        // Over the page and over the photo, so it carries more body than the panes do.
        .glassBehind(.regular, in: .rect(cornerRadius: 12))
        .vetroEdge(in: RoundedRectangle(cornerRadius: 12))
        // A click on it does nothing, and does not reach what is under it.
        .contentShape(.rect)
        .onTapGesture {}
        .frame(maxWidth: 780)
        .padding(.horizontal, 16)
        .padding(.top, 8)
    }
}

struct WorkshopSettings: View {
    @Environment(WorkshopModel.self) private var model
    @State private var isChoosing = false
    @AppStorage(FinishedNotifier.isOnKey) private var notifiesWhenFinished = FinishedNotifier.isOnByDefault

    var body: some View {
        Form {
            Section {
                LabeledContent("LMAS folder") {
                    VStack(alignment: .trailing, spacing: 6) {
                        Text(verbatim: model.runDirectory?.path ?? "None")
                            .font(.body.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                        Button("Choose…") { isChoosing = true }
                    }
                }
            } footer: {
                Text("The folder LMAS was started with. Bottega connects to ui.sock in it, and has no other way in.")
                    .foregroundStyle(.secondary)
            }
            Section {
                Toggle("Notify when work finishes", isOn: $notifiesWhenFinished)
                    .onChange(of: notifiesWhenFinished) { _, isOn in if isOn { model.notifier.askPermission() } }
            } footer: {
                Text("Only while Bottega is not in front, and only how many: a card done or failed, or a turn you took part in. No line, card or words.")
                    .foregroundStyle(.secondary)
            }
            if let overview = model.overview {
                Section("LMAS") {
                    LabeledContent("Version") { Text(verbatim: UntrustedText.line(overview.lmas, limit: 60)).monospaced() }
                    if let cards = overview.cards {
                        LabeledContent("Repository") { Text(verbatim: UntrustedText.line(cards.repo, limit: 200)).monospaced() }
                        LabeledContent("Agents") { Text(verbatim: cards.vendors.map { UntrustedText.line($0, limit: 30) }.joined(separator: ", ")) }
                        if let into = cards.mergeInto {
                            LabeledContent("Merges into") { Text(verbatim: UntrustedText.line(into, limit: 80)).monospaced() }
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fixedSize(horizontal: false, vertical: true)
        .fileImporter(isPresented: $isChoosing, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result { model.choose(runDirectory: url) }
        }
    }
}

#if DEBUG
/// `BOTTEGA_PANELS=1`: what the backdrop popover holds, and the arrange popover of the line that is open, drawn in
/// the window for a screenshot. A popover is a window of its own, which a picture of this one leaves out.
private struct PanelsForAScreenshot: View {
    @Environment(WorkshopModel.self) private var model
    static let isAsked = ProcessInfo.processInfo.environment["BOTTEGA_PANELS"] != nil

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            BackdropPanel().glassBehind(.regular, in: .rect(cornerRadius: 14))
            if let id = model.page.line, let line = model.state.threads[id] {
                ArrangePanel(line: line).glassBehind(.regular, in: .rect(cornerRadius: 14))
            }
        }
        .padding(.top, 90)
    }
}

/// `BOTTEGA_WINDOW=860x700`: the window opens that size, for a screenshot of a narrow one. Asked through
/// `setContentSize`, which the window's own narrowest still holds for, as it does for a drag of its edge.
private enum WindowForAScreenshot {
    @MainActor static func resize() async {
        guard let asked = ProcessInfo.processInfo.environment["BOTTEGA_WINDOW"] else { return }
        let sides = asked.split(separator: "x").compactMap { Double($0) }
        guard sides.count == 2 else { return }
        // After the window has been put back where it last was, which comes a moment after its content.
        try? await Task.sleep(for: .milliseconds(600))
        NSApp.windows.first { $0.canBecomeMain }?.setContentSize(NSSize(width: sides[0], height: sides[1]))
    }
}
#endif
