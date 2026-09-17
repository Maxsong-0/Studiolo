import AppKit
import SwiftUI

/// Every floating panel of a chat, for its place in a corner's stack.
enum FloatingPanelID: Hashable {
    case subagent, hydra, popped, usage, auto(UUID)
}

/// The side of the chat column a docked panel sits on.
enum PanelDockSide: Equatable {
    case leading, trailing
}

/// The corner a floating panel docks in.
enum PanelDockCorner: String, Equatable {
    case topLeading, topTrailing, bottomLeading, bottomTrailing

    var side: PanelDockSide {
        switch self {
        case .topLeading, .bottomLeading: .leading
        case .topTrailing, .bottomTrailing: .trailing
        }
    }

    var isTop: Bool { self == .topLeading || self == .topTrailing }

    static func make(side: PanelDockSide, isTop: Bool) -> PanelDockCorner {
        switch (side, isTop) {
        case (.leading, true): .topLeading
        case (.trailing, true): .topTrailing
        case (.leading, false): .bottomLeading
        case (.trailing, false): .bottomTrailing
        }
    }

    /// The corner on the other side of the chat column, at the same edge.
    var acrossTheColumn: PanelDockCorner {
        switch self {
        case .topLeading: .topTrailing
        case .topTrailing: .topLeading
        case .bottomLeading: .bottomTrailing
        case .bottomTrailing: .bottomLeading
        }
    }
}

/// The room the docked panels take from the chat column on each side; the timeline and
/// the chat box centre in what is left, so they stay lined up with each other.
struct PanelReserve: Equatable {
    var leading: CGFloat = 0
    var trailing: CGFloat = 0
}

/// The four docked spots for the pane as it is.
struct PanelDocks {
    var topLeading: CGPoint
    var topTrailing: CGPoint
    var bottomLeading: CGPoint
    var bottomTrailing: CGPoint

    init(layout: SubagentPanelLayout, reserve: PanelReserve) {
        topLeading = layout.dockedOrigin(.topLeading, reserve: reserve)
        topTrailing = layout.dockedOrigin(.topTrailing, reserve: reserve)
        bottomLeading = layout.dockedOrigin(.bottomLeading, reserve: reserve)
        bottomTrailing = layout.dockedOrigin(.bottomTrailing, reserve: reserve)
    }

    subscript(corner: PanelDockCorner) -> CGPoint {
        switch corner {
        case .topLeading: topLeading
        case .topTrailing: topTrailing
        case .bottomLeading: bottomLeading
        case .bottomTrailing: bottomTrailing
        }
    }

    /// A panel's spot in its side's grid of `slots` slots, spread evenly between
    /// the two docked spots: a panel in a top corner with `below = b` sits in
    /// slot `b`, one in a bottom corner in slot `slots - 1 - b`.
    func stacked(_ corner: PanelDockCorner, below: Int, slots: Int, layout: SubagentPanelLayout) -> CGPoint {
        let step = slots > 1 ? (bottomLeading.y - topLeading.y) / CGFloat(slots - 1) : 0
        let index = corner.isTop ? below : slots - 1 - below
        let y = topLeading.y + CGFloat(max(0, index)) * step
        return CGPoint(x: self[corner].x, y: y)
    }

    /// The corner and stack depth a side's slot holds: the top half sits in the
    /// side's top corner counting down from the chrome row, the bottom half in
    /// the bottom corner counting up from the chat box.
    static func corner(forSlot index: Int, of slots: Int, side: PanelDockSide) -> (corner: PanelDockCorner, below: Int) {
        if index <= (slots - 1) / 2 {
            (PanelDockCorner.make(side: side, isTop: true), index)
        } else {
            (PanelDockCorner.make(side: side, isTop: false), slots - 1 - index)
        }
    }
}

/// Where a floating panel goes and how big it is, from the chat pane's size. Docked, it
/// sits in a corner beside the chat column, the group centred in the pane; when the pane
/// is too narrow for both side by side it sits in the corner above the box instead, so
/// neither is ever covered. Dragged elsewhere, it stays within the pane.
struct SubagentPanelLayout: Equatable {
    /// The panel's width, given the room: the composer's own column width at most.
    static let width: CGFloat = 400
    static let minWidth: CGFloat = 300
    static let minHeight: CGFloat = 220
    /// Between the panel and the chat box.
    static let gap: CGFloat = 12
    /// The chat box's own margins (see ComposerArea), so the docked panel lines up with it.
    static let sideMargin: CGFloat = 20
    static let bottomMargin: CGFloat = 14
    /// The narrowest chat box worth typing in; below it the panel moves above the box.
    static let minComposerWidth: CGFloat = 360
    static let composerMaxWidth: CGFloat = 820
    /// How far past the pane's middle a held panel's centre goes before it changes corner,
    /// so one held on the line does not flicker between the two.
    static let cornerSlack: CGFloat = 56

    var pane: CGSize
    /// The chat box with its tabs and cards, so a panel above it clears them.
    var composerAreaHeight: CGFloat
    /// The most panels docked in any one corner. Stacked, they share the room between the
    /// chrome row and the chat box, so two panels sit one above the other without
    /// overlapping wherever the pane is tall enough for two of the smallest.
    var stackDepth: Int = 1
    /// The size the user dragged a panel to, if any (see `AppSettings.panelSize`). Fitted
    /// to the pane below, so a panel keeps the size it was given while there is room and
    /// gives way as the window shrinks, then takes it back.
    var preferred: CGSize? = nil
    /// The panels show a head's task and progress bar in place of its conversation (see
    /// `HydraHeadProgress`), which needs far less height: `compactHeight`, whatever
    /// height was dragged, though the dragged width still counts.
    var compact = false
    /// Room for the strip, a two-line task over the bar, and the chat box under them.
    static let compactHeight: CGFloat = 280

    var panelWidth: CGFloat {
        min(max(preferred?.width ?? Self.width, Self.minWidth), max(Self.minWidth, pane.width - 2 * Self.sideMargin))
    }

    /// Whether the panel and the chat box fit side by side.
    var sitsBesideComposer: Bool {
        pane.width - 2 * Self.sideMargin - panelWidth - Self.gap >= Self.minComposerWidth
    }

    /// Whether a panel on each side of the chat box still leaves a box worth typing in.
    /// Below this, panels dock on one side only (see `PanelScene.lockedSide`).
    var fitsBothSides: Bool {
        pane.width - 2 * Self.sideMargin - 2 * (panelWidth + Self.gap) >= Self.minComposerWidth
    }

    /// The room the docked panel takes from the chat box's row; the box centres in the rest.
    var composerReserve: CGFloat {
        sitsBesideComposer ? panelWidth + Self.gap : 0
    }

    /// The room a docked panel has from the chrome row down to the chat box, or to the
    /// bottom margin when it sits beside the box.
    var verticalRoom: CGFloat {
        let below = sitsBesideComposer ? Self.bottomMargin : composerAreaHeight + Self.gap
        return pane.height - Chrome.contentTopInset - below
    }

    var panelHeight: CGFloat {
        let ideal = compact ? Self.compactHeight : preferred?.height ?? min(520, max(300, pane.height * 0.5))
        let depth = CGFloat(max(1, stackDepth))
        let share = (verticalRoom - (depth - 1) * Self.gap) / depth
        return max(Self.minHeight, min(ideal, share))
    }

    var panelSize: CGSize { CGSize(width: panelWidth, height: panelHeight) }

    /// A size the user is dragging a panel to, kept within what the pane can hold: no
    /// narrower or shorter than the smallest panel, no wider than the pane's margins allow,
    /// no taller than the room from the chrome row to the chat box at that width.
    func fitted(_ size: CGSize) -> CGSize {
        var trial = self
        trial.preferred = CGSize(width: size.width, height: .greatestFiniteMagnitude)
        return CGSize(width: trial.panelWidth, height: max(Self.minHeight, min(size.height, trial.panelHeight)))
    }

    /// The docked spot in a corner: beside the chat column, the group centred, at the top
    /// under the chrome row or at the bottom level with the chat box; or, when the pane is
    /// too narrow for both side by side, in the corner above the box. Panels docked on
    /// both sides take room from both ends of the column, so it narrows to sit between them.
    func dockedOrigin(_ corner: PanelDockCorner, reserve: PanelReserve) -> CGPoint {
        let y = corner.isTop
            ? Chrome.contentTopInset
            : pane.height - (sitsBesideComposer ? Self.bottomMargin : composerAreaHeight + Self.gap) - panelHeight
        if sitsBesideComposer {
            let row = pane.width - reserve.leading - reserve.trailing
            let composerWidth = min(Self.composerMaxWidth, row - 2 * Self.sideMargin)
            let composerLeft = reserve.leading + (row - composerWidth) / 2
            switch corner.side {
            case .leading: return CGPoint(x: composerLeft - Self.gap - panelWidth, y: y)
            case .trailing: return CGPoint(x: composerLeft + composerWidth + Self.gap, y: y)
            }
        }
        return CGPoint(x: corner.side == .leading ? Self.sideMargin : pane.width - Self.sideMargin - panelWidth, y: y)
    }

    /// The corner a panel at `origin` belongs in: the side of the pane its centre is on, and
    /// the nearer of the top and bottom spots. The corner it has keeps it until its centre
    /// is `cornerSlack` past the middle, so a panel held on the line settles on one side.
    func dockCorner(nearest origin: CGPoint, keeping current: PanelDockCorner, docks: PanelDocks) -> PanelDockCorner {
        let centre = CGPoint(x: origin.x + panelWidth / 2, y: origin.y + panelHeight / 2)
        let middle = CGPoint(x: pane.width / 2, y: (docks.topLeading.y + docks.bottomLeading.y + panelHeight) / 2)
        let side: PanelDockSide = switch centre.x - middle.x {
        case ..<(-Self.cornerSlack): .leading
        case Self.cornerSlack...: .trailing
        default: current.side
        }
        let isTop: Bool = switch centre.y - middle.y {
        case ..<(-Self.cornerSlack): true
        case Self.cornerSlack...: false
        default: current.isTop
        }
        switch (side, isTop) {
        case (.leading, true): return .topLeading
        case (.trailing, true): return .topTrailing
        case (.leading, false): return .bottomLeading
        case (.trailing, false): return .bottomTrailing
        }
    }

    /// Which slot of the side's grid of `slots` slots a panel whose top-left
    /// is at `origin` is over: the slot whose y it sits nearest, rounded.
    func slotIndex(at origin: CGPoint, slots: Int, docks: PanelDocks) -> Int {
        let step = slots > 1 ? (docks.bottomLeading.y - docks.topLeading.y) / CGFloat(slots - 1) : 0
        return step > 0 ? min(slots - 1, max(0, Int(((origin.y - docks.topLeading.y) / step).rounded()))) : 0
    }

    /// Keeps a dragged panel inside the pane.
    func clamped(_ origin: CGPoint) -> CGPoint {
        let inset: CGFloat = 8
        return CGPoint(
            x: min(max(origin.x, inset), max(inset, pane.width - panelWidth - inset)),
            y: min(max(origin.y, inset), max(inset, pane.height - panelHeight - inset))
        )
    }
}

/// A helper thread's chat as a small floating glass panel: its conversation, its own chat
/// box at the bottom, and a glass pill in the top-right corner that closes it and stops the
/// chat. The strip along the top is the handle: drag it to move the panel anywhere in the pane.
struct SubagentPanel: View {
    @Environment(AppModel.self) private var model
    @Environment(WindowLiveResize.self) private var liveResize
    @Environment(\.colorScheme) private var colorScheme
    let thread: ChatThread
    let size: CGSize
    let workingDirectory: String?
    let projectName: String?
    /// The pointer's travel since the handle was grabbed.
    let onDrag: (CGSize) -> Void
    let onDragEnd: () -> Void
    let close: () -> Void

    @State private var scrollChrome = ChromeScrollModel()
    @State private var scrollState = TimelineScrollState()
    /// Closing while the helper works stops its turn, so that one is asked for first.
    @State private var isConfirmingClose = false

    private static let cornerRadius: CGFloat = 22
    private static let handleHeight: CGFloat = Chrome.chromeTopPadding + Chrome.capsuleHeight + 6

    var body: some View {
        let runtime = model.runtime(for: thread.id)
        let shape = RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous)
        Group {
            if thread.isHydraHead, !model.settings.hydraShowsHeadDetails {
                // A head shows its task and the progress bar instead of its steps; a
                // helper of the user's own keeps its conversation, since they talk to it.
                HydraHeadProgress(head: thread, runtime: runtime)
            } else {
                ThreadTimeline(
                    runtime: runtime,
                    scrollChrome: scrollChrome,
                    scrollState: scrollState,
                    projectName: projectName,
                    workingDirectory: workingDirectory,
                    supportsRewind: false,
                    columnHeight: size.height,
                    // A panel's transcript needs no rail, and the rail's hover tracking
                    // and geometry reader are per-frame work under the pointer.
                    showsMinimap: false
                )
                .equatable()
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            ComposerArea(runtime: runtime, workingDirectory: workingDirectory, compactModelChip: true, takesFocusOnAppear: false, showsChanges: false)
                .overlay(alignment: .top) {
                    JumpToLatestButton(scrollState: scrollState)
                }
        }
        .overlay(alignment: .top) {
            PaneTopVeil(model: scrollChrome)
        }
        .overlay(alignment: .top) {
            HStack(alignment: .top, spacing: 8) {
                // The handle. Invisible, like a window's title bar with no title; the
                // cursor says what it does.
                PanelDragHandle(onDrag: onDrag, onDragEnd: onDragEnd)
                    .frame(maxWidth: .infinity)
                    .frame(height: Self.handleHeight)
                    .help("Drag to move")
                    .accessibilityLabel(Text("Drag to move"))
                // The working status box at the strip's right end: the sending pulse
                // first, then who is working, in the one capsule.
                SubagentWorkingBox(title: thread.title, isRunning: runtime.isRunning)
                    .padding(.top, Chrome.chromeTopPadding)
                // The same mark as the team panel's, which closes nothing that is working:
                // this one ends a turn, so a helper still at it is asked about first.
                ChromeCircleButton(
                    symbol: "xmark",
                    help: runtime.isRunning ? "Close this chat and stop its turn" : "Close this chat"
                ) {
                    if runtime.isRunning { isConfirmingClose = true } else { close() }
                }
                .padding(.top, Chrome.chromeTopPadding)
                .padding(.trailing, Chrome.chromeHorizontalPadding)
                .popover(isPresented: $isConfirmingClose, arrowEdge: .bottom) {
                    ClosePanelPopover(title: thread.title) { close() }
                }
            }
        }
        .frame(width: size.width, height: size.height)
        // The frame follows the pane every frame of a live resize: any content keyed
        // to it settles after, never during.
        .animation(nil, value: liveResize.isActive)
        .background {
            // The window's own recipe, on the panel: one glass surface, a scrim for the
            // text over whatever the panel floats above, and the theme's tint.
            let isDark = colorScheme == .dark
            shape
                .fill(.clear)
                .glassEffect(.regular, in: shape)
                .overlay {
                    shape.fill(Chrome.glassTint.opacity(isDark ? 0.22 : 0.16))
                }
        }
        // Glass supplies the edge (as in `WindowBackdrop`): a hairline on top of it
        // read as a second ring around the panel whenever its window was key.
        .clipShape(shape)
        // The scrim sits under the glass and carries the panel's shadow: a plain filled
        // shape, so its shadow is drawn once and kept. A shadow on the whole panel was
        // blurred again with every token the transcript streamed under it.
        .background {
            let isDark = colorScheme == .dark
            shape
                .fill((isDark ? Color.black : Color.white).opacity(isDark ? 0.3 : 0.34))
                .shadow(color: .black.opacity(isDark ? 1 : 0.65), radius: 28, y: 10)
        }
    }
}

/// The helper's working status box at the strip's right end, beside the close mark:
/// first the sending pulse, then who is working while they work. One capsule, one
/// anchor, one size and one corner radius across both states, the existing mini
/// spinner and the panel slide for motion, so the send morphs subtly into the working
/// box with no jump and no second row. The drag handle and the close mark keep their
/// actions unchanged.
private struct SubagentWorkingBox: View {
    @Environment(WindowLiveResize.self) private var liveResize
    let title: String
    let isRunning: Bool

    /// Briefly true after work starts, so the box shows the sending pulse inside its
    /// own capsule before settling into the working words.
    @State private var isSending = false

    private var name: String {
        let short = TextCleanup.singleLine(title, limit: 24).trimmingCharacters(in: .whitespacesAndNewlines)
        return short.isEmpty ? "Helper" : short
    }

    var body: some View {
        Group {
            if isRunning || isSending {
                HStack(spacing: 6) {
                    MiniSpinner()
                    Text(verbatim: isSending && !isRunning ? "Sending…" : "\(name) working")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(1)
                        .contentTransition(.opacity)
                }
                .padding(.horizontal, Chrome.capsuleHorizontalPadding)
                .frame(height: Chrome.capsuleContentHeight)
                .padding(.vertical, Chrome.capsuleVerticalPadding)
                .fixedSize()
                .chromeGlassCapsule()
                .transition(.softAppear)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(Text(isRunning ? "\(name) working" : "Sending"))
            }
        }
        // Held while the window is resized: the panel's frame moves every frame.
        .animation(liveResize.isActive ? nil : Chrome.panelSlide, value: isRunning)
        .animation(liveResize.isActive ? nil : Chrome.panelSlide, value: isSending)
        .onChange(of: isRunning) { _, running in
            if running { isSending = true }
        }
        .task(id: isSending) {
            guard isSending else { return }
            try? await Task.sleep(for: .seconds(0.9))
            guard !Task.isCancelled else { return }
            isSending = false
        }
    }
}

/// What the helper panel's close mark asks while its turn is running, in the popover the
/// sidebar's delete question uses: the stop is the destructive row, and clicking away keeps
/// the helper working.
private struct ClosePanelPopover: View {
    let title: String
    let onClose: () -> Void

    var body: some View {
        PopoverMenu {
            PopoverSectionHeader("Stop this helper?")
            PopoverNote("“\(title)” is still working. Closing the panel stops its turn. What it has done so far stays, and its chat moves to the sidebar under this one.")
            PopoverDivider()
            PopoverItem("Close and stop", symbol: "stop.circle", isDestructive: true) { onClose() }
            PopoverItem("Keep it working", symbol: "arrow.uturn.backward") {}
        }
        .frame(width: 300)
    }
}

/// A floating panel's place while its handle is held: the corner under the pointer, and
/// the flick it is let go with. Its own object rather than chat state on purpose: only the
/// panel's placement reads it, so each pointer move re-lays out that one offset and nothing
/// else in the chat.
@Observable @MainActor
final class PanelDragState {
    /// The panel's corner under the pointer, moved with no animation; nil at rest.
    private(set) var position: CGPoint?
    /// Where the panel was when the handle took hold, so each move is measured from there.
    @ObservationIgnored private var grab: CGPoint?
    /// The pointer's speed over the last few moves, in points a second, for the flick.
    @ObservationIgnored private var velocity = CGVector.zero
    @ObservationIgnored private var lastMove: TimeInterval = 0

    /// How far the pointer's speed carries the panel past where it is let go; a flick
    /// towards a corner lands there without the pointer having to reach it.
    private static let flickCarry: TimeInterval = 0.12
    /// A pointer that has rested this long before letting go drops the panel where it is.
    private static let restBeforeDrop: TimeInterval = 0.08

    /// The panel's corner once the pointer has travelled `translation` from where its
    /// handle was grabbed while the panel sat at `rest`, kept inside the pane.
    func move(by translation: CGSize, from rest: CGPoint, in layout: SubagentPanelLayout) -> CGPoint {
        let start = grab ?? rest
        grab = start
        let next = layout.clamped(CGPoint(x: start.x + translation.width, y: start.y + translation.height))
        let now = CACurrentMediaTime()
        if let position {
            let elapsed = now - lastMove
            // Two events in the same frame say nothing about speed; the last reading stands.
            if elapsed >= 0.004 {
                let instant = CGVector(dx: (next.x - position.x) / elapsed, dy: (next.y - position.y) / elapsed)
                velocity = CGVector(dx: velocity.dx * 0.4 + instant.dx * 0.6, dy: velocity.dy * 0.4 + instant.dy * 0.6)
            }
        } else {
            velocity = .zero
        }
        lastMove = now
        position = next
        return next
    }

    /// Pulls a panel dragged to a free spot back inside the pane: the pane shrinks
    /// under it in a live resize. Only writes when the spot is actually outside.
    func reclamp(in layout: SubagentPanelLayout) {
        guard let position else { return }
        let next = layout.clamped(position)
        if next != position { self.position = next }
    }

    /// Lets the panel go, and says where it was heading: where it is, carried on a little
    /// by the flick it was released with. Nil when it was never moved.
    func release() -> CGPoint? {
        defer {
            position = nil
            grab = nil
            velocity = .zero
        }
        guard let position else { return nil }
        guard CACurrentMediaTime() - lastMove < Self.restBeforeDrop else { return position }
        return CGPoint(x: position.x + velocity.dx * Self.flickCarry, y: position.y + velocity.dy * Self.flickCarry)
    }
}

/// A floating panel at its docked spot, or under the pointer while its handle is held.
/// The handle moves it live; a drop and a resize settle it with a slide, as does the
/// panel's size when another panel docks into its corner or leaves it. Only this view
/// observes the drag, so the chat around the panel is left alone while it moves.
/// A window live resize holds the settle slide too: the dock follows the pane every
/// frame, so a spring restarted per frame is what made panels lag and land elsewhere.
struct PlacedPanel<Content: View>: View {
    let drag: PanelDragState
    /// The panel's docked spot.
    let rest: CGPoint
    /// The panel's size, which the corner's stack decides.
    let size: CGSize
    let content: Content
    /// The grips along the panel's two free edges, when it can be resized.
    var resize: PanelResize? = nil
    /// True while a grip is held: the size follows the pointer with no animation.
    var isResizing = false

    var body: some View {
        content
            .overlay {
                if let resize {
                    PanelResizeGrips(resize: resize)
                }
            }
            .modifier(PanelPlacement(drag: drag, rest: rest, size: size, isResizing: isResizing))
    }
}

/// Places the panel: at its docked spot, or under the pointer while held. Only this
/// modifier observes the drag, and a modifier's content is a placeholder, so the
/// panel's own body is never re-run for a pointer move.
private struct PanelPlacement: ViewModifier {
    @Environment(WindowLiveResize.self) private var liveResize
    let drag: PanelDragState
    let rest: CGPoint
    let size: CGSize
    let isResizing: Bool

    func body(content: Content) -> some View {
        let origin = drag.position ?? rest
        // Read now, so the slide turning off and on is one settle from the current
        // spot: an origin kept from before the resize would swing in from stale.
        let settles = drag.position == nil && !isResizing && !liveResize.isActive
        content
            .offset(x: origin.x, y: origin.y)
            .animation(settles ? Chrome.panelSlide : nil, value: origin)
            .animation(settles ? Chrome.panelSlide : nil, value: size)
    }
}

/// Which of a panel's dimensions a grip changes.
struct PanelResizeAxes: OptionSet {
    let rawValue: Int
    static let width = PanelResizeAxes(rawValue: 1)
    static let height = PanelResizeAxes(rawValue: 2)
    static let both: PanelResizeAxes = [.width, .height]
}

/// How a docked panel is resized: from the corner it docks in, the two edges facing the
/// chat are free, and pulling them grows the panel away from its corner.
struct PanelResize {
    /// The corner the panel docks in; the grips sit on the opposite edges.
    let corner: PanelDockCorner
    /// The pointer's travel since a grip was grabbed, and which dimensions that grip moves.
    let onResize: (CGSize, PanelResizeAxes) -> Void
    let onResizeEnd: () -> Void
    /// A double-click on a grip: back to the automatic size.
    let onReset: () -> Void
}

/// The size a panel had when a grip took hold, so each move is measured from there.
@Observable @MainActor
final class PanelResizeState {
    private(set) var start: CGSize?
    /// The size under the pointer while a grip is held; the layout reads it in place of
    /// the stored size, which is written once when the grip lets go.
    var liveSize: CGSize?
    /// The usage panel's own height under the pointer, likewise.
    var liveUsageHeight: CGFloat?

    var isActive: Bool { start != nil }

    /// The size to measure from: the one remembered from this grab, else `size`, remembered.
    func begin(at size: CGSize) -> CGSize {
        if let start { return start }
        start = size
        return size
    }

    func end() { start = nil }
}

/// The grips: a thin strip along each free edge for one dimension, and a square at the
/// free corner for both. Invisible, like a window's edges; the cursor says what they do.
private struct PanelResizeGrips: View {
    let resize: PanelResize

    private static let edge: CGFloat = 6
    private static let corner: CGFloat = 16

    var body: some View {
        let dock = resize.corner
        // The free edges are the ones away from the dock corner.
        let freeSide: HorizontalAlignment = dock.side == .leading ? .trailing : .leading
        let freeEdge: VerticalAlignment = dock.isTop ? .bottom : .top
        ZStack {
            grip(.width, position: dock.side == .leading ? .right : .left)
                .frame(width: Self.edge)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: Alignment(horizontal: freeSide, vertical: .center))
            grip(.height, position: dock.isTop ? .bottom : .top)
                .frame(height: Self.edge)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: Alignment(horizontal: .center, vertical: freeEdge))
            grip(.both, position: Self.cornerPosition(of: dock))
                .frame(width: Self.corner, height: Self.corner)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: Alignment(horizontal: freeSide, vertical: freeEdge))
        }
    }

    private func grip(_ axes: PanelResizeAxes, position: NSCursor.FrameResizePosition) -> some View {
        PanelDragHandle(
            onDrag: { resize.onResize($0, axes) },
            onDragEnd: resize.onResizeEnd,
            cursor: .frameResize(position: position, directions: .all),
            dragCursor: .frameResize(position: position, directions: .all),
            onDoubleClick: resize.onReset
        )
        .help("Drag to resize · Double-click for the automatic size")
        .accessibilityLabel(Text("Drag to resize"))
    }

    /// The free corner's cursor: opposite the dock corner.
    private static func cornerPosition(of dock: PanelDockCorner) -> NSCursor.FrameResizePosition {
        switch dock {
        case .topLeading: .bottomRight
        case .topTrailing: .bottomLeft
        case .bottomLeading: .topRight
        case .bottomTrailing: .topLeft
        }
    }
}

/// The strip a floating panel is dragged by. AppKit tracks the press itself, the way the
/// sidebar's resize grip does: the first click takes hold even when the window is not key,
/// nothing under the strip can claim the drag once it has begun, and the hand cursor is a
/// real cursor rect. A SwiftUI drag gesture here lost most presses to the transcript's
/// scroll view beneath it. Reports the pointer's travel since the press in the pane's
/// coordinates, y running down like the panel's offset.
struct PanelDragHandle: NSViewRepresentable {
    let onDrag: (CGSize) -> Void
    let onDragEnd: () -> Void
    /// The cursor over the handle, and the one while it is held: the hand for a move, a
    /// resize cursor for a grip.
    var cursor: NSCursor = .openHand
    var dragCursor: NSCursor = .closedHand
    var onDoubleClick: (() -> Void)? = nil

    func makeNSView(context: Context) -> PanelDragHandleView {
        let view = PanelDragHandleView(frame: .zero)
        update(view)
        return view
    }

    func updateNSView(_ nsView: PanelDragHandleView, context: Context) {
        update(nsView)
    }

    private func update(_ view: PanelDragHandleView) {
        view.onDrag = onDrag
        view.onDragEnd = onDragEnd
        view.onDoubleClick = onDoubleClick
        view.dragCursor = dragCursor
        if view.cursor !== cursor {
            view.cursor = cursor
            view.window?.invalidateCursorRects(for: view)
        }
    }
}

final class PanelDragHandleView: NSView {
    var onDrag: ((CGSize) -> Void)?
    var onDragEnd: (() -> Void)?
    var onDoubleClick: (() -> Void)?
    var cursor: NSCursor = .openHand
    var dragCursor: NSCursor = .closedHand

    /// How far the pointer travels before a press becomes a drag.
    private static let slop: CGFloat = 2

    private var pressOrigin: NSPoint = .zero
    private var isDragging = false

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// A press on the handle moves the panel, never the window.
    override var mouseDownCanMoveWindow: Bool { false }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: cursor)
    }

    override func mouseDown(with event: NSEvent) {
        pressOrigin = event.locationInWindow
        isDragging = false
        if event.clickCount == 2 { onDoubleClick?() }
    }

    override func mouseDragged(with event: NSEvent) {
        let travel = travel(to: event)
        if !isDragging {
            guard abs(travel.width) >= Self.slop || abs(travel.height) >= Self.slop else { return }
            isDragging = true
            dragCursor.push()
        }
        onDrag?(travel)
    }

    override func mouseUp(with event: NSEvent) {
        guard isDragging else { return }
        isDragging = false
        NSCursor.pop()
        onDrag?(travel(to: event))
        onDragEnd?()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Taken out mid-drag, the panel lets go of the cursor with it.
        if window == nil, isDragging {
            isDragging = false
            NSCursor.pop()
        }
    }

    /// The pointer's travel since the press. AppKit's y runs up; the panel's offset runs down.
    private func travel(to event: NSEvent) -> CGSize {
        let point = event.locationInWindow
        return CGSize(width: point.x - pressOrigin.x, height: pressOrigin.y - point.y)
    }
}
