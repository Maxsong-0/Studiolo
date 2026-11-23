// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct RootView: View {
    @Environment(AppModel.self) private var model
    @State private var detailSize: CGSize = .zero

    var body: some View {
        let sidebar = model.sidebar
        let rightBar = model.rightSidebar
        // With the sidebar floating, the collapsed column hands its list to a panel over
        // the chat rather than the toolbar popover; only-floating drops the column altogether.
        let onlyFloats = model.settings.sidebarFloats && model.settings.sidebarOnlyFloats
        let floats = model.settings.sidebarFloats && (onlyFloats || !sidebar.isVisible)
        HStack(spacing: 0) {
            // The rows are laid out at the sidebar's width once, and the slide moves the clip
            // alone, so no row re-wraps per frame.
            SidebarView()
                .frame(width: sidebar.width)
                .frame(width: onlyFloats ? 0 : sidebar.renderedWidth, alignment: .trailing)
                .clipped()
                .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { sidebar.noteLaidOutWidth($0) }
                .overlay(alignment: .trailing) {
                    SidebarResizeHandle(
                        isActive: !onlyFloats && sidebar.isVisible,
                        onBegin: { sidebar.beginDrag() },
                        onChange: { sidebar.drag(by: $0) },
                        onEnd: { sidebar.endDrag() }
                    )
                    .frame(width: SidebarResizeHandle.hitWidth)
                    .padding(.top, Chrome.trafficLightTop + Chrome.trafficLightDiameter)
                }

            DetailView()
                .frame(minWidth: 0, maxWidth: .infinity)
                // The sheet keeps to its own bounds. A conversation gliding sideways to make
                // room for a panel (`ReserveSlide` carries it on an offset), a panel settling
                // into a corner of a pane that just changed size, and rows re-wrapping in a
                // live resize all reach past the sheet for a few frames, and drew over the
                // sidebar or off the window's edge until they landed.
                .clipped()
                .padding(.leading, sidebar.isVisible && !onlyFloats ? 0 : Chrome.sheetInset)
                .padding(.trailing, Chrome.sheetInset)
                .padding(.vertical, Chrome.sheetInset)
                .onGeometryChange(for: CGSize.self, of: { $0.size }) { detailSize = $0 }
                .overlay(alignment: .topLeading) {
                    if floats, detailSize != .zero {
                        FloatingSidebarPanel(area: detailSize, canClose: !onlyFloats) { sidebar.toggle() }
                            .transition(.scale(scale: 0.94).combined(with: .opacity))
                    }
                }
                .animation(Chrome.panelSlide, value: floats)

            // The third column, flush with the window's trailing edge: laid out at its own
            // width once and clipped, so the slide moves the clip alone and no row inside
            // re-wraps per frame — the same trick the list on the left plays.
            RightSidebarView()
                .frame(width: rightBar.width)
                .frame(width: rightBar.renderedWidth, alignment: .leading)
                .clipped()
                .overlay(alignment: .leading) {
                    SidebarResizeHandle(
                        isActive: rightBar.isVisible,
                        onBegin: { rightBar.beginDrag() },
                        onChange: { rightBar.drag(by: $0) },
                        onEnd: { rightBar.endDrag() }
                    )
                    .frame(width: SidebarResizeHandle.hitWidth)
                    .padding(.top, Chrome.trafficLightTop + Chrome.trafficLightDiameter)
                }
        }
        // While the right sidebar is closed, dragging the window's trailing edge pulls it out.
        .overlay(alignment: .trailing) {
            SidebarResizeHandle(
                isActive: !rightBar.isVisible,
                onBegin: { rightBar.beginDrag() },
                onChange: { rightBar.drag(by: $0) },
                onEnd: { rightBar.endDrag() }
            )
            .frame(width: SidebarResizeHandle.hitWidth)
            .padding(.top, Chrome.trafficLightTop + Chrome.trafficLightDiameter)
        }
        // While the sidebar is hidden, dragging the window's leading edge pulls it back out.
        .overlay(alignment: .leading) {
            SidebarResizeHandle(
                isActive: !onlyFloats && !sidebar.isVisible,
                onBegin: { sidebar.beginDrag() },
                onChange: { sidebar.drag(by: $0) },
                onEnd: { sidebar.endDrag() }
            )
            .frame(width: SidebarResizeHandle.hitWidth)
            .padding(.top, Chrome.trafficLightTop + Chrome.trafficLightDiameter)
        }
        .background { WindowBackdrop(opacity: model.settings.backdropOpacity, wallpaper: WallpaperStore.shared.image) }
        .background { WindowChromeConfigurator(sidebarVisible: sidebar.holdsTrafficLights, sidebarDragging: sidebar.isDragging) }
        // Clipped after the backdrop is painted, so the glass and its edge stop at the window's own curve.
        .clipShape(RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous))
        .ignoresSafeArea()
        .coordinateSpace(.named(GenieAnimator.coordinateSpace))
        .overlay { GenieLayer() }
        .overlay { RowGlideLayer() }
        .overlay {
            if model.isCommandPalettePresented {
                CommandPalette()
            }
        }
        .dropDestination(for: URL.self) { urls, _ in
            handleDrop(urls)
        }
        .alert(
            model.alert?.title ?? "",
            isPresented: Binding(
                get: { model.alert != nil },
                set: { if !$0 { model.dismissAlert() } }
            ),
            presenting: model.alert
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { alert in
            Text(alert.message)
        }
        .modifier(StudioloSheets())
        // Outermost, so the overlays above (the palette, the floating sidebar) read the
        // text size too. The chat columns are hosted apart and set it for themselves.
        .modifier(WindowTypeEnvironment(settings: model.settings))
    }

    @discardableResult
    private func handleDrop(_ urls: [URL]) -> Bool {
        var folders: [URL] = []
        var files: [URL] = []
        for url in urls {
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else { continue }
            if isDirectory.boolValue { folders.append(url) } else { files.append(url) }
        }
        var lastProject: Project?
        for folder in folders { lastProject = model.addProject(at: folder) }
        if let lastProject { model.newThread(in: lastProject) }
        if !files.isEmpty, let threadID = model.selectedThreadID {
            Storage.attach(files.map(AttachmentSource.file), to: Bindable(model.runtime(for: threadID)).draft.attachments)
        }
        return !folders.isEmpty || !files.isEmpty
    }
}

/// Whether the chat column a view sits in is the one on screen. The columns of the threads
/// visited last stay built behind the selected one (see `ThreadColumns`), so a view that
/// would react to appearing or disappearing reacts to this instead.
extension EnvironmentValues {
    @Entry var chatColumnIsShown = true
}

struct DetailView: View {
    @Environment(AppModel.self) private var model
    @Environment(WindowLiveResize.self) private var liveResize
    /// Threads whose columns are built behind the selection, ahead of a click on them.
    @State private var prebuilt: [UUID] = []
    @State private var hasPrebuiltTopRows = false
    /// `kCGAnyInputEventType`, which Quartz leaves out of the Swift enum.
    private static let anyInput = CGEventType(rawValue: ~0)!

    var body: some View {
        if let prefix = model.selectedArchivePrefix {
            ArchivePage(prefix: prefix)
                .id(prefix)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .detailSheet()
        } else if let threadID = model.selectedThreadID, model.thread(threadID) != nil {
            let running = Set(model.threads.compactMap { model.existingRuntime(for: $0.id)?.isRunning == true ? $0.id : nil })
            ThreadColumns(threadID: threadID, prebuilt: prebuilt, running: running, model: model, liveResize: liveResize)
                .onChange(of: threadID) { _, _ in prebuilt = [] }
                .task(id: threadID) {
                    // The neighbours, and at launch the sidebar's top rows, one at a time, once
                    // the reader has settled on the thread and nothing streams in it: each is a
                    // frame's worth of building.
                    try? await Task.sleep(for: .milliseconds(1500))
                    var candidates = model.neighbors(of: threadID)
                    if !hasPrebuiltTopRows {
                        hasPrebuiltTopRows = true
                        candidates += model.sidebarThreads.prefix(ThreadColumns.topRowsBuiltAtLaunch).map(\.id)
                    }
                    for id in candidates where !Task.isCancelled {
                        // Never under the reader's hands: a build holds the main thread for a
                        // frame or a few, which a keystroke would wait on.
                        while !Task.isCancelled, CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: Self.anyInput) < 1 {
                            try? await Task.sleep(for: .milliseconds(300))
                        }
                        guard !Task.isCancelled, !(model.existingRuntime(for: threadID)?.isRunning ?? false) else { return }
                        if id != threadID, !prebuilt.contains(id) { prebuilt.append(id) }
                        try? await Task.sleep(for: .milliseconds(400))
                    }
                }
        } else {
            Group {
                if model.projects.isEmpty {
                    WelcomeView()
                } else {
                    NoThreadView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .detailSheet()
        }
    }
}

/// The chat column, hosted on its own per thread. The columns of the threads visited last
/// stay in the pane, hidden, so switching back to one is a change of visibility rather
/// than a rebuild of the timeline, the composer and the panels: the rows of a long thread
/// took a quarter of a second to build each time. Hosting each column apart also keeps
/// every view tree small: SwiftUI walks a tree whole whenever a focusable view appears in
/// it or the pointer moves over it. Hidden rather than detached, because a hosting view
/// taken out of the window disappears to its views, and every task of the column starts
/// over when it comes back.
private struct ThreadColumns: NSViewRepresentable {
    /// How many of the sidebar's top rows are built behind the first thread shown.
    static let topRowsBuiltAtLaunch = 6

    let threadID: UUID
    let prebuilt: [UUID]
    /// Threads with a turn under way. A hidden column of one goes: its rows would stream
    /// unseen, at the same cost as on screen.
    let running: Set<UUID>
    let model: AppModel
    let liveResize: WindowLiveResize

    func makeNSView(context: Context) -> NSView {
        NSView()
    }

    func updateNSView(_ pane: NSView, context: Context) {
        let columns = context.coordinator
        columns.show(threadID, in: pane, model: model, liveResize: liveResize)
        columns.dropHidden(where: { running.contains($0) || model.thread($0) == nil || model.existingRuntime(for: $0) == nil })
        columns.prebuild(prebuilt.filter { model.thread($0) != nil && !running.contains($0) }, in: pane, model: model, liveResize: liveResize)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    @MainActor final class Coordinator {
        /// How many columns stay built behind the shown one.
        private static let keptLimit = 8
        /// Hidden columns keep the size they were built at and take the pane's size only
        /// when shown: resizing every hidden column per frame during a sidebar slide or
        /// live resize is what made them lag.
        private var columns: [UUID: FrameHostingView] = [:]
        /// The threads with a column, the one to drop first at the front and the shown one
        /// last. A column built ahead of a click goes in at the front: it is the first to
        /// make room for the columns the reader has been to.
        private var order: [UUID] = []
        private var shown: UUID?
        /// What was asked for ahead of a click, so a column dropped since is not built over
        /// and over on every update.
        private var requested: Set<UUID> = []
        /// Columns dropped, kept until the switch that dropped them has been drawn: taking
        /// a view tree down costs a frame's worth of time, better spent after the frame.
        private var retired: [FrameHostingView] = []
        private var retirement: Task<Void, Never>?

        func show(_ id: UUID, in pane: NSView, model: AppModel, liveResize: WindowLiveResize) {
            guard id != shown else { return }
            if let shown, let previous = columns[shown] {
                previous.isHidden = true
                previous.autoresizingMask = []
                previous.rootView = Self.root(shown, isShown: false, model: model, liveResize: liveResize)
            }
            if let kept = columns[id] {
                kept.rootView = Self.root(id, isShown: true, model: model, liveResize: liveResize)
                kept.frame = pane.bounds
                kept.autoresizingMask = [.width, .height]
                kept.isHidden = false
            } else {
                make(id, isShown: true, in: pane, model: model, liveResize: liveResize)
            }
            shown = id
            order.removeAll { $0 == id }
            order.append(id)
            while order.count > Self.keptLimit + 1 { drop(order[0]) }
        }

        /// Drops the hidden columns the test names; each is built afresh when its thread is
        /// next selected.
        func dropHidden(where goes: (UUID) -> Bool) {
            for id in Array(columns.keys) where id != shown && goes(id) { drop(id) }
        }

        /// Builds the columns asked for, hidden and laid out at the pane's size, so a click
        /// on one of the threads finds its rows made. Each is built once per request: one
        /// dropped to make room for the columns visited since stays dropped.
        func prebuild(_ ids: [UUID], in pane: NSView, model: AppModel, liveResize: WindowLiveResize) {
            for id in ids where !requested.contains(id) && columns[id] == nil {
                let started = ContinuousClock.now
                make(id, isShown: false, in: pane, model: model, liveResize: liveResize).layoutSubtreeIfNeeded()
                SwitchLatency.note("prebuilt \(model.thread(id)?.title.prefix(24) ?? "?") in \(started.duration(to: .now))")
                order.insert(id, at: 0)
                while order.count > Self.keptLimit + 1 { drop(order[0]) }
            }
            requested = Set(ids)
        }

        @discardableResult
        private func make(_ id: UUID, isShown: Bool, in pane: NSView, model: AppModel, liveResize: WindowLiveResize) -> FrameHostingView {
            let column = FrameHostingView(rootView: Self.root(id, isShown: isShown, model: model, liveResize: liveResize))
            column.frame = pane.bounds
            column.isHidden = !isShown
            column.autoresizingMask = isShown ? [.width, .height] : []
            pane.addSubview(column)
            columns[id] = column
            return column
        }

        private static func root(_ id: UUID, isShown: Bool, model: AppModel, liveResize: WindowLiveResize) -> AnyView {
            AnyView(
                ChatView(runtime: model.runtime(for: id))
                    .environment(\.chatColumnIsShown, isShown)
                    // The text size and the faces from Settings, for the whole column: the
                    // conversation, the chrome row, the chat box, the panels and the terminal.
                    // Hosted on its own, the column never sees the window root's.
                    .modifier(WindowTypeEnvironment(settings: model.settings))
                    .modifier(HostedEnvironment(model: model, liveResize: liveResize))
            )
        }

        private func drop(_ id: UUID) {
            order.removeAll { $0 == id }
            guard let column = columns.removeValue(forKey: id) else { return }
            retired.append(column)
            retirement?.cancel()
            retirement = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled, let self else { return }
                for column in retired { column.removeFromSuperview() }
                retired.removeAll()
            }
        }
    }
}

struct WelcomeView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        ScrollView {
            VStack(spacing: 30) {
                VStack(spacing: 14) {
                    Image(nsImage: NSApp.applicationIconImage)
                        .resizable()
                        .frame(width: 104, height: 104)
                    Text("Bottega")
                        .zoomedFont(size: 34, weight: .semibold)
                    Text("A calm, native home for your coding agents.")
                        .zoomedFont(size: 15)
                        .foregroundStyle(Chrome.secondaryText)
                }
                VStack(spacing: 10) {
                    Button {
                        model.chooseProjectFolder()
                    } label: {
                        Label("Add a project", systemImage: "folder.badge.plus")
                            .padding(.horizontal, 6)
                    }
                    .buttonStyle(.glassProminent)
                    .controlSize(.extraLarge)
                    Text("Or drop a folder anywhere in this window.")
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.secondaryText)
                }
                ChromeSection(title: "Activity list") {
                    ChromeCard {
                        ChromeRow(title: "Project mark", detail: settings.activityThreadStyle.detail) {
                            ChromeVisualPicker(options: ActivityThreadStyle.allCases.map { ($0, $0.title) }, selection: $settings.activityThreadStyle) { style in
                                Image(systemName: style == .icon ? "list.bullet" : "text.alignleft")
                            }
                        }
                    }
                }
                .frame(maxWidth: 480)
                ChromeSection(title: "Providers") {
                    ProviderChecklist()
                }
                .frame(maxWidth: 480)
            }
            .padding(48)
            .frame(maxWidth: .infinity)
        }
        .scrollIndicators(.never)
        .scrollBounceBehavior(.basedOnSize)
    }
}

struct ProviderChecklist: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        // The providers switched on in Settings, and only those: launch checks nothing about
        // the others (see `ProviderRegistry.refreshAll`), and listing them here would offer
        // to install what was put aside on purpose.
        let providers = ProviderKind.allCases.filter { model.settings.isEnabled($0) }
        ChromeCard {
            if providers.isEmpty {
                HStack(spacing: 12) {
                    Text("Every provider is switched off.")
                        .zoomedFont(size: 13)
                        .foregroundStyle(Chrome.secondaryText)
                    Spacer()
                    Button("Open Settings") { WindowManager.shared.showSettings(page: .providers) }
                        .buttonStyle(.glass)
                        .help("Open Settings > Providers")
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 11)
            }
            ForEach(Array(providers.enumerated()), id: \.element) { index, provider in
                if index > 0 { ChromeRowDivider() }
                ProviderStatusRow(provider: provider)
            }
        }
    }
}

private struct ProviderStatusRow: View {
    @Environment(AppModel.self) private var model
    let provider: ProviderKind

    var body: some View {
        let status = model.providers.status(provider)
        HStack(spacing: 12) {
            ProviderIcon(provider: provider, size: 18)
                .foregroundStyle(Chrome.primaryText)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: provider.displayName)
                    .zoomedFont(size: 13)
                Text(verbatim: status.isChecking ? "Checking…" : status.summary)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer()
            if status.isChecking {
                ProgressView()
                    .controlSize(.small)
            } else if !status.isInstalled {
                if provider.isAPIKeyBased {
                    // Nothing to install: the key is typed in Settings, so the row opens it
                    // rather than sending the reader to a web page for it.
                    OpenProvidersSettingsButton()
                } else {
                    Link("Install", destination: provider.installURL)
                        .buttonStyle(.glass)
                }
            } else if status.auth == .signedOut {
                if provider.isAPIKeyBased {
                    OpenProvidersSettingsButton()
                } else {
                    Button("Login") { Workspace.openTerminal(running: provider.loginCommand) }
                        .buttonStyle(.glass)
                }
            } else {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(Chrome.success)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
    }
}

/// Takes the reader to the Providers page, where a key-based provider's key is typed.
private struct OpenProvidersSettingsButton: View {
    var body: some View {
        Button("Add key") {
            WindowManager.shared.showSettings()
            SettingsNavigation.shared.requestedPage = .providers
        }
        .buttonStyle(.glass)
        .help("Open Settings > Providers")
    }
}

struct NoThreadView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "bubble.left.and.text.bubble.right")
                .zoomedFont(size: 46, weight: .light)
                .foregroundStyle(Chrome.secondaryText)
            Text("Pick a thread or start a new one")
                .zoomedFont(size: 17, weight: .medium)
            Button {
                model.newThread()
            } label: {
                Label("New thread", systemImage: "square.and.pencil")
            }
            .buttonStyle(.glassProminent)
            .controlSize(.large)
            // The chord as the store has it now, so a remap never leaves a stale key here.
            if let chord = ShortcutStore.label(for: .newThread) {
                Text(verbatim: chord)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
