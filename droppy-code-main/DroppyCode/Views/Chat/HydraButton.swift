import AppKit
import SwiftUI

/// The chat's Hydra mark, first in the chrome row: one dragon head in profile. With Hydra on
/// in Settings it is always active in every chat: charged, a ring of the roster's colours
/// turning around it and a soft glow breathing behind, so the chat reads as a team at
/// work. The badge above it counts the heads at work; tapping the badge (or the mark)
/// opens the floating panel, tapping again dismisses it.
struct HydraButton: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    let thread: ChatThread
    let runtime: ThreadRuntime

    @State private var isHovering = false
    /// With Hydra on and nothing out yet, the button says so rather than doing nothing.
    @State private var isExplaining = false
    /// Briefly true after heads go out, so the badge first shows the sending pulse
    /// inside its own box before morphing into the working count.
    @State private var isSending = false
    /// Right-tapping the mark opens the per-chat Hydra switch in place.
    @State private var isHydraSwitchShown = false
    /// The first chat's intro, opened by the mark itself until it has been read.
    @State private var isIntroShown = false

    var body: some View {
        let isOn = model.hydraIsOn(thread)
        // This chat's own heads, read through its child cells: another lead's head
        // starting or finishing never re-renders this button.
        let heads = model.panelHeads(of: thread.id)
        let running = heads.count { $0.hydra?.status == .running }
        // The lead writing head briefs, before any head status is running: a native
        // agent tool at work, or a delegation block in the reply. Derived from the
        // lead's own timeline, so the badge pulses as soon as delegation starts.
        let isDelegating = isOn && running == 0 && runtime.isRunning && Self.isLeadDelegating(runtime.entries)
        Button {
            if !model.settings.hasSeenHydraIntro { isIntroShown = true } else if heads.isEmpty { isExplaining = true } else { togglePanel() }
        } label: {
            ZStack {
                if isOn {
                    HydraCharge(isWorking: running > 0)
                        .transition(.scale(scale: 0.6).combined(with: .opacity))
                }
                HydraMarkView(isOn: isOn, isHovering: isHovering)
            }
            .frame(width: Chrome.capsuleHeight, height: Chrome.capsuleHeight)
            .contentShape(Circle())
            // The badge is part of the glass's own content, which is the one place that
            // draws above the glass: the chrome row's container composites every glass in
            // it over everything else in it, so a badge laid over the button after the
            // glass, as an overlay or a later sibling, came out underneath.
            .overlay(alignment: .topTrailing) {
                if running > 0 || isSending || isDelegating {
                    HydraSendWorkingBadge(running: running, isSending: isSending || isDelegating)
                        .offset(x: 3, y: -3)
                        .transition(.scale.combined(with: .opacity))
                }
            }
            .animation(.spring(response: 0.3, dampingFraction: 0.7), value: running)
            .animation(Chrome.panelSlide, value: isSending)
            .animation(Chrome.panelSlide, value: isDelegating)
            // Switched on or off for the chat, the charge ring scales and fades rather than popping.
            .animation(Chrome.panelSlide, value: isOn)
            .onChange(of: running) { old, new in
                guard new > old else { return }
                isSending = true
            }
            .task(id: isSending) {
                guard isSending else { return }
                try? await Task.sleep(for: .seconds(0.9))
                guard !Task.isCancelled else { return }
                isSending = false
            }
        }
        .buttonStyle(.plain)
        .chromeGlassCircle()
        .background {
            HydraRightTap { isHydraSwitchShown.toggle() }
        }
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .popover(isPresented: $isExplaining, arrowEdge: .bottom) {
            HydraIdlePopover(thread: thread)
        }
        .popover(isPresented: $isHydraSwitchShown, arrowEdge: .bottom) {
            HydraSwitchPopover(thread: thread, isOn: isOn, isHydraSwitchShown: $isHydraSwitchShown)
        }
        .popover(isPresented: $isIntroShown, arrowEdge: .bottom) {
            HydraIntroPopover { model.settings.hasSeenHydraIntro = true; isIntroShown = false }
                .interactiveDismissDisabled()
        }
        .onAppear { if !model.settings.hasSeenHydraIntro, isOn { isIntroShown = true } }
        // The capture run hangs the intro's still from the mark, where the app shows it.
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
            if WebsiteCaptures.isEnabled { WebsiteCaptures.hydraMarkFrame = frame }
        }
        .help(help(isOn: isOn, running: running, hasHeads: !heads.isEmpty))
        .accessibilityLabel(Text(panelHelp(running: running, hasHeads: !heads.isEmpty)))
        .accessibilityValue(Text(isOn ? "On" : "Off"))
    }

    /// Shows the floating panel; shown, hides it. The panel fades and scales in and out
    /// in place while the chat slides to make room or take it back. Heads keep working
    /// either way. With nothing out yet the popover explains instead (see the body).
    private func togglePanel() {
        withAnimation(Chrome.panelSlide) { runtime.isHydraPanelHidden.toggle() }
    }

    /// Whether the lead's own timeline shows delegation under way: a native agent tool
    /// still running, or a delegation block in a reply (streaming briefs, or written
    /// while the heads have yet to go out; the block leaves the reply once they do).
    @MainActor
    private static func isLeadDelegating(_ entries: [TimelineEntry]) -> Bool {
        guard let lastEntry = entries.last else { return false }
        // The row's own turn, fixed for its lifetime: reading it through `item` would make
        // this button observe the last row of the turn before as well.
        let lastTurnID = lastEntry.turnID
        for entry in entries.reversed() {
            guard entry.turnID == lastTurnID else { break }
            switch entry.item.content {
            case .tool(let call) where call.kind == .agent && call.status == .running:
                return true
            case .assistant(let message):
                // The reply still streaming changed with this render, so it is read afresh;
                // the finished ones before it have not, and their answers are kept.
                let hasBlock = message.isStreaming
                    ? HydraPrompts.hasDelegationBlock(in: message.text)
                    : hasDelegationBlock(in: message.text, rowID: entry.id)
                if hasBlock { return true }
            default:
                break
            }
        }
        return false
    }

    /// Whether a finished reply holds a delegation block, remembered by its row. Every
    /// streamed token of the turn re-renders this button (the rows it reads are observed),
    /// and running the fence regex over every finished reply of the turn again for each
    /// token was a whole turn's worth of text on the main thread per token. The row's id
    /// is the key rather than its text: a finished reply's text no longer changes, and
    /// hashing a long reply per render would be a walk over it too.
    @MainActor private static var delegationChecks = RecentCache<String, Bool>(limit: 32)

    @MainActor
    private static func hasDelegationBlock(in text: String, rowID: String) -> Bool {
        if let known = delegationChecks.value(for: rowID) { return known }
        let found = HydraPrompts.hasDelegationBlock(in: text)
        delegationChecks.insert(found, for: rowID)
        return found
    }

    private func panelHelp(running: Int, hasHeads: Bool) -> String {
        guard hasHeads else { return "Hydra is on, no heads out yet" }
        if running == 0 { return "Hydra is on, show the panel" }
        return running == 1 ? "Hydra is on, 1 head working, show or hide the panel" : "Hydra is on, \(running) heads working, show or hide the panel"
    }

    private func help(isOn: Bool, running: Int, hasHeads: Bool) -> String {
        guard isOn else { return "Hydra is on in Settings" }
        var parts = ["Hydra is on"]
        if let pair = model.hydraPair(for: thread) {
            parts.append(HydraPairSummary.workers(pair, registry: model.providers))
        } else {
            parts.append("Heads run on this chat's model")
        }
        if running > 0 { parts.append(running == 1 ? "1 head working" : "\(running) heads working") }
        guard hasHeads else {
            parts.append("No heads out yet")
            return parts.joined(separator: " · ")
        }
        parts.append(runtime.isHydraPanelHidden ? "Show the panel" : "Hide the panel")
        return parts.joined(separator: " · ")
    }
}

/// What a right-tap on the Hydra mark offers, in place: the per-chat switch, on or
/// off for this chat only. Left-click keeps opening the panel; this only flips the chat.
private struct HydraSwitchPopover: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread
    let isOn: Bool
    @Binding var isHydraSwitchShown: Bool

    var body: some View {
        PopoverMenu {
            if !model.settings.hydraEnabled {
                PopoverSectionHeader("Hydra is off in Settings")
                PopoverItem("Turn Hydra on") {
                    model.settings.hydraEnabled = true
                    model.setHydra(true, for: thread.id)
                    isHydraSwitchShown = false
                }
            } else {
                PopoverSectionHeader(isOn ? "Hydra is on for this chat" : "Hydra is off for this chat")
                PopoverItem("Turn Hydra on for this chat", isChecked: isOn) {
                    model.setHydra(true, for: thread.id)
                    isHydraSwitchShown = false
                }
                PopoverItem("Turn Hydra off for this chat", isChecked: !isOn) {
                    model.setHydra(false, for: thread.id)
                    isHydraSwitchShown = false
                }
            }
        }
    }
}

/// Catches a right-tap, a right-click or a control-click, on the Hydra mark. A
/// background with no drawing of its own, sized to the button, so the button's
/// left-click, badge, hover and help text are exactly as they were. It watches the
/// window's events rather than hanging a recognizer on itself: the SwiftUI button in
/// front of it claims every hit over the mark, so a recognizer on the background never
/// saw a click. A secondary click that lands inside this view's frame fires the action
/// and goes no further; every other event passes untouched. Control-click counts as a
/// secondary click too.
private struct HydraRightTap: NSViewRepresentable {
    var action: () -> Void

    func makeNSView(context: Context) -> HydraRightTapHost {
        HydraRightTapHost(action: action)
    }

    func updateNSView(_ nsView: HydraRightTapHost, context: Context) {
        nsView.action = action
    }
}

private final class HydraRightTapHost: NSView {
    var action: () -> Void
    private var monitor: Any?

    init(action: @escaping () -> Void) {
        self.action = action
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    deinit { removeMonitor() }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { removeMonitor() } else { installMonitor() }
    }

    private func installMonitor() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.rightMouseDown, .leftMouseDown]) { [weak self] event in
            guard let self, self.catches(event) else { return event }
            self.action()
            return nil
        }
    }

    private func removeMonitor() {
        guard let monitor else { return }
        NSEvent.removeMonitor(monitor)
        self.monitor = nil
    }

    /// Whether the event is a secondary click on this view: in its window, over its
    /// frame, and either the right button or the left one with Control held.
    private func catches(_ event: NSEvent) -> Bool {
        guard let window, event.window === window, !isHiddenOrHasHiddenAncestor, !bounds.isEmpty else { return false }
        let isSecondary = event.type == .rightMouseDown || event.modifierFlags.contains(.control)
        guard isSecondary else { return false }
        return bounds.contains(convert(event.locationInWindow, from: nil))
    }
}

/// What the Hydra mark says while the team is on but nothing is out: that it is on, which
/// pair leads, and what sends heads out. The button has no panel to show yet, and a mark
/// that swallowed the click said none of this.
private struct HydraIdlePopover: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread

    var body: some View {
        PopoverMenu {
            PopoverSectionHeader("Hydra is on")
            if let pair = model.hydraPair(for: thread) {
                PopoverNote(HydraPairSummary.title(pair, registry: model.providers))
                PopoverNote(HydraPairSummary.workers(pair, registry: model.providers))
            } else {
                PopoverNote("Heads run on this chat's model and effort.")
            }
            PopoverDivider()
            PopoverNote("No heads are out yet. Ask for something big enough to split up and this chat sends them out, each with a panel of its own.")
            PopoverDivider()
            PopoverNote("Right-click the mark to switch Hydra off for this chat.")
        }
        .frame(width: 290)
    }
}

/// The dragon-head mark: quiet when Hydra is off, full when it is on or under the pointer.
private struct HydraMarkView: View {
    let isOn: Bool
    let isHovering: Bool

    var body: some View {
        HydraMarkImage()
            .foregroundStyle(Chrome.primaryText.opacity(isOn || isHovering ? 1 : 0.6))
            .frame(width: 18, height: 18)
    }
}

/// The charge around an active Hydra button: a ring of the roster's colours and a glow
/// behind the mark. While a head works the ring keeps turning and the glow breathes;
/// both are animations the render server runs on its own (a rotation and an opacity,
/// repeating), so a working team costs the main thread nothing. With no head out the
/// ring stands still: the button sits inside a glass surface, and glass redraws with
/// every frame its content moves, so a ring that turned all day kept the compositor
/// busy for every chat with Hydra on. Reduced motion keeps the still ring too.
struct HydraCharge: View {
    /// Whether any head is at work: the ring turns and the glow breathes only then.
    var isWorking = true

    fileprivate static let colors: [Color] = HydraRoster.personas.prefix(6).map(\.color)
    /// The same colours for the moving ring, which is drawn by a layer.
    fileprivate static let layerColors: [NSColor] = colors.map { NSColor($0) }

    var body: some View {
        ZStack {
            if isWorking, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                HydraChargeMotion()
            } else {
                Self.glow(breathing: false)
                Self.ring(turned: false)
            }
        }
        .allowsHitTesting(false)
    }

    static func glow(breathing: Bool) -> some View {
        Circle()
            .fill(
                RadialGradient(
                    colors: [Chrome.accent.opacity(0.46), Chrome.accent.opacity(0)],
                    center: .center,
                    startRadius: 2,
                    endRadius: 15
                )
            )
            .padding(2)
            .opacity(breathing ? 1 : 0.6)
    }

    static func ring(turned: Bool, bright: Bool = false) -> some View {
        Circle()
            .strokeBorder(
                AngularGradient(
                    gradient: Gradient(colors: colors + [colors[0]]),
                    center: .center
                ),
                lineWidth: 2
            )
            .padding(1)
            .rotationEffect(.degrees(turned ? 360 : 0))
            .opacity(bright ? 1 : 0.85)
    }
}

/// The charge in motion. A view of its own, so the repeating animations start when a head
/// goes out and are torn down with it when the last one is back. Drawn by Core Animation:
/// the ring's turn and the glow's breath are animations on layers of their own, run by the
/// render server, so no frame of them touches the main thread or re-composites the glass
/// row around the button. As SwiftUI animations they re-rendered the button every frame,
/// and the button sits in the chrome row's glass container, which composited the whole
/// row again each time, for as long as any head worked.
private struct HydraChargeMotion: NSViewRepresentable {
    func makeNSView(context: Context) -> HydraChargeLayerView {
        let view = HydraChargeLayerView()
        view.configure(colors: HydraCharge.layerColors, accent: Chrome.accentNSColor)
        return view
    }

    func updateNSView(_ view: HydraChargeLayerView, context: Context) {
        view.configure(colors: HydraCharge.layerColors, accent: Chrome.accentNSColor)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: HydraChargeLayerView, context: Context) -> CGSize? {
        // Fills whatever the button proposes, as the circles it replaces did.
        proposal.replacingUnspecifiedDimensions()
    }
}

/// The moving charge as layers: the glow, a radial gradient breathing between the still
/// glow's opacity and full, and the ring, a conic gradient turning once every four seconds
/// under a stroked-circle mask and brightening in step with the glow. The geometry is the
/// still charge's exactly (`HydraCharge.glow` and `.ring`), so the swap between the two
/// states moves nothing. Every animation runs from one fixed origin on the shared clock,
/// so putting it back on wherever the render server may have dropped it (the app coming
/// to the front, the window uncovered, the view joining a window late) changes nothing
/// on screen: the ring keeps its angle and the glow its brightness.
final class HydraChargeLayerView: NSView {
    private let glow = CAGradientLayer()
    private let ring = CAGradientLayer()
    private let ringMask = CAShapeLayer()
    private var colors: [NSColor] = []
    private var accent: NSColor = .controlAccentColor

    private static let turnKey = "turn"
    private static let breathKey = "breathe"
    /// The still ring's and glow's opacity, from `HydraCharge`; each breath rises to full.
    private static let ringRestOpacity: Float = 0.85
    private static let glowRestOpacity: Float = 0.6
    private static let turnDuration: CFTimeInterval = 4
    private static let breathDuration: CFTimeInterval = 1.4
    private static let lineWidth: CGFloat = 2
    /// The glow's gradient: full out to the start radius, gone at the end radius.
    private static let glowStartRadius: CGFloat = 2
    private static let glowEndRadius: CGFloat = 15

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        layer?.masksToBounds = false

        glow.type = .radial
        glow.startPoint = CGPoint(x: 0.5, y: 0.5)
        glow.locations = [NSNumber(value: Double(Self.glowStartRadius / Self.glowEndRadius)), 1]
        // A circle, as the still glow is: the gradient reaches past its edge.
        glow.masksToBounds = true
        glow.opacity = Self.glowRestOpacity

        ring.type = .conic
        ring.startPoint = CGPoint(x: 0.5, y: 0.5)
        ring.endPoint = CGPoint(x: 1, y: 0.5)
        ring.opacity = Self.ringRestOpacity
        ringMask.fillColor = nil
        ringMask.strokeColor = NSColor.black.cgColor
        ringMask.lineWidth = Self.lineWidth
        ring.mask = ringMask

        // Never implicitly animated: geometry and colours are set outright, and the
        // opacity and the turn belong to the explicit animations.
        let still: [String: CAAction] = [
            "opacity": NSNull(), "bounds": NSNull(), "position": NSNull(), "colors": NSNull(),
            "endPoint": NSNull(), "cornerRadius": NSNull(), "path": NSNull(), "transform": NSNull(),
        ]
        for sublayer in [glow, ring, ringMask] as [CALayer] {
            sublayer.actions = still
        }
        layer?.addSublayer(glow)
        layer?.addSublayer(ring)

        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(appDidBecomeActive), name: NSApplication.didBecomeActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(windowDidChangeOcclusionState(_:)), name: NSWindow.didChangeOcclusionStateNotification, object: nil)
        // The accent is set on the glow once, not resolved on every draw, so a theme
        // change has to reach it here.
        center.addObserver(self, selector: #selector(themeDidChange), name: ThemeManager.didChange, object: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func configure(colors: [NSColor], accent: NSColor) {
        let recolor = colors != self.colors || accent != self.accent
        self.colors = colors
        self.accent = accent
        if recolor { applyColors() }
        // The animations go on once; the lifecycle below puts them back wherever they can
        // be lost. A re-render of the button (every hover) is not one of those.
        if ring.animation(forKey: Self.turnKey) == nil { installAnimations() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Joining a window late (a button built off screen), the layers need their colours
        // resolved for its appearance and their animations running from the shared clock.
        guard window != nil else { return }
        applyColors()
        installAnimations()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        // The system accent is dynamic; re-resolve it for the new appearance.
        applyColors()
    }

    @objc private func appDidBecomeActive() {
        guard window != nil else { return }
        installAnimations()
    }

    @objc private func windowDidChangeOcclusionState(_ note: Notification) {
        // Every window ordered in posts this, popovers and tooltips included: only this
        // view's own window coming back into view concerns it.
        guard let window, (note.object as? NSWindow) === window, window.occlusionState.contains(.visible) else { return }
        installAnimations()
    }

    @objc private func themeDidChange() {
        accent = Chrome.accentNSColor
        applyColors()
    }

    override func layout() {
        super.layout()
        let bounds = self.bounds
        guard bounds.width > 4, bounds.height > 4 else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        // The still glow: a circle two points in, its gradient running from two to fifteen
        // points out of the centre whatever the circle's size.
        let glowRect = bounds.insetBy(dx: 2, dy: 2)
        glow.bounds = CGRect(origin: .zero, size: glowRect.size)
        glow.position = CGPoint(x: glowRect.midX, y: glowRect.midY)
        glow.cornerRadius = min(glowRect.width, glowRect.height) / 2
        glow.endPoint = CGPoint(
            x: 0.5 + Self.glowEndRadius / glowRect.width,
            y: 0.5 + Self.glowEndRadius / glowRect.height
        )
        // The still ring: two points of stroke just inside a circle one point in. The
        // mask turns with the ring, and a circle looks the same at every angle.
        let ringRect = bounds.insetBy(dx: 1, dy: 1)
        ring.bounds = CGRect(origin: .zero, size: ringRect.size)
        ring.position = CGPoint(x: ringRect.midX, y: ringRect.midY)
        ringMask.frame = ring.bounds
        ringMask.path = CGPath(
            ellipseIn: ring.bounds.insetBy(dx: Self.lineWidth / 2, dy: Self.lineWidth / 2),
            transform: nil
        )
        CATransaction.commit()
    }

    /// The accent, resolved in this view's appearance (the system accent has a light and
    /// a dark variant), into the glow's stops; the ring's colours into the conic gradient.
    /// Those go in reversed: the gradient's angle grows counter-clockwise on a y-up layer,
    /// where the still ring's angular gradient runs clockwise.
    private func applyColors() {
        guard let first = colors.first else { return }
        var accent = self.accent.cgColor
        effectiveAppearance.performAsCurrentDrawingAppearance {
            accent = self.accent.cgColor
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        glow.colors = [accent.copy(alpha: 0.46) ?? accent, accent.copy(alpha: 0) ?? accent]
        ring.colors = (colors + [first]).reversed().map(\.cgColor)
        CATransaction.commit()
    }

    /// Puts the turn and the two breaths on afresh, each from one second on the shared
    /// clock, so however often they are put back the phase is the one the clock says.
    private func installAnimations() {
        ring.removeAllAnimations()
        glow.removeAllAnimations()
        let origin = ring.convertTime(1, from: nil)

        let turn = CABasicAnimation(keyPath: "transform.rotation.z")
        turn.fromValue = 0
        // The layer is y-up, so a negative angle turns clockwise on screen, the way the
        // still ring's `rotationEffect(.degrees(360))` did.
        turn.toValue = -2 * Double.pi
        turn.duration = Self.turnDuration
        turn.repeatCount = .infinity
        turn.timingFunction = CAMediaTimingFunction(name: .linear)
        turn.beginTime = origin
        turn.isRemovedOnCompletion = false
        ring.add(turn, forKey: Self.turnKey)

        for (layer, rest) in [(ring, Self.ringRestOpacity), (glow, Self.glowRestOpacity)] {
            let breath = CABasicAnimation(keyPath: "opacity")
            breath.fromValue = rest
            breath.toValue = 1
            breath.duration = Self.breathDuration
            breath.autoreverses = true
            breath.repeatCount = .infinity
            breath.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            breath.beginTime = origin
            breath.isRemovedOnCompletion = false
            layer.add(breath, forKey: Self.breathKey)
        }
    }
}

/// The count of heads at work, above the button's top-right: tapping it is tapping the
/// button. While the lead delegates it first shows the sending pulse, then morphs in
/// place into the working count: the same anchor, the same capsule at the same size, the
/// existing mini spinner and the panel slide for motion, so nothing jumps and no second
/// row ever appears.
private struct HydraSendWorkingBadge: View {
    let running: Int
    let isSending: Bool

    var body: some View {
        Group {
            if isSending {
                MiniSpinner(cellSize: 2.4)
            } else {
                Text(verbatim: "\(running)")
                    .font(.system(size: 9, weight: .bold).monospacedDigit())
                    .foregroundStyle(.white)
                    .contentTransition(.opacity)
            }
        }
        .frame(minWidth: 14, minHeight: 14)
        .padding(.horizontal, 4)
        .padding(.vertical, 1)
        // The pulse reads on glass, the count on accent: the same capsule either
        // way, so the send-to-working change is a tint crossfade, never a jump.
        .background(isSending ? Chrome.glassTint : Chrome.accent, in: Capsule())
        .contentShape(Capsule())
        .animation(Chrome.panelSlide, value: isSending)
        .help(running == 1 ? "1 head working · Show or hide the Hydra panel" : "\(running) heads working · Show or hide the Hydra panel")
        .accessibilityHidden(true)
    }
}

/// One-line summaries of a pair, for tooltips and rows.
enum HydraPairSummary {
    /// The heads' model by name: the pair's own, or, on another provider than the lead's
    /// with none chosen, that provider's default, which is what the heads run there.
    @MainActor
    static func workerModel(_ pair: HydraPair, registry: ProviderRegistry) -> String? {
        if let model = pair.workerModel { return registry.model(model, for: pair.headsProvider)?.shortName ?? model }
        guard pair.sendsHeadsElsewhere else { return nil }
        return registry.defaultModel(for: pair.headsProvider)?.shortName
    }

    /// "Heads on Opus · High effort", "Heads on Gemini 3.8 Flash (Antigravity)", or what
    /// they inherit.
    @MainActor
    static func workers(_ pair: HydraPair, registry: ProviderRegistry) -> String {
        var parts: [String] = []
        if let model = workerModel(pair, registry: registry) {
            parts.append("Heads on " + model + (pair.sendsHeadsElsewhere ? " (\(pair.headsProvider.displayName))" : ""))
        } else if pair.sendsHeadsElsewhere {
            parts.append("Heads on \(pair.headsProvider.displayName)")
        } else {
            parts.append("Heads on the chat's model")
        }
        if let effort = pair.workerEffort { parts.append(ModelOption.effortTitle(effort) + " effort") }
        return parts.joined(separator: " · ")
    }

    /// "Claude lead, Antigravity heads" across providers, or just "Claude".
    static func providers(_ pair: HydraPair) -> String {
        pair.sendsHeadsElsewhere ? "\(pair.provider.displayName) lead, \(pair.headsProvider.displayName) heads" : pair.provider.displayName
    }

    /// The efforts in one phrase: "High effort" when lead and heads share one, "Lead high,
    /// heads medium effort" when they differ, one side alone when only it is set, and
    /// nothing when neither is, so a row with defaults says nothing about effort.
    static func efforts(_ pair: HydraPair) -> String? {
        let lead = pair.orchestratorEffort.map { ModelOption.effortTitle($0).lowercased() }
        let heads = pair.workerEffort.map { ModelOption.effortTitle($0).lowercased() }
        switch (lead, heads) {
        case let (lead?, heads?) where lead == heads: return "\(lead.capitalized) effort"
        case let (lead?, heads?): return "Lead \(lead), heads \(heads) effort"
        case let (lead?, nil): return "Lead \(lead) effort"
        case let (nil, heads?): return "Heads \(heads) effort"
        case (nil, nil): return nil
        }
    }

    /// "Fable + Opus", "Any model + Sonnet", "Opus + itself", and across
    /// providers "Fable + Gemini 3.8 Flash".
    @MainActor
    static func title(_ pair: HydraPair, registry: ProviderRegistry) -> String {
        let lead = pair.orchestratorModel.map { registry.model($0, for: pair.provider)?.shortName ?? $0 } ?? "Any \(pair.provider.displayName) model"
        guard let worker = workerModel(pair, registry: registry) ?? (pair.sendsHeadsElsewhere ? pair.headsProvider.displayName : nil) else {
            return "\(lead) + itself"
        }
        return "\(lead) + \(worker)"
    }
}
