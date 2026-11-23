// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The chat's Crew mark, first in the chrome row: an anchor. With Crew on in Settings it
/// shows in every chat, and is active wherever the chat's own switch (a right-tap away) is
/// on too: charged, a ring of the theme's colours turning around it and a soft glow
/// breathing behind, so the chat reads as a team at work. With that switch off it sits
/// quiet, and says so. The badge above it counts the hands at work; tapping the badge (or
/// the mark) opens the deck in the right sidebar, tapping again closes it.
struct CrewButton: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    let thread: ChatThread
    let runtime: ThreadRuntime

    @State private var isHovering = false
    /// With nothing out yet, the button says where Crew stands rather than doing nothing.
    @State private var isExplaining = false
    /// Briefly true after hands go out, so the badge first shows the sending pulse
    /// inside its own box before morphing into the working count.
    @State private var isSending = false
    /// Right-tapping the mark opens the per-chat Crew switch in place.
    @State private var isCrewSwitchShown = false

    var body: some View {
        let isOn = model.crewIsOn(thread)
        // This chat's own hands, read through its child cells: another lead's hand
        // starting or finishing never re-renders this button.
        let hands = model.panelHands(of: thread.id)
        let running = hands.count { $0.crew?.status == .running }
        // The lead writing hand briefs, before any hand status is running: a native
        // agent tool at work, or a delegation block in the reply. Derived from the
        // lead's own timeline, so the badge pulses as soon as delegation starts.
        let isDelegating = isOn && running == 0 && runtime.isRunning && Self.isLeadDelegating(runtime.entries)
        Button {
            if hands.isEmpty { isExplaining = true } else { togglePanel() }
        } label: {
            ZStack {
                if isOn {
                    // The theme is read here, so switching it recolours the charge at once.
                    CrewCharge(isWorking: running > 0, theme: model.settings.theme)
                        .transition(.scale(scale: 0.6).combined(with: .opacity))
                }
                CrewMarkView(isOn: isOn, isHovering: isHovering)
            }
            .frame(width: Chrome.capsuleHeight, height: Chrome.capsuleHeight)
            .contentShape(Circle())
            // The badge is part of the glass's own content, which is the one place that
            // draws above the glass: the chrome row's container composites every glass in
            // it over everything else in it, so a badge laid over the button after the
            // glass, as an overlay or a later sibling, came out underneath.
            .overlay(alignment: .topTrailing) {
                if running > 0 || isSending || isDelegating {
                    CrewSendWorkingBadge(running: running, isSending: isSending || isDelegating)
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
            CrewRightTap { isCrewSwitchShown.toggle() }
        }
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .popover(isPresented: $isExplaining, arrowEdge: .bottom) {
            CrewIdlePopover(thread: thread, isOn: isOn)
        }
        .popover(isPresented: $isCrewSwitchShown, arrowEdge: .bottom) {
            CrewSwitchPopover(thread: thread, isOn: isOn, isCrewSwitchShown: $isCrewSwitchShown)
        }
        .help(help(isOn: isOn, running: running, hasHands: !hands.isEmpty))
        .accessibilityLabel(Text(panelHelp(isOn: isOn, running: running, hasHands: !hands.isEmpty)))
        .accessibilityValue(Text(isOn ? "On" : "Off"))
    }

    /// Opens the right sidebar on the deck; already there, closes the column. The chat
    /// slides to make room or take it back, and hands keep working either way. With
    /// nothing out yet the popover explains instead (see the body).
    private func togglePanel() {
        runtime.showsCrewDeck.toggle()
    }

    /// Whether the lead's own timeline shows delegation under way: a native agent tool
    /// still running, or a delegation block in a reply (streaming briefs, or written
    /// while the hands have yet to go out; the block leaves the reply once they do).
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
                    ? hasDelegationBlockWhileStreaming(in: message.text, rowID: entry.id)
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
        let found = CrewPrompts.hasDelegationBlock(in: text)
        delegationChecks.insert(found, for: rowID)
        return found
    }

    /// How far each streaming reply has been scanned for its opening fence, by row. The
    /// reply grows by appending, so each render scans only what arrived since the last
    /// one, with a few bytes of overlap for a fence split across two flushes; running the
    /// fence regex over a reply of tens of kilobytes on every render was a whole reply's
    /// worth of text on the main thread per token. A row found to hold a block stays found.
    @MainActor private static var streamingScans: [String: (scanned: Int, found: Bool)] = [:]

    @MainActor
    private static func hasDelegationBlockWhileStreaming(in text: String, rowID: String) -> Bool {
        if let known = streamingScans[rowID], known.found { return true }
        let scanned = streamingScans[rowID]?.scanned ?? 0
        let count = text.utf8.count
        if count == scanned { return false }
        // A reply rewritten shorter is read from the start again.
        let from = count < scanned ? 0 : max(0, scanned - 32)
        let start = text.utf8.index(text.utf8.startIndex, offsetBy: from)
        let found = CrewPrompts.hasDelegationOpener(in: text[start...])
        if streamingScans.count > 64 { streamingScans.removeAll(keepingCapacity: true) }
        streamingScans[rowID] = (count, found)
        return found
    }

    /// Whether Crew is on here, as the first words of everything the mark says. Off is this
    /// chat's own switch: the mark only shows with Crew on in Settings (see `ChatChromeRow`),
    /// and the master is read all the same for the moment the mark takes to fade out once
    /// it goes off there. The words used to say "on" whatever the switch said.
    private func crewState(isOn: Bool) -> String {
        if isOn { return "Crew is on" }
        return model.settings.crewEnabled ? "Crew is off for this chat" : "Crew is off in Settings"
    }

    private func panelHelp(isOn: Bool, running: Int, hasHands: Bool) -> String {
        let state = crewState(isOn: isOn)
        guard hasHands else { return isOn ? "\(state), no hands out yet" : state }
        if running == 0 { return "\(state), show the panel" }
        return running == 1 ? "\(state), 1 hand working, show or hide the panel" : "\(state), \(running) hands working, show or hide the panel"
    }

    private func help(isOn: Bool, running: Int, hasHands: Bool) -> String {
        var parts = [crewState(isOn: isOn)]
        // What the hands run on is a lead's to say; with Crew off here no hand goes out.
        if isOn {
            if let pair = model.crewPair(for: thread) {
                parts.append(CrewPairSummary.workers(pair, registry: model.providers))
            } else {
                parts.append("Hands run on this chat's model")
            }
        }
        if running > 0 { parts.append(running == 1 ? "1 hand working" : "\(running) hands working") }
        guard hasHands else {
            // The switch is on a right-click (`CrewSwitchPopover`), which offers the master
            // too when that is what is off.
            parts.append(isOn ? "No hands out yet" : "Right-click to turn it on")
            return parts.joined(separator: " · ")
        }
        parts.append(runtime.showsCrewDeck ? "Hide the deck" : "Show the deck")
        return parts.joined(separator: " · ")
    }
}

/// What a right-tap on the Crew mark offers, in place: the per-chat switch, on or
/// off for this chat only. Left-click keeps opening the panel; this only flips the chat.
private struct CrewSwitchPopover: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread
    let isOn: Bool
    @Binding var isCrewSwitchShown: Bool

    var body: some View {
        PopoverMenu {
            if !model.settings.crewEnabled {
                PopoverSectionHeader("Crew is off in Settings")
                PopoverItem("Turn Crew on") {
                    model.settings.crewEnabled = true
                    model.setCrew(true, for: thread.id)
                    isCrewSwitchShown = false
                }
            } else {
                PopoverSectionHeader(isOn ? "Crew is on for this chat" : "Crew is off for this chat")
                PopoverItem("Turn Crew on for this chat", isChecked: isOn) {
                    model.setCrew(true, for: thread.id)
                    isCrewSwitchShown = false
                }
                PopoverItem("Turn Crew off for this chat", isChecked: !isOn) {
                    model.setCrew(false, for: thread.id)
                    isCrewSwitchShown = false
                }
            }
        }
    }
}

/// Catches a right-tap, a right-click or a control-click, on the Crew mark. A
/// background with no drawing of its own, sized to the button, so the button's
/// left-click, badge, hover and help text are exactly as they were. It watches the
/// window's events rather than hanging a recognizer on itself: the SwiftUI button in
/// front of it claims every hit over the mark, so a recognizer on the background never
/// saw a click. A secondary click that lands inside this view's frame fires the action
/// and goes no further; every other event passes untouched. Control-click counts as a
/// secondary click too.
private struct CrewRightTap: NSViewRepresentable {
    var action: () -> Void

    func makeNSView(context: Context) -> CrewRightTapHost {
        CrewRightTapHost(action: action)
    }

    func updateNSView(_ nsView: CrewRightTapHost, context: Context) {
        nsView.action = action
    }
}

private final class CrewRightTapHost: NSView {
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

/// What the Crew mark says while nothing is out, as the switch stands. On: that it is,
/// which pair leads, and what sends hands out. Off: that it is, for this chat or (for the
/// moment the mark takes to fade out) in Settings, and how to turn it back on. The button
/// has no panel to show yet, and a mark that swallowed the click said none of this; the
/// popover used to say Crew was on with the chat's own switch off.
private struct CrewIdlePopover: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread
    let isOn: Bool

    var body: some View {
        PopoverMenu {
            if isOn {
                PopoverSectionHeader("Crew is on")
                if let pair = model.crewPair(for: thread) {
                    PopoverNote(CrewPairSummary.title(pair, registry: model.providers))
                    PopoverNote(CrewPairSummary.workers(pair, registry: model.providers))
                } else {
                    PopoverNote(ownModelNote)
                }
                PopoverDivider()
                PopoverNote("No hands are out yet. Ask for something big enough to split up and this chat sends them out, each with a panel of its own.")
                PopoverDivider()
                PopoverNote("Right-click the mark to switch Crew off for this chat.")
            } else if model.settings.crewEnabled {
                PopoverSectionHeader("Crew is off for this chat")
                PopoverNote("It leads no team: no hands go out from it until Crew is back on here.")
                PopoverDivider()
                PopoverNote("Right-click the mark to switch Crew on for this chat.")
            } else {
                PopoverSectionHeader("Crew is off in Settings")
                PopoverNote("No chat leads a team while it is off.")
                PopoverDivider()
                PopoverNote("Right-click the mark to turn Crew on.")
            }
        }
        .frame(width: 290)
    }

    /// What hands run on with no pair: the chat's own model, and the effort the launch
    /// resolves for them. That is the chat's own unless "Hands work at a working effort"
    /// brings a lead thinking above the working rung down to it (`AppModel.crewLaunch`),
    /// which the note used to leave out.
    private var ownModelNote: String {
        guard let effort = model.crewLaunch(for: thread)?.workerEffort, effort != thread.effort else {
            return "Hands run on this chat's model and effort."
        }
        return "Hands run on this chat's model, at \(ModelOption.effortTitle(effort).lowercased()) effort."
    }
}

/// The anchor: quiet when Crew is off, full when it is on or under the pointer.
private struct CrewMarkView: View {
    let isOn: Bool
    let isHovering: Bool

    var body: some View {
        CrewMarkImage()
            .foregroundStyle(Chrome.primaryText.opacity(isOn || isHovering ? 1 : 0.6))
            .frame(width: 18, height: 18)
    }
}

/// The charge around an active Crew button: a ring of the theme's colours and a glow of
/// its accent behind the mark. While a hand works the ring keeps turning and the glow
/// breathes; both are animations the render server runs on its own (a rotation and an
/// opacity, repeating), so a working team costs the main thread nothing. With no hand out
/// the ring stands still: the button sits inside a glass surface, and glass redraws with
/// every frame its content moves, so a ring that turned all day kept the compositor
/// busy for every chat with Crew on. Reduced motion keeps the still ring too.
struct CrewCharge: View {
    /// Whether any hand is at work: the ring turns and the glow breathes only then.
    var isWorking = true
    /// The theme the charge is drawn in. Its colours are read through `Chrome`; holding
    /// the theme makes a switch of it redraw the charge, still or moving.
    let theme: AppTheme

    /// The ring: the theme's accent and status hues, then the accent and the success hue
    /// again, fainter, so it turns through the palette the rest of the window wears. The
    /// roster's colours it once used stayed the same under every theme.
    fileprivate static var colors: [Color] {
        [Chrome.accent, Chrome.success, Chrome.warning, Chrome.danger, Chrome.accent.opacity(0.6), Chrome.success.opacity(0.6)]
    }
    /// The same colours for the moving ring, which is drawn by a layer: the accent as
    /// AppKit has it, so the system theme's stays the Mac's dynamic control accent. Made
    /// once per palette and handed back as the same objects after that: the button redraws
    /// with every token a lead streams, and fresh colours each time would read as a change
    /// and recolour the layers on every one of them (see `CrewChargeLayerView.configure`).
    @MainActor fileprivate static var layerColors: [NSColor] {
        let palette = [Chrome.accent, Chrome.success, Chrome.warning, Chrome.danger]
        if let cached = layerColorCache, cached.palette == palette { return cached.colors }
        let accent = Chrome.accentNSColor
        let success = NSColor(Chrome.success)
        let colors = [accent, success, NSColor(Chrome.warning), NSColor(Chrome.danger), accent.withAlphaComponent(0.6), success.withAlphaComponent(0.6)]
        layerColorCache = (palette, colors)
        return colors
    }
    /// The palette the layer colours were made from, and the colours themselves.
    @MainActor private static var layerColorCache: (palette: [Color], colors: [NSColor])? = nil

    var body: some View {
        ZStack {
            if isWorking, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
                CrewChargeMotion(theme: theme)
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

/// The charge in motion. A view of its own, so the repeating animations start when a hand
/// goes out and are torn down with it when the last one is back. Drawn by Core Animation:
/// the ring's turn and the glow's breath are animations on layers of their own, run by the
/// render server, so no frame of them touches the main thread or re-composites the glass
/// row around the button. As SwiftUI animations they re-rendered the button every frame,
/// and the button sits in the chrome row's glass container, which composited the whole
/// row again each time, for as long as any hand worked.
private struct CrewChargeMotion: NSViewRepresentable {
    /// The theme the layers are coloured for: a new one reaches `updateNSView`, which
    /// recolours them (the layer view also hears `ThemeManager.didChange` on its own).
    let theme: AppTheme

    func makeNSView(context: Context) -> CrewChargeLayerView {
        let view = CrewChargeLayerView()
        view.configure(colors: CrewCharge.layerColors, accent: Chrome.accentNSColor)
        return view
    }

    func updateNSView(_ view: CrewChargeLayerView, context: Context) {
        view.configure(colors: CrewCharge.layerColors, accent: Chrome.accentNSColor)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: CrewChargeLayerView, context: Context) -> CGSize? {
        // Fills whatever the button proposes, as the circles it replaces did.
        proposal.replacingUnspecifiedDimensions()
    }
}

/// The moving charge as layers: the glow, a radial gradient breathing between the still
/// glow's opacity and full, and the ring, a conic gradient turning once every four seconds
/// under a stroked-circle mask and brightening in step with the glow. The geometry is the
/// still charge's exactly (`CrewCharge.glow` and `.ring`), so the swap between the two
/// states moves nothing. Every animation runs from one fixed origin on the shared clock,
/// so putting it back on wherever the render server may have dropped it (the app coming
/// to the front, the window uncovered, the view joining a window late) changes nothing
/// on screen: the ring keeps its angle and the glow its brightness.
final class CrewChargeLayerView: NSView {
    private let glow = CAGradientLayer()
    private let ring = CAGradientLayer()
    private let ringMask = CAShapeLayer()
    private var colors: [NSColor] = []
    private var accent: NSColor = .controlAccentColor

    private static let turnKey = "turn"
    private static let breathKey = "breathe"
    /// The still ring's and glow's opacity, from `CrewCharge`; each breath rises to full.
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
        // The accent and the ring's hues are set on the layers once, not resolved on every
        // draw, so a theme change has to reach them here.
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
        colors = CrewCharge.layerColors
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
    /// a dark variant), into the glow's stops; the ring's colours, resolved the same way
    /// (the system theme's status hues are dynamic too), into the conic gradient. Those go
    /// in reversed: the gradient's angle grows counter-clockwise on a y-up layer, where the
    /// still ring's angular gradient runs clockwise.
    private func applyColors() {
        guard let first = colors.first else { return }
        let stops = (colors + [first]).reversed()
        var accent = self.accent.cgColor
        var ringColors = stops.map(\.cgColor)
        effectiveAppearance.performAsCurrentDrawingAppearance {
            accent = self.accent.cgColor
            ringColors = stops.map(\.cgColor)
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        glow.colors = [accent.copy(alpha: 0.46) ?? accent, accent.copy(alpha: 0) ?? accent]
        ring.colors = ringColors
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

/// The count of hands at work, above the button's top-right: tapping it is tapping the
/// button. While the lead delegates it first shows the sending pulse, then morphs in
/// place into the working count: the same anchor, the same capsule at the same size, the
/// existing mini spinner and the panel slide for motion, so nothing jumps and no second
/// row ever appears.
private struct CrewSendWorkingBadge: View {
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
        .help(running == 1 ? "1 hand working · Show or hide the Crew panel" : "\(running) hands working · Show or hide the Crew panel")
        .accessibilityHidden(true)
    }
}

/// One-line summaries of a pair, for tooltips and rows.
enum CrewPairSummary {
    /// The hands' model by name: the pair's own, or, on another provider than the lead's
    /// with none chosen, that provider's default, which is what the hands run there.
    @MainActor
    static func workerModel(_ pair: CrewPair, registry: ProviderRegistry) -> String? {
        if let model = pair.workerModel { return registry.model(model, for: pair.handsProvider)?.shortName ?? model }
        guard pair.sendsHandsElsewhere else { return nil }
        return registry.defaultModel(for: pair.handsProvider)?.shortName
    }

    /// "Hands on Opus · High effort", "Hands on Gemini 3.8 Flash (Antigravity)", or what
    /// they inherit.
    @MainActor
    static func workers(_ pair: CrewPair, registry: ProviderRegistry) -> String {
        var parts: [String] = []
        if let model = workerModel(pair, registry: registry) {
            parts.append("Hands on " + model + (pair.sendsHandsElsewhere ? " (\(pair.handsProvider.displayName))" : ""))
        } else if pair.sendsHandsElsewhere {
            parts.append("Hands on \(pair.handsProvider.displayName)")
        } else {
            parts.append("Hands on the chat's model")
        }
        if let effort = pair.workerEffort { parts.append(ModelOption.effortTitle(effort) + " effort") }
        return parts.joined(separator: " · ")
    }

    /// "Claude lead, Antigravity hands" across providers, or just "Claude".
    static func providers(_ pair: CrewPair) -> String {
        pair.sendsHandsElsewhere ? "\(pair.provider.displayName) lead, \(pair.handsProvider.displayName) hands" : pair.provider.displayName
    }

    /// The efforts in one phrase: "High effort" when lead and hands share one, "Lead high,
    /// hands medium effort" when they differ, one side alone when only it is set, and
    /// nothing when neither is, so a row with defaults says nothing about effort.
    static func efforts(_ pair: CrewPair) -> String? {
        let lead = pair.orchestratorEffort.map { ModelOption.effortTitle($0).lowercased() }
        let hands = pair.workerEffort.map { ModelOption.effortTitle($0).lowercased() }
        switch (lead, hands) {
        case let (lead?, hands?) where lead == hands: return "\(lead.capitalized) effort"
        case let (lead?, hands?): return "Lead \(lead), hands \(hands) effort"
        case let (lead?, nil): return "Lead \(lead) effort"
        case let (nil, hands?): return "Hands \(hands) effort"
        case (nil, nil): return nil
        }
    }

    /// "Fable + Opus", "Any model + Sonnet", "Opus + itself", and across
    /// providers "Fable + Gemini 3.8 Flash".
    @MainActor
    static func modelsTitle(_ pair: CrewPair, registry: ProviderRegistry) -> String {
        let lead = pair.orchestratorModel.map { registry.model($0, for: pair.provider)?.shortName ?? $0 } ?? "Any \(pair.provider.displayName) model"
        guard let worker = workerModel(pair, registry: registry) ?? (pair.sendsHandsElsewhere ? pair.handsProvider.displayName : nil) else {
            return "\(lead) + itself"
        }
        return "\(lead) + \(worker)"
    }

    /// The pair by the name the user gave it, else by its models.
    @MainActor
    static func title(_ pair: CrewPair, registry: ProviderRegistry) -> String {
        pair.customName ?? modelsTitle(pair, registry: registry)
    }
}
