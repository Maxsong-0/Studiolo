// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import SwiftUI

/// The window: sidebar, the page, and the inspector.
struct WorkshopView: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            WorkshopSidebar()
                .navigationSplitViewColumnWidth(min: 210, ideal: 250, max: 360)
        } detail: {
            PageView()
                .frame(minWidth: 420)
                .inspector(isPresented: $model.showsInspector) {
                    WorkshopInspector()
                        .inspectorColumnWidth(min: 260, ideal: 320, max: 520)
                }
        }
        .sheet(item: $model.askBeingRead) { item in
            AskSheet(id: item.id)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.reportSeen()
        }
    }
}

/// What the middle shows: the page picked, or what has to be done before there is one.
struct PageView: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        VStack(spacing: 0) {
            StatusBanner()
            NoticeBanner()
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .navigationTitle(title)
        .navigationSubtitle(subtitle)
        .toolbar {
            if case .line = model.page {
                ToolbarItem {
                    @Bindable var model = model
                    Picker("View", selection: $model.showsLanes) {
                        Label("Timeline", systemImage: "text.alignleft").tag(false)
                        Label("Lanes", systemImage: "arrow.left.arrow.right").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .help("Show this line as a timeline or as letters between agents")
                }
            }
            ToolbarItem {
                Button("Inspector", systemImage: "sidebar.trailing") { model.showsInspector.toggle() }
                    .help(model.showsInspector ? "Hide the inspector" : "Show the inspector")
            }
        }
    }

    @ViewBuilder private var content: some View {
        if model.source == .none {
            ChooseRunDirectory()
        } else if model.needsRegistration {
            RegistrationView()
        } else {
            switch model.page {
            case .dispatcher:
                TimelinePage(rows: model.state.dispatcherRows, key: .dispatcher, empty: Self.emptyDispatcher)
            case .waiting: WaitingView()
            case .board: BoardView()
            case .activity: ActivityView()
            case .devices: DevicesView()
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

    private var title: String {
        if case .line(let id) = model.page, let line = model.state.threads[id] { return model.state.title(of: line) }
        return model.page.fixedTitle
    }

    private var subtitle: String {
        switch model.page {
        case .line(let id):
            guard let line = model.state.threads[id] else { return "" }
            return "\(Wording.lineKind(model.state.kind(of: line))) · \(UntrustedText.line(id, limit: 40))"
        case .dispatcher, .waiting, .board, .activity, .devices:
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
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.quaternary.opacity(0.5))
        .overlay(alignment: .bottom) { Divider() }
    }
}

struct WorkshopSettings: View {
    @Environment(WorkshopModel.self) private var model
    @State private var isChoosing = false

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
