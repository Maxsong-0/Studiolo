import AppKit
import SwiftUI

/// Sets up the hosting window the way Droppy's settings window is built: a transparent title bar over
/// a clear window, a SwiftUI glass pane underneath, and AppKit's own traffic lights pinned to the chrome,
/// with the title bar kept reaching down to them so they take their clicks wherever they are pinned.
struct WindowChromeConfigurator: NSViewRepresentable {
    var sidebarVisible = true
    var sidebarDragging = false

    func makeNSView(context: Context) -> WindowChromeProbeView {
        WindowChromeProbeView()
    }

    func updateNSView(_ nsView: WindowChromeProbeView, context: Context) {
        nsView.sidebarDragging = sidebarDragging
        nsView.sidebarVisible = sidebarVisible
    }
}

final class WindowChromeProbeView: NSView {
    var sidebarDragging = false
    var sidebarVisible = true {
        didSet {
            guard sidebarVisible != oldValue, let window else { return }
            // A restarted animation every frame of a live resize is what made the
            // buttons lag behind the window; they follow unanimated, then settle.
            WindowChrome.placeTrafficLights(on: window, sidebarVisible: sidebarVisible, animated: !window.inLiveResize && !sidebarDragging)
        }
    }

    /// The title bar whose height this view keeps (see `WindowChrome.fitTitlebar`), or nil until
    /// the view is in a window.
    private weak var titlebarContainer: NSView?
    /// The window the didEndLiveResize observer belongs to, so moving windows never stack them.
    private weak var resizeWindow: NSWindow?
    /// A fit is queued for after AppKit's current layout pass.
    private var titlebarFitPending = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let titlebarContainer {
            NotificationCenter.default.removeObserver(self, name: NSView.frameDidChangeNotification, object: titlebarContainer)
            self.titlebarContainer = nil
        }
        if let resizeWindow {
            NotificationCenter.default.removeObserver(self, name: NSWindow.didEndLiveResizeNotification, object: resizeWindow)
            self.resizeWindow = nil
        }
        guard let window else { return }
        WindowChrome.configure(window)
        WindowChrome.placeTrafficLights(on: window, sidebarVisible: sidebarVisible, animated: false)
        // The probe's previous window keeps its own observer; moving windows register fresh.
        NotificationCenter.default.addObserver(
            self, selector: #selector(windowDidEndLiveResize), name: NSWindow.didEndLiveResizeNotification, object: window
        )
        resizeWindow = window
        // AppKit tiles the title bar back to its own height when the window resizes, and with
        // that the buttons level with the chrome are out of its reach again. It says so through
        // the container's frame, and the fit goes back on.
        guard let container = WindowChrome.titlebarContainer(of: window) else { return }
        container.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(titlebarFrameDidChange), name: NSView.frameDidChangeNotification, object: container
        )
        titlebarContainer = container
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    /// Runs inside AppKit's own tiling of the title bar; the fit waits for it to finish, so the
    /// container is never resized halfway through a pass that is laying out its subviews.
    /// Cheap per frame during a live resize: a container that already has the wanted
    /// height queues nothing, and the fit itself never lays out or draws synchronously.
    @objc private func titlebarFrameDidChange(_ notification: Notification) {
        guard let container = notification.object as? NSView,
              container.frame.height < WindowChrome.titlebarHeight,
              !titlebarFitPending else { return }
        titlebarFitPending = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            titlebarFitPending = false
            if let window { WindowChrome.fitTitlebar(of: window) }
        }
    }

    /// One final unanimated placement once the window settles, so the buttons end up
    /// correct after following the drag without animations.
    @objc private func windowDidEndLiveResize(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        WindowChrome.placeTrafficLights(on: window, sidebarVisible: sidebarVisible, animated: false)
        WindowChrome.fitTitlebar(of: window)
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

@MainActor
enum WindowChrome {
    private static let leadingIdentifier = "droppycode.trafficLight.leading"
    private static let topIdentifier = "droppycode.trafficLight.top"
    /// Windows `configure` has already set up. Weak, so a closed window drops out on its own.
    private static let configuredWindows = NSHashTable<NSWindow>.weakObjects()

    static func configure(_ window: NSWindow) {
        // Idempotent: viewDidMoveToWindow fires on every reorder, so the setup pays once.
        if configuredWindows.contains(window) { return }
        configuredWindows.add(window)
        window.styleMask.insert(.fullSizeContentView)
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        // A focused window draws a separator under the title bar that cuts through the rounded corner.
        window.titlebarSeparatorStyle = .none
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        window.isMovableByWindowBackground = false
        window.invalidateShadow()
        // AppKit owns the key view loop, rebuilt only when Tab navigation asks for it.
        // Left off, SwiftUI rebuilds the loop itself, synchronously, every time a
        // focusable view or an AppKit view in the window changes shape: it walks every
        // focus item in reading order, and on macOS 26 that walk never ended inside the
        // composer's follow-up queue tab, so the main thread stood still until a force
        // quit (the Sept 15 2026 freezes, `FocusBridge.updateDefaultKeyViewLoop` at the
        // top of every sample). With this on, SwiftUI only marks the loop dirty and keeps
        // its focus items by the finite, group-based route; nothing is rebuilt during
        // layout. Full Keyboard Access is what makes Tab move between controls at all,
        // and the loop is still rebuilt for it, just on demand.
        window.autorecalculatesKeyViewLoop = true
    }

    /// Where the buttons sit with the sidebar hidden: level with the chat's chrome capsules,
    /// which then step aside for them, and inset like the chrome is from the sheet's edge.
    static var collapsedTrafficLightTop: CGFloat {
        Chrome.sheetInset + Chrome.chromeTopPadding + (Chrome.capsuleHeight - Chrome.trafficLightDiameter) / 2
    }

    /// How far down the title bar reaches: to the bottom of the chrome row, so the buttons are
    /// inside it in either placement. AppKit only hands a click or a hover to a button within the
    /// title bar's own bounds, and its own bar stops short of buttons level with the chrome; the
    /// clicks fell through to the pane underneath. Being transparent, the bar shows nothing of
    /// its size; what changes is that the row's gaps drag the window, as a title bar's do, while
    /// the controls on it never do. Every chrome control carries a `NoWindowDragArea` behind its
    /// content (and native controls refuse the drag on their own), so a press starting on the zoom
    /// slider or any button, menu or field works only that control and leaves the window in place.
    static var titlebarHeight: CGFloat {
        Chrome.sheetInset + Chrome.chromeTopPadding + Chrome.capsuleHeight
    }

    /// The view AppKit sizes the title bar with: the buttons' bar and its accessories together.
    static func titlebarContainer(of window: NSWindow) -> NSView? {
        window.standardWindowButton(.closeButton)?.superview?.superview
    }

    /// Reaches the title bar down to `titlebarHeight`, keeping its top edge where it is. The
    /// buttons' bar fills the container, so it grows with it and the pinned buttons come inside.
    /// Never lays out synchronously: the fit lands on the frame and AppKit's own pass lays
    /// it out, so a notification fired mid-tiling costs one frame assignment.
    static func fitTitlebar(of window: NSWindow) {
        guard let container = titlebarContainer(of: window), let themeFrame = container.superview else { return }
        var fitted = container.frame
        guard fitted.height < titlebarHeight else { return }
        if themeFrame.isFlipped {
            fitted.size.height = titlebarHeight
        } else {
            fitted.origin.y = fitted.maxY - titlebarHeight
            fitted.size.height = titlebarHeight
        }
        container.frame = fitted
    }

    /// Pins the native buttons with constraints, since the title bar's own layout pass resets
    /// frames. Beside the sidebar they sit in its top corner; with it hidden they move over
    /// to the chat's chrome row and down to its centre line, with the slide the chrome makes.
    static func placeTrafficLights(on window: NSWindow, sidebarVisible: Bool, animated: Bool) {
        let leading = sidebarVisible ? Chrome.trafficLightLeading : Chrome.sheetInset + Chrome.chromeHorizontalPadding
        let top = sidebarVisible ? Chrome.trafficLightTop : collapsedTrafficLightTop
        guard let titlebar = window.standardWindowButton(.closeButton)?.superview else { return }
        fitTitlebar(of: window)
        var changed = false
        let types: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
        for (index, type) in types.enumerated() {
            guard let button = window.standardWindowButton(type) else { continue }
            button.isHidden = false
            let x = leading + CGFloat(index) * Chrome.trafficLightPitch
            if button.translatesAutoresizingMaskIntoConstraints {
                button.translatesAutoresizingMaskIntoConstraints = false
                let leadingConstraint = button.leadingAnchor.constraint(equalTo: titlebar.leadingAnchor, constant: x)
                leadingConstraint.identifier = leadingIdentifier
                let topConstraint = button.topAnchor.constraint(equalTo: titlebar.topAnchor, constant: top)
                topConstraint.identifier = topIdentifier
                NSLayoutConstraint.activate([
                    leadingConstraint,
                    topConstraint,
                    button.widthAnchor.constraint(equalToConstant: Chrome.trafficLightDiameter),
                    button.heightAnchor.constraint(equalToConstant: Chrome.trafficLightDiameter),
                ])
                changed = true
            } else {
                for constraint in titlebar.constraints where constraint.firstItem as? NSView === button {
                    let target: CGFloat? = switch constraint.identifier {
                    case leadingIdentifier: x
                    case topIdentifier: top
                    default: nil
                    }
                    guard let target, constraint.constant != target else { continue }
                    constraint.constant = target
                    changed = true
                }
            }
        }
        guard changed else { return }
        // Never animated mid-resize: a fresh group every frame is what lagged the buttons.
        guard animated, !window.inLiveResize else {
            titlebar.layoutSubtreeIfNeeded()
            return
        }
        // The same run as Chrome.panelSlide, so the buttons and the chrome row that makes
        // room for them arrive together. A plain ease-out spends the run gliding; a curve
        // that is nearly done after its first few frames reads as the buttons popping over.
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        NSAnimationContext.runAnimationGroup { context in
            context.duration = reduceMotion ? 0.18 : 0.32
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            context.allowsImplicitAnimation = true
            titlebar.layoutSubtreeIfNeeded()
        }
    }
}

// MARK: - Sidebar layout

/// The sidebar's width and visibility. Dragging its edge resizes it, and dragging past the
/// collapse point hides it, the same interaction as Droppy's settings sidebar.
@MainActor
@Observable
final class SidebarLayout {
    static let minimumWidth: CGFloat = 210
    static let maximumWidth: CGFloat = 440
    static let collapseThreshold: CGFloat = 110
    static let defaultWidth: CGFloat = 268

    private enum Key {
        static let width = "sidebarWidth"
        static let startsOpen = "sidebarStartsOpen"
    }

    private(set) var width: CGFloat
    private(set) var isVisible: Bool
    /// Whether the sidebar is open when the app starts (Settings › Appearance). Collapsed
    /// by default: the list button in the toolbar shows and hides it any time, and that
    /// choice lasts for the session alone.
    var startsOpen: Bool {
        didSet { persist() }
    }
    /// Whether the sidebar, as laid out this frame, is wide enough to hold the window buttons
    /// in its top corner. Until it is (while it slides open, or is dragged out from the edge)
    /// they stay over the chat's chrome, and the chrome keeps their room; the moment it is,
    /// they move over and the chrome closes up, together.
    private(set) var holdsTrafficLights: Bool
    /// True while the sidebar's edge is held; the chat follows the width with no spring until it is let go.
    private(set) var isDragging = false

    /// The width from which the buttons fit beside the sidebar's edge with their usual clearance.
    static var trafficLightsFitWidth: CGFloat {
        Chrome.trafficLightLeading + Chrome.trafficLightsWidth + Chrome.trafficLightClearance
    }

    @ObservationIgnored private var anchorWidth: CGFloat = 0
    @ObservationIgnored private var restingWidth: CGFloat
    /// The width as last laid out, so a resize frame that changes nothing writes nothing.
    @ObservationIgnored private var lastLaidOutWidth: CGFloat = -1

    init() {
        let defaults = WebsiteCaptures.defaults ?? .standard
        let stored = (defaults.object(forKey: Key.width) as? Double).map { CGFloat($0) } ?? Self.defaultWidth
        restingWidth = min(Self.maximumWidth, max(Self.minimumWidth, stored))
        width = restingWidth
        let visible = defaults.object(forKey: Key.startsOpen) as? Bool ?? false
        startsOpen = visible
        isVisible = visible
        holdsTrafficLights = visible
    }

    var renderedWidth: CGFloat { isVisible ? width : 0 }

    /// The sidebar's width as laid out, reported every frame it changes, so the buttons follow
    /// the sidebar as it actually is on screen rather than the state it is heading for.
    /// Unchanged reports write nothing, so a height-only resize frame costs no state.
    func noteLaidOutWidth(_ laidOut: CGFloat) {
        guard laidOut != lastLaidOutWidth else { return }
        lastLaidOutWidth = laidOut
        let fits = laidOut >= Self.trafficLightsFitWidth
        if fits != holdsTrafficLights { holdsTrafficLights = fits }
    }

    func beginDrag() {
        isDragging = true
        if isVisible {
            anchorWidth = width
            return
        }
        anchorWidth = 0
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            width = 0
            isVisible = true
        }
    }

    func drag(by translation: CGFloat) {
        width = min(Self.maximumWidth, max(0, anchorWidth + translation))
    }

    func endDrag() {
        if width < Self.collapseThreshold {
            withAnimation(Chrome.panelSlide) { isVisible = false }
            width = restingWidth
        } else {
            let settled = max(Self.minimumWidth, width)
            restingWidth = settled
            withAnimation(Chrome.panelSlide) { width = settled }
        }
        persist()
        isDragging = false
    }

    func toggle() {
        if !isVisible { width = restingWidth }
        withAnimation(Chrome.panelSlide) { isVisible.toggle() }
        persist()
    }

    private func persist() {
        let defaults = WebsiteCaptures.defaults ?? .standard
        defaults.set(Double(restingWidth), forKey: Key.width)
        defaults.set(startsOpen, forKey: Key.startsOpen)
    }
}

// MARK: - Resize handle

/// The grab strip on the sidebar edge. AppKit gives it a real resize cursor and first-click tracking.
struct SidebarResizeHandle: NSViewRepresentable {
    static let hitWidth: CGFloat = 9

    let isActive: Bool
    let onBegin: () -> Void
    let onChange: (CGFloat) -> Void
    let onEnd: () -> Void

    func makeNSView(context: Context) -> SidebarResizeHandleView {
        let view = SidebarResizeHandleView(frame: .zero)
        update(view)
        return view
    }

    func updateNSView(_ nsView: SidebarResizeHandleView, context: Context) {
        update(nsView)
    }

    private func update(_ view: SidebarResizeHandleView) {
        view.onBegin = onBegin
        view.onChange = onChange
        view.onEnd = onEnd
        view.isActive = isActive
    }
}

final class SidebarResizeHandleView: NSView {
    var onBegin: (() -> Void)?
    var onChange: ((CGFloat) -> Void)?
    var onEnd: (() -> Void)?

    var isActive = true {
        didSet {
            guard isActive != oldValue else { return }
            window?.invalidateCursorRects(for: self)
            refreshHover()
            updateGrip(animated: false)
        }
    }

    private enum Grip {
        static let width: CGFloat = 2.5
        static let restHeight: CGFloat = 10
        static let shownHeight: CGFloat = 22
        static let alpha: Float = 0.6
        static let hoverGlow: Float = 0.05
        static let dragGlow: Float = 0.10
        static let duration: CFTimeInterval = 0.18
        static let verticalInset = Chrome.trafficLightTop + Chrome.trafficLightDiameter
    }

    private var pressOriginX: CGFloat = 0
    private var trackingArea: NSTrackingArea?
    private var isHovering = false
    private var isDragging = false
    private let gripLayer = CALayer()
    private let glowLayer = CALayer()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        configureLayers()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configureLayers()
    }

    private func configureLayers() {
        wantsLayer = true
        gripLayer.cornerRadius = Grip.width / 2
        gripLayer.opacity = 0
        glowLayer.cornerRadius = 4
        glowLayer.opacity = 0
        applyColors()
        layer?.addSublayer(glowLayer)
        layer?.addSublayer(gripLayer)
    }

    private func applyColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let color = NSColor.labelColor.cgColor
            gripLayer.backgroundColor = color
            glowLayer.backgroundColor = color
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyColors()
    }

    override func layout() {
        super.layout()
        // The strip moves with the sidebar's edge: a toggle, or a release that springs the
        // width back, carries it out from under a resting cursor, and AppKit only reports
        // enter and exit for the mouse moving. Each new frame asks where the cursor is now.
        let hovered = isHovering
        refreshHover()
        updateGrip(animated: hovered != isHovering)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        refreshHover()
        updateGrip(animated: false)
    }

    private var showsGrip: Bool { isActive && (isHovering || isDragging) }

    /// Reads the cursor's actual place, so `isHovering` never outlives the cursor being over
    /// the strip; the entered/exited pair only keeps it true while the mouse itself moves.
    private func refreshHover() {
        guard isActive, let window, !isHiddenOrHasHiddenAncestor else {
            isHovering = false
            return
        }
        let point = convert(window.mouseLocationOutsideOfEventStream, from: nil)
        isHovering = bounds.contains(point)
    }

    private func updateGrip(animated: Bool) {
        let height = showsGrip ? Grip.shownHeight : Grip.restHeight
        CATransaction.begin()
        if animated {
            CATransaction.setAnimationDuration(Grip.duration)
            CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeOut))
        } else {
            CATransaction.setDisableActions(true)
        }
        gripLayer.frame = CGRect(x: (bounds.width - Grip.width) / 2, y: (bounds.height - height) / 2, width: Grip.width, height: height)
        gripLayer.opacity = showsGrip ? Grip.alpha : 0
        glowLayer.frame = CGRect(
            x: bounds.minX + 0.5,
            y: Grip.verticalInset,
            width: max(0, bounds.width - 1),
            height: max(0, bounds.height - Grip.verticalInset)
        )
        glowLayer.opacity = showsGrip ? (isDragging ? Grip.dragGlow : Grip.hoverGlow) : 0
        CATransaction.commit()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// The strip's top runs through the title bar, where AppKit moves the window for any view
    /// that lets it; a press on the grip resizes the sidebar and nothing else.
    override var mouseDownCanMoveWindow: Bool { false }

    override func resetCursorRects() {
        super.resetCursorRects()
        guard isActive else { return }
        addCursorRect(bounds, cursor: .columnResize)
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        isActive ? super.hitTest(point) : nil
    }

    override func mouseDown(with event: NSEvent) {
        pressOriginX = event.locationInWindow.x
        isDragging = true
        updateGrip(animated: true)
        onBegin?()
    }

    override func mouseDragged(with event: NSEvent) {
        onChange?(event.locationInWindow.x - pressOriginX)
    }

    override func mouseUp(with event: NSEvent) {
        onChange?(event.locationInWindow.x - pressOriginX)
        isDragging = false
        onEnd?()
        // No exit arrives while the button is held; the release decides from the cursor.
        refreshHover()
        updateGrip(animated: true)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let trackingArea { removeTrackingArea(trackingArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        isHovering = isActive
        updateGrip(animated: true)
    }

    override func mouseExited(with event: NSEvent) {
        isHovering = false
        updateGrip(animated: true)
    }
}
