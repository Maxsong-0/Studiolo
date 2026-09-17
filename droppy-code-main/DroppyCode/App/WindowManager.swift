import AppKit
import OSLog
import SwiftUI

/// Builds Droppy Code's windows the way Droppy builds its settings window: a plain titled AppKit window
/// with a clear background and a SwiftUI glass pane, so no scene background draws a second corner.
@MainActor
final class WindowManager {
    static let shared = WindowManager()

    let model = AppModel()

    private var mainWindow: NSWindow?
    private var settingsWindow: NSWindow?

    private enum Size {
        static let main = NSSize(width: 1320, height: 860)
        static let mainMinimum = NSSize(width: 860, height: 560)
        static let settings = NSSize(width: 780, height: 580)
    }

    private static let mainFrameName = "DroppyCodeMainWindow"

    func showMain() {
        let window = mainWindow ?? makeMainWindow()
        mainWindow = window
        present(window)
    }

    func showSettings(page: SettingsPage? = nil) {
        if let page { SettingsNavigation.shared.requestedPage = page }
        let window = settingsWindow ?? makeSettingsWindow()
        settingsWindow = window
        centerOverMainWindow(window)
        present(window)
    }

    /// Opens a window centered over the main window, kept on the screen the main window is on.
    private func centerOverMainWindow(_ window: NSWindow) {
        guard let main = mainWindow, main.isVisible, !main.isMiniaturized else {
            window.center()
            return
        }
        let parent = main.frame
        var frame = window.frame
        frame.origin.x = (parent.midX - frame.width / 2).rounded()
        frame.origin.y = (parent.midY - frame.height / 2).rounded()
        if let visible = (main.screen ?? NSScreen.main)?.visibleFrame {
            frame.origin.x = min(max(frame.origin.x, visible.minX), visible.maxX - frame.width)
            frame.origin.y = min(max(frame.origin.y, visible.minY), visible.maxY - frame.height)
        }
        window.setFrame(frame, display: false)
    }

    private func present(_ window: NSWindow) {
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
    }

    private func makeMainWindow() -> NSWindow {
        let window = makeWindow(title: AppInfo.name, size: Size.main, resizable: true, windowClass: ThreadWindow.self) { RootView() }
        (window as? ThreadWindow)?.selectThread = { [weak model] offset in model?.selectThread(offset: offset) }
        window.contentMinSize = Size.mainMinimum
        if !window.setFrameUsingName(Self.mainFrameName) {
            window.center()
        }
        window.setFrameAutosaveName(Self.mainFrameName)
        return window
    }

    /// The main window for the website captures: the same window, at a fixed size, with
    /// no frame autosave, so the run never reads or writes where the real window sits.
    func makeCaptureWindow(size: NSSize) -> NSWindow {
        makeWindow(title: AppInfo.name, size: size, resizable: true) { RootView() }
    }

    private func makeSettingsWindow() -> NSWindow {
        let window = makeWindow(title: "Settings", size: Size.settings, resizable: false, windowClass: SecondaryWindow.self) { SettingsView() }
        window.center()
        return window
    }

    private func makeWindow<Content: View>(
        title: String,
        size: NSSize,
        resizable: Bool,
        windowClass: NSWindow.Type = NSWindow.self,
        @ViewBuilder content: () -> Content
    ) -> NSWindow {
        var style: NSWindow.StyleMask = [.titled, .closable, .miniaturizable, .fullSizeContentView]
        if resizable { style.insert(.resizable) }
        let frame = NSRect(origin: .zero, size: size)
        let window = windowClass.init(contentRect: frame, styleMask: style, backing: .buffered, defer: false)
        window.title = title
        window.isReleasedWhenClosed = false
        window.animationBehavior = .documentWindow
        window.tabbingMode = .disallowed
        WindowChrome.configure(window)

        let liveResize = WindowLiveResize(window: window)
        let hosting = WindowHostingView(rootView: AnyView(HostedRoot(model: model, liveResize: liveResize, content: content())))
        hosting.frame = frame
        let container = NSView(frame: frame)
        container.addSubview(hosting)
        window.contentView = container
        return window
    }
}

/// Whether the window is being dragged to a new size right now. Views read it from the
/// environment to hold their settle animations and heavier layout work until the drag
/// ends: a spring restarted on every frame of a live resize is what made panels lag
/// behind the window and land somewhere else. Flipped by AppKit's live-resize
/// notifications for this one window, so a resize elsewhere leaves it alone.
@MainActor @Observable
final class WindowLiveResize {
    private(set) var isActive = false
    /// Whether the sidebar's edge or the terminal's divider is being dragged: the pane
    /// changes width or height every frame the way a live resize does, so the timeline
    /// steps its row width and holds its animations exactly as it does for one. Written
    /// by the chat view, which is where both drags are observed.
    var isPaneResizing = false
    /// The window or a pane is being dragged to a new size right now.
    var isReshaping: Bool { isActive || isPaneResizing }

    /// The window keeps this alive for as long as it exists, so the observations are
    /// never taken down: a window is not made twice.
    init(window: NSWindow) {
        let center = NotificationCenter.default
        center.addObserver(forName: NSWindow.willStartLiveResizeNotification, object: window, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.isActive = true }
        }
        center.addObserver(forName: NSWindow.didEndLiveResizeNotification, object: window, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.isActive = false }
        }
    }
}

/// Shared environment for every hosted window, plus the app-wide appearance choice.
private struct HostedRoot<Content: View>: View {
    let model: AppModel
    let liveResize: WindowLiveResize
    let content: Content

    var body: some View {
        content
            .environment(model)
            .environment(liveResize)
            .buttonBorderShape(.capsule)
            .preferredColorScheme(model.settings.theme.spec.scheme)
            .modifier(ThemeTint(theme: model.settings.theme))
            .onChange(of: model.settings.theme, initial: true) { _, theme in
                NSApp.appearance = switch theme.spec.scheme {
                case nil: nil
                case .light: NSAppearance(named: .aqua)
                case .dark: NSAppearance(named: .darkAqua)
                @unknown default: nil
                }
            }
    }
}

/// Tints prominent glass buttons, toggles and progress views with the
/// theme's accent. System/Light/Dark keep the system control accent.
private struct ThemeTint: ViewModifier {
    let theme: AppTheme

    func body(content: Content) -> some View {
        // One branch, never two: an `if` here made the whole window's content a different
        // view once a theme with an accent replaced one without, so every page was built
        // afresh and Settings jumped back to its top on a theme change. A nil tint keeps
        // the system accent.
        content.tint(theme.spec.accent)
    }
}

/// Lets the window frame define the hosting view's size instead of the content hugging it.
private final class WindowHostingView: NSHostingView<AnyView> {
    required init(rootView: AnyView) {
        super.init(rootView: rootView)
        sizingOptions = []
        safeAreaRegions = []
        translatesAutoresizingMaskIntoConstraints = true
        autoresizingMask = [.width, .height]
    }

    @MainActor required dynamic init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    /// Every window here holds focusable text (the composer, a search field), so the
    /// answer is always yes. SwiftUI's own answer searches the whole view tree for a
    /// navigable item, and AppKit asks on every display cycle while content scrolls
    /// under the title bar to work out the window's drag region.
    override var acceptsFirstResponder: Bool { true }

    /// SwiftUI marks the hosting view dirty by raising these two flags. AppKit refuses
    /// the raise while the window is inside its own display cycle, and on macOS 26 the
    /// refusal is an exception that ends the app: after layout, the window works out its
    /// drag region by asking this view for its opaque content (the title bar sits over
    /// it), SwiftUI answers by evaluating the view graph, and a scroll view whose
    /// geometry the layout just changed publishes it with a fresh transaction, right
    /// there inside the ask (the Sept 16 2026 crash, `_postWindowNeedsUpdateConstraints`
    /// under `_resetDragMargins`). The flag is raised again on the next run loop turn
    /// instead, once the cycle is over; nothing is lost but a frame.
    override var needsUpdateConstraints: Bool {
        get { super.needsUpdateConstraints }
        set { raiseFlag(newValue, retrying: false) { self.setNeedsUpdateConstraintsUnguarded($0) } }
    }

    override var needsLayout: Bool {
        get { super.needsLayout }
        set { raiseFlag(newValue, retrying: false) { self.setNeedsLayoutUnguarded($0) } }
    }

    private func setNeedsUpdateConstraintsUnguarded(_ value: Bool) { super.needsUpdateConstraints = value }
    private func setNeedsLayoutUnguarded(_ value: Bool) { super.needsLayout = value }

    private func raiseFlag(_ value: Bool, retrying: Bool, _ set: @escaping (Bool) -> Void) {
        guard let exception = DCCatchException({ set(value) }) else { return }
        guard !retrying else {
            Self.log.error("Display flag refused twice: \(exception.name.rawValue, privacy: .public) \(exception.reason ?? "", privacy: .public)")
            return
        }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            // Lowered first: AppKit may have kept the view's own flag from the refused
            // raise, and a raise that finds it already up tells the window nothing.
            _ = DCCatchException { set(false) }
            self.raiseFlag(value, retrying: true, set)
        }
    }

    private static let log = Logger(subsystem: "iordv.droppycode", category: "window")
}
