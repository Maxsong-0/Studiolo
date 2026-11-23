// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct ChatView: View {
    @Environment(AppModel.self) private var model
    @Environment(WindowLiveResize.self) private var liveResize
    @Bindable var runtime: ThreadRuntime

    @State private var git = GitStatusModel()
    @State private var scrollChrome = ChromeScrollModel()
    @State private var scrollState = TimelineScrollState()
    /// Full column height. The timeline rail centres in this, so composer and
    /// queue growth never shifts it.
    @State private var columnHeight: CGFloat = 0
    /// The conversation pane (the timeline with the chat box, above any terminal), which
    /// the helper panel floats in.
    @State private var paneSize: CGSize = .zero
    /// The chat box with its tabs and cards, so a helper panel above it clears them.
    @State private var composerAreaHeight: CGFloat = 0
    /// A panel under the pointer while its handle is held. Objects, so a move re-lays
    /// out one panel.
    @State private var subagentDrag = PanelDragState()
    @State private var usageDrag = PanelDragState()
    /// What the usage panel's content asked for, so the panel is no taller than its rows
    /// (see `usagePanel`).
    @State private var usageContentHeight: CGFloat?
    /// One grip held at a time, whichever panel it is on.
    @State private var panelResize = PanelResizeState()
    /// Whether a panel is under the pointer right now, on any of the drag states above.
    ///
    /// While one is, the conversation stops answering hit tests. Every pointer move during
    /// a drag is a hover event, and SwiftUI answers one by walking the responder tree and
    /// rendering the view graph again — through the timeline, which is the largest tree in
    /// the window. With several panels open that walk is what makes a drag stutter: a spin
    /// report taken mid-drag spends 326 of 364 samples under NSHostingView.layout(), most
    /// of it in ViewGraphRootValueUpdater.render, reached from
    /// EventBindingManager.enqueueHoverUpdateIfNeeded.
    ///
    /// Nothing in the conversation needs to answer a hover while a panel is being moved,
    /// so for the length of the drag it does not.
    private var isMovingAPanel: Bool {
        subagentDrag.position != nil || usageDrag.position != nil
    }
    /// Whether the panels and the room the chat makes for them follow the pointer with
    /// no spring: a panel grip or either sidebar's edge is held. A sidebar drag moves the
    /// pane every frame the way a live resize does; a spring restarted per frame lagged
    /// the panels behind and swam. A sidebar's open or close slide is held the same way
    /// for its run.
    private var panelsHeld: Bool {
        panelResize.isActive
            || model.sidebar.isDragging || model.sidebar.isSliding
            || model.rightSidebar.isDragging || model.rightSidebar.isSliding
            || runtime.isTerminalResizing
    }
    /// The thread the scene was last laid out for. A body pass on a different thread is a
    /// switch: panels and reserves snap to the new thread's layout rather than glide from
    /// the old one's.
    @State private var sceneThreadID: UUID?

    var body: some View {
        let thread = model.thread(runtime.threadID)
        let project = thread.flatMap { model.project($0.projectID) }
        // The thread and project are read here, once, and passed down as values: the timeline
        // rows and the composer never observe them, so a change to either re-renders this
        // body alone and the children only where what they were handed changed.
        let workingDirectory = thread.flatMap { $0.worktreePath ?? project?.path }
        let directory = workingDirectory ?? LoginEnvironment.homeDirectory
        let title = thread?.title ?? ""
        // Who is in a panel comes from the model and changes rarely; the geometry
        // comes from the measured pane and changes every frame of a live resize.
        // Split, so a resize frame recomputes docks and reserves only, never the
        // thread lists.
        let members = PanelMembers(runtime: runtime, model: model)
        let scene = PanelScene(members: members, runtime: runtime, model: model, paneSize: paneSize, composerAreaHeight: composerAreaHeight, usageContentHeight: usageContentHeight, preferredSize: livePreferredSize, usageHeightOverride: panelResize.liveUsageHeight)
        let holds = panelsHeld
        let switching = sceneThreadID != runtime.threadID
        VStack(spacing: 0) {
            ThreadTimeline(
                runtime: runtime,
                scrollChrome: scrollChrome,
                scrollState: scrollState,
                projectName: project?.name,
                workingDirectory: workingDirectory,
                allowsUndo: true,
                columnHeight: columnHeight,
                // A panel docked on the left takes the rail's edge; the rail goes with it. So
                // does a pane too narrow for both: the column reads at up to 820 points and the
                // rail floats over its gutter, so under about 900 the two would overlap.
                showsMinimap: !scene.dockedSides.contains(.leading)
                    && (paneSize == .zero || paneSize.width - scene.reserve.leading - scene.reserve.trailing >= 900)
            )
            .equatable()
            // Hover costs a responder walk and a render of this whole tree; a panel being
            // dragged sends one per pointer move and needs none of them (see
            // `isMovingAPanel`).
            .allowsHitTesting(!isMovingAPanel)
            // The timeline is the one part swapped per thread: the column around it (chrome
            // row, chat box, panels, git status, measured sizes) stays mounted, so a switch
            // re-lays out nothing else.
            .id(runtime.threadID)
            .environment(\.recentStepsShown, model.settings.recentStepsShown)
            // The room the docked panels take from either side; the conversation and the
            // box centre in the rest, so they stay lined up with each other (see
            // `ReserveSlide` for how they get there without laying out on every frame).
            .modifier(ReserveSlide(reserve: scene.reserve, slides: scene.isMeasured && !liveResize.isActive && !holds && !switching))
            .safeAreaInset(edge: .bottom, spacing: 0) {
                ComposerArea(runtime: runtime, workingDirectory: workingDirectory)
                    .overlay(alignment: .top) {
                        JumpToLatestButton(scrollState: scrollState)
                    }
                    .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { composerAreaHeight = $0 }
                    .onChange(of: composerAreaHeight) { oldValue, newValue in
                        // The area growing (the queued follow-ups tab expanding, a
                        // follow-up being queued, an approval card arriving) shrinks the
                        // timeline's viewport from its bottom edge. The timeline pulls its
                        // end to the new edge for a reader who is at it, and leaves a
                        // reader who scrolled up to read exactly where they are: the
                        // growth is the composer's, never the reader's cue to jump. Not off
                        // the first measurement or a thread switch's fresh scroll state.
                        guard newValue > oldValue + 0.5, !liveResize.isActive, oldValue > 0 else { return }
                        scrollState.noteComposerAreaGrowth()
                    }
                    .modifier(ReserveSlide(reserve: scene.reserve, slides: scene.isMeasured && !liveResize.isActive && !holds && !switching, animatesWidth: true))
            }
            .overlay(alignment: .top) {
                PaneTopVeil(model: scrollChrome)
            }
            .overlay(alignment: .top) {
                ChatChromeRow(
                    runtime: runtime,
                    scrollChrome: scrollChrome,
                    title: title,
                    project: project,
                    directory: directory,
                    git: git
                )
            }
            .overlay(alignment: .topLeading) {
                ZStack(alignment: .topLeading) {
                    // Not before the pane has a size: the panel's spot comes from it.
                    if let subagent = scene.subagent, scene.isMeasured {
                        subagentPanel(subagent, scene: scene, project: project)
                    }
                }
                .animation(switching ? nil : Chrome.panelSlide, value: scene.subagent?.id)
            }
            .overlay(alignment: .topLeading) {
                ZStack(alignment: .topLeading) {
                    if let usage = scene.usage, scene.showsUsage {
                        usagePanel(usage, scene: scene)
                    }
                }
                .animation(switching ? nil : Chrome.panelSlide, value: scene.usage)
            }
            .onGeometryChange(for: CGSize.self, of: { $0.size }) { paneSize = $0 }
            // Free-floating panels are kept inside the pane while it shrinks, with no
            // animation; when the resize ends they land docked exactly (see below).
            .onChange(of: paneSize) {
                guard liveResize.isActive else { return }
                // The scene above was worked out for this pane size already.
                let layout = scene.layout
                subagentDrag.reclamp(in: layout)
                usageDrag.reclamp(in: layout)
            }
            // Either sidebar's drag reshapes the pane per frame the way a live resize
            // does, and so does its toggle slide; the timeline and composer read one
            // signal for all of them.
            .onChange(of: panelsHeld, initial: true) { _, dragging in
                liveResize.isPaneResizing = dragging
            }
            // The pane's final size: docked panels land on their new docked spot, and
            // a free spot left outside is pulled back inside.
            .onChange(of: liveResize.isActive) { _, resizing in
                guard !resizing else { return }
                subagentDrag.reclamp(in: scene.layout)
                usageDrag.reclamp(in: scene.layout)
            }

            if runtime.isTerminalVisible {
                TerminalPanel(runtime: runtime, directory: directory)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, maxHeight: .infinity)
        .detailSheet()
        // The column stays mounted across a thread switch; only the timeline is swapped
        // (see its `.id(runtime.threadID)`). That swap must replace in place, never
        // slide or fade. Any transition here displaces every row, the composer and
        // the chrome row mid-flight, clips leading text, unseats the traffic lights
        // the chrome keeps room for, and doubles the title against itself.
        .transition(.identity)
        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { columnHeight = $0 }
        .animation(switching ? nil : Chrome.panelSlide, value: runtime.isTerminalVisible)
        // A merge or pull request link in this chat can hand its request to a helper, which
        // opens in the panel and lands it on the same provider and model as this thread.
        .environment(\.mergeRequestTarget, MergeRequestTarget(chatID: runtime.threadID) { link in
            model.spawnSubagent(from: runtime.threadID, title: link.title, prompt: link.prompt)
        })
        .task(id: directory) {
            await git.refresh(directory, force: false)
        }
        .onChange(of: runtime.diffRevision) {
            Task { await git.refresh(directory) }
        }
        // The column is not rebuilt on a thread switch, so what belonged to the previous
        // thread's timeline and panels starts fresh here. The measured sizes
        // (`columnHeight`, `paneSize`, `composerAreaHeight`) are the pane's, not the
        // thread's, and stay.
        .onChange(of: runtime.threadID) {
            sceneThreadID = runtime.threadID
            scrollChrome = ChromeScrollModel()
            scrollState = TimelineScrollState()
            subagentDrag = PanelDragState()
            usageDrag = PanelDragState()
            panelResize = PanelResizeState()
        }
        .onAppear { sceneThreadID = runtime.threadID }
    }

    // MARK: - Floating panels

    /// The helper's panel, in its corner.
    private func subagentPanel(_ subagent: ChatThread, scene: PanelScene, project: Project?) -> some View {
        let slot = scene.slots[.subagent] ?? PanelScene.PanelSlot(corner: runtime.subagentPanelDock, below: 0, lift: 0)
        let rest = Self.rest(for: slot, scene: scene)
        return PlacedPanel(drag: subagentDrag, rest: rest, size: scene.layout.panelSize, content: SubagentPanel(
            thread: subagent,
            size: scene.layout.panelSize,
            // The helper works in the project folder, whatever this thread's worktree.
            workingDirectory: subagent.worktreePath ?? project?.path,
            projectName: project?.name,
            onDrag: { translation in
                let position = subagentDrag.move(by: translation, from: rest, in: scene.layout)
                dock(.subagent, \.subagentPanelDock, nearest: position, scene: scene)
                reorder(.subagent, at: position, corner: runtime.subagentPanelDock, scene: scene)
            },
            onDragEnd: {
                if let heading = subagentDrag.release() {
                    dock(.subagent, \.subagentPanelDock, nearest: heading, scene: scene)
                    reorder(.subagent, at: heading, corner: runtime.subagentPanelDock, scene: scene)
                }
            },
            close: {
                // The helper moves to the sidebar under this thread; its row opens with the
                // same slide as the panel leaving.
                withAnimation(Chrome.panelSlide) {
                    model.closeSubagent(subagent.id)
                }
            }
        )
        // Inside the offset, so the panel grows in and fades out in place.
        .transition(Self.panelTransition), resize: resizer(for: runtime.subagentPanelDock, scene: scene), isResizing: panelsHeld)
    }

    /// The usage panel: beyond whichever panels are in the same corner, so it is the last
    /// of a stack. Across the column from the hands by default, at the left.
    private func usagePanel(_ usage: PanelUsage, scene: PanelScene) -> some View {
        let slot = scene.slots[.usage] ?? PanelScene.PanelSlot(corner: model.settings.usagePanelDock, below: 0, lift: 0)
        let corner = slot.corner
        let rest = Self.rest(for: slot, scene: scene)
        let full = scene.layout.panelSize
        let minimum = PanelScene.usageMinimumHeight
        // The panel's own height, worked out with the scene (see `PanelScene.usageHeight`).
        let height = scene.usageHeight ?? full.height
        let size = CGSize(width: full.width, height: height)
        // A bottom-docked panel keeps its bottom edge where the full-height one would
        // sit; a top-docked one keeps its top.
        let origin = corner.isTop ? rest : CGPoint(x: rest.x, y: rest.y + (full.height - height))
        return PlacedPanel(drag: usageDrag, rest: origin, size: size, content: UsageFloatingPanel(
            runtime: runtime,
            provider: usage.provider,
            handsProvider: usage.handsProvider,
            size: size,
            onContentHeight: { wanted in if usageContentHeight != wanted { usageContentHeight = wanted } },
            onDrag: { translation in
                let position = usageDrag.move(by: translation, from: origin, in: scene.layout)
                dockUsage(nearest: position, scene: scene)
                reorder(.usage, at: position, corner: model.settings.usagePanelDock, scene: scene)
            },
            onDragEnd: {
                if let heading = usageDrag.release() {
                    dockUsage(nearest: heading, scene: scene)
                    reorder(.usage, at: heading, corner: model.settings.usagePanelDock, scene: scene)
                }
            },
            dismiss: {
                // For every chat; the usage popover's pop-out button brings it back.
                withAnimation(Chrome.panelSlide) {
                    model.settings.showsUsagePanel = false
                }
            }
        )
        .transition(Self.panelTransition), resize: usageResizer(for: corner, scene: scene, size: size, minimum: minimum), isResizing: panelsHeld)
    }

    /// The grips of a panel docked in `corner`. A pull on a free edge grows the panel away
    /// from its corner; the size is kept in Settings for every chat's panels, fitted to
    /// this pane's room, so it follows the pointer here and the window's size after.
    private func resizer(for corner: PanelDockCorner, scene: PanelScene) -> PanelResize {
        PanelResize(
            corner: corner,
            onResize: { translation, axes in
                let start = panelResize.begin(at: scene.layout.panelSize)
                // The pointer's travel runs right and down; a panel docked at the right
                // grows leftwards, one docked at the bottom grows upwards.
                let acrossSign: CGFloat = corner.side == .trailing ? -1 : 1
                let downSign: CGFloat = corner.isTop ? 1 : -1
                var next = start
                if axes.contains(.width) { next.width += translation.width * acrossSign }
                if axes.contains(.height) { next.height += translation.height * downSign }
                panelResize.liveSize = scene.layout.fitted(next)
            },
            onResizeEnd: commitPanelResize,
            onReset: {
                withAnimation(Chrome.panelSlide) { model.settings.panelSize = nil }
            }
        )
    }

    /// The usage panel's grips: its width is the panels' shared width, its height its own
    /// (see `AppSettings.usagePanelHeight`); a double-click puts it back to fitting its rows.
    private func usageResizer(for corner: PanelDockCorner, scene: PanelScene, size: CGSize, minimum: CGFloat) -> PanelResize {
        PanelResize(
            corner: corner,
            onResize: { translation, axes in
                let start = panelResize.begin(at: size)
                // The pointer's travel runs right and down; a panel docked at the right
                // grows leftwards, one docked at the bottom grows upwards.
                let acrossSign: CGFloat = corner.side == .trailing ? -1 : 1
                let downSign: CGFloat = corner.isTop ? 1 : -1
                if axes.contains(.width) {
                    var next = scene.layout.panelSize
                    next.width = start.width + translation.width * acrossSign
                    panelResize.liveSize = scene.layout.fitted(next)
                }
                if axes.contains(.height) {
                    panelResize.liveUsageHeight = max(minimum, start.height + translation.height * downSign)
                }
            },
            onResizeEnd: commitPanelResize,
            onReset: {
                withAnimation(Chrome.panelSlide) { model.settings.usagePanelHeight = nil }
            }
        )
    }

    /// The grip let go: the size it was pulled to is written to Settings once, exact.
    /// While the grip was held only `panelResize` changed, so the pointer moved the
    /// panel and the room it takes, not every chat's stored size.
    private func commitPanelResize() {
        if let size = panelResize.liveSize { model.settings.panelSize = size }
        if let height = panelResize.liveUsageHeight { model.settings.usagePanelHeight = height }
        panelResize.liveSize = nil
        panelResize.liveUsageHeight = nil
        panelResize.end()
    }

    /// The size the layout prefers while a grip is held, else nil for the stored one. The
    /// width is stepped to 12pt while held, so the timeline re-wraps a few times over a
    /// drag rather than every frame; the committed size keeps the exact value.
    private var livePreferredSize: CGSize? {
        guard panelResize.isActive, let live = panelResize.liveSize else { return nil }
        return CGSize(width: (live.width / 12).rounded() * 12, height: live.height)
    }

    /// A panel grows in and fades out in place.
    private static var panelTransition: AnyTransition {
        .asymmetric(
            insertion: .scale(scale: 0.92).combined(with: .opacity),
            removal: .scale(scale: 0.96).combined(with: .opacity)
        )
    }

    /// Docks a panel in the corner nearest to `position`. Called as the panel is held as
    /// well as when it is let go, so the chat makes room on the new side while the panel
    /// is still under the pointer, and the panel has its spot the moment it is dropped.
    private func dock(_ id: FloatingPanelID, _ corner: ReferenceWritableKeyPath<ThreadRuntime, PanelDockCorner>, nearest position: CGPoint, scene: PanelScene) {
        let wanted = scene.layout.dockCorner(nearest: position, keeping: runtime[keyPath: corner], docks: scene.docks)
        let locked = scene.lockedSide(excluding: id)
        // Docked right, the corner is the column's; the drag only reorders (see `reorder`).
        let next = model.settings.panelsDockRight ? PanelDockCorner.topTrailing : (locked.map { PanelDockCorner.make(side: $0, isTop: wanted.isTop) } ?? wanted)
        guard next != runtime[keyPath: corner] else { return }
        runtime[keyPath: corner] = next
    }

    /// The usage panel's corner is one for every chat (see `AppSettings.usagePanelDock`).
    private func dockUsage(nearest position: CGPoint, scene: PanelScene) {
        let wanted = scene.layout.dockCorner(nearest: position, keeping: model.settings.usagePanelDock, docks: scene.docks)
        let locked = scene.lockedSide(excluding: .usage)
        let next = model.settings.panelsDockRight ? PanelDockCorner.topTrailing : (locked.map { PanelDockCorner.make(side: $0, isTop: wanted.isTop) } ?? wanted)
        guard next != model.settings.usagePanelDock else { return }
        model.settings.usagePanelDock = next
    }

    /// The held panel takes the slot it is over on its side, live: a free slot
    /// is taken, a taken one on a full side is swapped for the panel's old
    /// slot, otherwise the occupant and the rest move one slot along; the
    /// corners and the order are rewritten from the slots.
    private func reorder(_ id: FloatingPanelID, at position: CGPoint, corner: PanelDockCorner, scene: PanelScene) {
        let side = corner.side
        let others = scene.slots.filter { $0.value.corner.side == side && $0.key != id }
        let n = max(scene.slotsOnSide[side] ?? 1, others.count + 1)
        let target = scene.layout.slotIndex(at: position, slots: n, docks: scene.docks)
        var occupancy: [Int: FloatingPanelID] = [:]
        for (panelID, slot) in others {
            occupancy[slot.corner.isTop ? slot.below : n - 1 - slot.below] = panelID
        }
        let previous: Int? = scene.slots[id].flatMap { $0.corner.side == side ? ($0.corner.isTop ? $0.below : n - 1 - $0.below) : nil }
        let full = !(others.count + 1 > (scene.slotsOnSide[side] ?? 1))
        if let other = occupancy[target], previous != nil, full {
            occupancy[previous!] = other
            occupancy[target] = id
        } else if let other = occupancy[target] {
            occupancy.removeValue(forKey: target)
            occupancy[target] = id
            let forward = previous == nil || previous! < target
            var displaced = other
            var next = target + (forward ? 1 : -1)
            while true {
                if next < 0 || next >= n {
                    let free = forward ? (0..<n).first(where: { occupancy[$0] == nil }) : (0..<n).reversed().first(where: { occupancy[$0] == nil })
                    if let free { occupancy[free] = displaced }
                    break
                }
                if occupancy[next] == nil {
                    occupancy[next] = displaced
                    break
                }
                let occupant = occupancy[next]!
                occupancy[next] = displaced
                displaced = occupant
                next += forward ? 1 : -1
            }
        } else {
            occupancy[target] = id
        }
        for (index, panelID) in occupancy {
            let (corner, _) = PanelDocks.corner(forSlot: index, of: n, side: side)
            setCorner(panelID, corner)
        }
        let occupiedCorners = Dictionary(uniqueKeysWithValues: occupancy.map { ($0.value, PanelDocks.corner(forSlot: $0.key, of: n, side: side).corner) })
        let top = occupancy.filter { occupiedCorners[$0.value]?.isTop == true }.sorted { $0.key < $1.key }.map(\.value)
        let bottom = occupancy.filter { occupiedCorners[$0.value]?.isTop == false }.sorted { $0.key > $1.key }.map(\.value)
        let sideOrder = top + bottom
        let onSide = Set(occupancy.values)
        let kept = runtime.panelStackOrder.filter { !onSide.contains($0) && $0 != id }
        let next = kept + sideOrder
        if next != runtime.panelStackOrder { runtime.panelStackOrder = next }
    }

    private func setCorner(_ id: FloatingPanelID, _ corner: PanelDockCorner) {
        switch id {
        case .subagent:
            if runtime.subagentPanelDock != corner { runtime.subagentPanelDock = corner }
        case .usage:
            if model.settings.usagePanelDock != corner { model.settings.usagePanelDock = corner }
        }
    }

    private static func rest(for slot: PanelScene.PanelSlot, scene: PanelScene) -> CGPoint {
        let stacked = scene.docks.stacked(slot.corner, below: slot.below, slots: scene.slotsOnSide[slot.corner.side] ?? 1, layout: scene.layout)
        return CGPoint(x: stacked.x, y: slot.corner.isTop ? stacked.y + slot.lift : stacked.y - slot.lift)
    }
}

/// Makes room for the docked panels on either side and slides the content to its new
/// place, without laying it out at every width in between. Animating the padding itself
/// re-wrapped every visible message and the chat box on each frame of the spring, which
/// stuttered on a long conversation; here the padding snaps (one layout, at the final
/// width) and the content starts where it was and glides over on a transform, which
/// costs nothing per frame. The slide is keyed to a change in the room, never to a
/// measurement: a pane measured for the first time lays out in place, and a window resize
/// or a panel's width grip moves things at once, as they should (`slides` is off then).
/// What the Crew linger watches: the team panel's presence, and which thread it is on.
extension EnvironmentValues {
    /// How far `ReserveSlide` has pushed its content off the column's leading edge right
    /// now: the leading room a docked panel takes plus the glide that carries the rows
    /// there. The minimap rail cancels it, so it stays on the edge while the rows move.
    @Entry var reserveSlideLeading: CGFloat = 0
    /// The glide alone, so a view can follow the snap and the glide with the rows but
    /// ignore the padding.
    @Entry var reserveSlideOffset: CGFloat = 0
}

private struct ReserveSlide: ViewModifier {
    let reserve: PanelReserve
    let slides: Bool
    /// Whether the padding itself glides. The box is one pill and cheap to lay out, so
    /// its width follows the room; the conversation's rows are not, so its padding
    /// snaps and the offset below carries it.
    var animatesWidth = false

    @State private var offset: CGFloat = 0

    func body(content: Content) -> some View {
        content
            .environment(\.reserveSlideLeading, reserve.leading)
            .environment(\.reserveSlideOffset, offset)
            .padding(.leading, reserve.leading)
            .padding(.trailing, reserve.trailing)
            // Only a dock or undock glides; while a panel is dragged or resized the room
            // changes every frame, and the box follows the pointer without a spring.
            .animation(animatesWidth && slides ? Chrome.panelSlide : nil, value: reserve)
            .offset(x: offset)
            .onChange(of: reserve) { old, new in
                // The column centres in the room left, so its centre moves by half the
                // change in the room on either side: start from where it was and glide.
                let shift = ((new.leading - new.trailing) - (old.leading - old.trailing)) / 2
                guard slides, !animatesWidth, shift != 0 else {
                    offset = 0
                    return
                }
                var snap = Transaction()
                snap.disablesAnimations = true
                withTransaction(snap) { offset -= shift }
                withAnimation(Chrome.panelSlide) { offset = 0 }
            }
    }
}

/// Which threads sit in the chat's floating panels. Read from the model once per
/// body, apart from the pane measurements: a live resize then re-lays out geometry
/// without walking any thread list again.
@MainActor
private struct PanelMembers {
    let subagent: ChatThread?
    let isDocked: Bool
    /// The usage panel's providers, when this chat shows one: the chat's own, and the
    /// pair's hands provider when that is another, so both plans' limits stack.
    let usage: PanelUsage?

    init(runtime: ThreadRuntime, model: AppModel) {
        subagent = model.panelSubagent(of: runtime.threadID)
        isDocked = subagent != nil
        if let thread = model.thread(runtime.threadID), model.settings.showsUsagePanel {
            var handsProvider: ProviderKind?
            if model.crewIsOn(thread), let pair = model.crewPair(for: thread) {
                let provider = model.crewHandsProvider(of: pair)
                handsProvider = provider == thread.provider ? nil : provider
            }
            usage = PanelUsage(provider: thread.provider, handsProvider: handsProvider)
        } else {
            usage = nil
        }
    }
}

/// Whose limits the usage panel shows: the chat's provider, and the hands' when a pair
/// sends them out on another.
struct PanelUsage: Equatable {
    let provider: ProviderKind
    let handsProvider: ProviderKind?
}

/// Everything the chat's floating panels need in one place, worked out once per body:
/// which panels there are, which are docked where, the room they take, and their spots.
@MainActor
private struct PanelScene {
    let layout: SubagentPanelLayout
    /// The helper this thread spawned, if it still has its panel open. It sits in a corner
    /// and takes room from that side of the chat column, so the conversation and the box
    /// centre in the rest and sit beside it.
    let subagent: ChatThread?
    let isDocked: Bool
    /// A panel's place: its corner, how many panels sit under it in that corner's
    /// stack, and how much further from the edge it sits when a usage panel taller
    /// than one slot is under it.
    struct PanelSlot {
        let corner: PanelDockCorner
        let below: Int
        let lift: CGFloat
    }
    let slots: [FloatingPanelID: PanelSlot]
    let slotsOnSide: [PanelDockSide: Int]

    /// The one side every panel must take when the pane cannot hold both (see
    /// `SubagentPanelLayout.fitsBothSides`): the side the other docked panels are on,
    /// the helper panel's own when it is among them. Nil when both sides fit or no other
    /// panel is docked, so the panel being placed is free.
    func lockedSide(excluding id: FloatingPanelID) -> PanelDockSide? {
        guard !layout.fitsBothSides else { return nil }
        let others = slots.filter { $0.key != id }
        guard !others.isEmpty else { return nil }
        if let helper = others[.subagent] { return helper.corner.side }
        let leading = others.values.count { $0.corner.side == .leading }
        let trailing = others.count - leading
        return leading >= trailing ? .leading : .trailing
    }
    /// The usage panel's own height: what it was dragged to, else its rows, capped at the
    /// room left in its corner under the panels stacked below it (never the hands' compact
    /// height); nil without a usage panel.
    let usageHeight: CGFloat?
    /// The usage panel's providers, when the chat shows one (see `PanelMembers`).
    let usage: PanelUsage?
    let isMeasured: Bool
    let showsUsage: Bool
    /// The sides with a panel docked on them, in a fixed order, as the key for the slides
    /// that make room: it changes with a dock, never with a measurement.
    let dockedSides: [PanelDockSide]
    let reserve: PanelReserve
    let docks: PanelDocks

    /// `preferredSize` and `usageHeightOverride` stand in for the stored sizes while a
    /// grip is held (see `PanelResizeState.liveSize`); nil reads Settings.
    init(members: PanelMembers, runtime: ThreadRuntime, model: AppModel, paneSize: CGSize, composerAreaHeight: CGFloat, usageContentHeight: CGFloat?, preferredSize: CGSize? = nil, usageHeightOverride: CGFloat? = nil) {
        let geometry = Self.geometry(members: members, runtime: runtime, model: model, paneSize: paneSize, composerAreaHeight: composerAreaHeight, usageContentHeight: usageContentHeight, preferredSize: preferredSize, usageHeightOverride: usageHeightOverride)
        subagent = members.subagent
        isDocked = members.isDocked
        slots = geometry.slots
        slotsOnSide = geometry.slotsOnSide
        usageHeight = geometry.usageHeight
        usage = members.usage
        isMeasured = geometry.isMeasured
        showsUsage = geometry.showsUsage
        dockedSides = geometry.dockedSides
        layout = geometry.layout
        reserve = geometry.reserve
        docks = geometry.docks
    }

    /// The geometry alone, from the members and the measured pane: what a resize frame
    /// recomputes, with no model reads of its own.
    struct Geometry {
        let layout: SubagentPanelLayout
        let slots: [FloatingPanelID: PanelSlot]
        let slotsOnSide: [PanelDockSide: Int]
        let usageHeight: CGFloat?
        let isMeasured: Bool
        let showsUsage: Bool
        let dockedSides: [PanelDockSide]
        let reserve: PanelReserve
        let docks: PanelDocks
    }

    /// The smallest usage panel: its strip and one row.
    static let usageMinimumHeight = FloatingPanelMetrics.stripHeight + 44

    /// The panels' geometry: what is docked where, the room they take, and their spots.
    static func geometry(members: PanelMembers, runtime: ThreadRuntime, model: AppModel, paneSize: CGSize, composerAreaHeight: CGFloat, usageContentHeight: CGFloat?, preferredSize: CGSize? = nil, usageHeightOverride: CGFloat? = nil) -> Geometry {
        let isMeasured = paneSize != .zero
        let preferred = preferredSize ?? model.settings.panelSize
        let showsUsage = members.usage != nil && isMeasured
        let helperShowsProgress = members.subagent?.isCrewHand ?? true
        // A helper panel showing a hand's progress rather than its steps needs far less
        // height: the compact one.
        let compact = !model.settings.crewShowsHandDetails && helperShowsProgress && members.subagent != nil
        // One panel at its natural height is a slot; the usage panel is measured against
        // it, since it has a height of its own and may take more than one.
        let dockRight = model.settings.panelsDockRight
        let single = SubagentPanelLayout(pane: paneSize, composerAreaHeight: composerAreaHeight, stackDepth: 1, preferred: preferred, compact: compact, fillsColumn: dockRight)
        let slotHeight = single.panelHeight
        let gap = SubagentPanelLayout.gap
        var placed: [(id: FloatingPanelID, corner: PanelDockCorner)] = []
        if members.isDocked { placed.append((.subagent, runtime.subagentPanelDock)) }
        if showsUsage { placed.append((.usage, model.settings.usagePanelDock)) }
        // Docked on the right (the setting): every panel, whatever corner it was dragged to,
        // stacks down the trailing side under the chrome row. A drag still reorders within
        // the column (see `reorder`) but never changes side, and nothing floats.
        if dockRight { placed = placed.map { ($0.id, PanelDockCorner.topTrailing) } }
        let usageCorner = dockRight ? PanelDockCorner.topTrailing : model.settings.usagePanelDock
        var usageHeight: CGFloat?
        if showsUsage {
            let under = Self.order(placed, by: runtime.panelStackOrder)[.usage]?.below ?? 0
            let room = single.verticalRoom - CGFloat(under) * (slotHeight + gap)
            let wanted = usageHeightOverride ?? model.settings.usagePanelHeight ?? usageContentHeight ?? slotHeight
            usageHeight = min(max(Self.usageMinimumHeight, room), max(Self.usageMinimumHeight, wanted))
        }
        // The room the usage panel takes past its slot, which a panel stacked beyond it
        // steps over.
        let usageLift = max(0, (usageHeight ?? 0) - slotHeight)
        let layoutFitsBothSides = single.fitsBothSides
        // How many panels a side can hold at the smallest they shrink to, three at most:
        // the slots a panel can be dropped in. Counted at the ideal height a tall pane
        // gave one slot a side, and a second panel could only ever take the other corner.
        let smallest = compact ? SubagentPanelLayout.compactHeight : SubagentPanelLayout.minHeight
        let natural = min(3, max(1, Int((single.verticalRoom + gap) / (smallest + gap))))
        // Too narrow for both sides: every panel goes to the side the team panel is on,
        // else the fuller side, each keeping its top or bottom.
        if !layoutFitsBothSides {
            let sides = Set(placed.map(\.corner.side))
            if sides.count > 1 {
                let home: PanelDockSide = placed.first { $0.id == .subagent }?.corner.side
                    ?? (placed.count { $0.corner.side == .leading } >= placed.count { $0.corner.side == .trailing } ? .leading : .trailing)
                placed = placed.map { ($0.id, PanelDockCorner.make(side: home, isTop: $0.corner.isTop)) }
            }
        }
        var slotsByID: [FloatingPanelID: PanelSlot] = [:]
        let ordered = Self.order(placed, by: runtime.panelStackOrder)
        let usageBelow = ordered[.usage]?.below
        for (id, entry) in ordered {
            let lift: CGFloat
            if let usageBelow, entry.corner == usageCorner, showsUsage, id != .usage, entry.below > usageBelow {
                lift = usageLift
            } else {
                lift = 0
            }
            slotsByID[id] = PanelSlot(corner: entry.corner, below: entry.below, lift: lift)
        }
        let corners = placed.map(\.corner)
        let sides = Set(corners.map(\.side))
        let dockedSides = [PanelDockSide.leading, .trailing].filter(sides.contains)
        var slotsOnSide: [PanelDockSide: Int] = [:]
        for side in [PanelDockSide.leading, PanelDockSide.trailing] {
            slotsOnSide[side] = max(natural, placed.count { $0.corner.side == side })
        }
        // The height the panels share comes from how tightly a side's slots are taken:
        // slots are spread evenly between the top and bottom spots, so the closest two
        // occupied slots set the tallest panel that leaves them clear. Neighbours share
        // the room by the slot count; panels a slot apart keep half each; one alone keeps
        // its full height. The larger of the two sides' depths, since one layout serves both.
        var stackDepth = 1
        for side in [PanelDockSide.leading, PanelDockSide.trailing] {
            let n = slotsOnSide[side] ?? 1
            let indices = slotsByID.values
                .filter { $0.corner.side == side }
                .map { $0.corner.isTop ? $0.below : n - 1 - $0.below }
                .sorted()
            guard indices.count > 1 else { continue }
            let closest = zip(indices, indices.dropFirst()).map { $1 - $0 }.min() ?? 1
            let depth = Int((Double(n - 1) / Double(max(1, closest))).rounded(.up)) + 1
            stackDepth = max(stackDepth, depth)
        }
        let layout = SubagentPanelLayout(pane: paneSize, composerAreaHeight: composerAreaHeight, stackDepth: stackDepth, preferred: preferred, compact: compact, fillsColumn: dockRight)
        let reserve = PanelReserve(
            leading: sides.contains(.leading) ? layout.composerReserve : 0,
            trailing: sides.contains(.trailing) ? layout.composerReserve : 0
        )
        return Geometry(
            layout: layout,
            slots: slotsByID,
            slotsOnSide: slotsOnSide,
            usageHeight: usageHeight,
            isMeasured: isMeasured,
            showsUsage: showsUsage,
            dockedSides: dockedSides,
            reserve: reserve,
            docks: PanelDocks(layout: layout, reserve: reserve)
        )
    }

    static func order(_ placed: [(id: FloatingPanelID, corner: PanelDockCorner)], by stackOrder: [FloatingPanelID]) -> [FloatingPanelID: (corner: PanelDockCorner, below: Int)] {
        var rank: [FloatingPanelID: Int] = [:]
        for id in stackOrder where rank[id] == nil {
            rank[id] = rank.count
        }
        var out: [FloatingPanelID: (corner: PanelDockCorner, below: Int)] = [:]
        let corners = Set(placed.map(\.corner))
        for corner in corners {
            let inCorner = placed.enumerated().filter { $0.element.corner == corner }
                .sorted {
                    let left = (rank[$0.element.id] ?? Int.max, $0.offset)
                    let right = (rank[$1.element.id] ?? Int.max, $1.offset)
                    return left < right
                }
                .map(\.element)
            for (below, entry) in inCorner.enumerated() {
                out[entry.id] = (entry.corner, below)
            }
        }
        return out
    }

    private static func flippedVertically(_ corner: PanelDockCorner) -> PanelDockCorner {
        switch corner {
        case .topLeading: .bottomLeading
        case .bottomLeading: .topLeading
        case .topTrailing: .bottomTrailing
        case .bottomTrailing: .topTrailing
        }
    }
}

/// The "jump to latest" button floating above the chat box while the reader is scrolled up.
struct JumpToLatestButton: View {
    let scrollState: TimelineScrollState

    var body: some View {
        if scrollState.showsJumpButton {
            ChromeCircleButton(symbol: "arrow.down", help: "Jump to latest") {
                scrollState.jumpToLatest()
            }
            .padding(.bottom, 10)
            .frame(height: 0, alignment: .bottom)
            .transition(
                .asymmetric(
                    insertion: .scale(scale: 0.6).combined(with: .opacity).combined(with: .offset(y: 10)),
                    removal: .scale(scale: 0.85).combined(with: .opacity).combined(with: .offset(y: 6))
                )
            )
        }
    }
}

/// The sticky glass controls across the top of the conversation.
private struct ChatChromeRow: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let scrollChrome: ChromeScrollModel
    let title: String
    let project: Project?
    let directory: String
    let git: GitStatusModel

    var body: some View {
        // Follows the buttons, not the sidebar's state: they leave for the sidebar only once it
        // is wide enough to hold them, and the row closes up as they go.
        let sidebarVisible = model.sidebar.holdsTrafficLights
        let bar = model.rightSidebar
        // One glass pass for the whole row. The row floats over the conversation, so its
        // capsules sample fresh content on every scrolled frame; drawn separately, each was
        // a pass of its own. The spacing is well under the gaps, so nothing morphs together.
        GlassEffectContainer(spacing: 2) {
            HStack(alignment: .center, spacing: 10) {
                // With the sidebar away, its list is one press away: the button arrives with the
                // window buttons and leaves with them. Not when the list floats as the only
                // sidebar (see `AppSettings.sidebarOnlyFloats`): there is nothing to open.
                if !sidebarVisible, !(model.settings.sidebarFloats && model.settings.sidebarOnlyFloats) {
                    ThreadsButton()
                        .transition(.softAppear)
                }
                ChromeCircleButton(symbol: "square.and.pencil", help: "New thread" + ShortcutStore.hint(for: .newThread)) {
                    model.newThread(in: project)
                }
                if let thread = model.thread(runtime.threadID) {
                    // The team mark, once Crew is on in Settings: in every chat, active where
                    // the chat's own switch is on too. A hand leads no team of its own.
                    if model.settings.crewEnabled, !thread.isHelper {
                        CrewButton(thread: thread, runtime: runtime)
                            .transition(.softAppear)
                    }
                    PermissionMenu(thread: thread)
                }
                if git.isRepository {
                    BranchMenu(runtime: runtime, directory: directory, git: git)
                }
                ChatChromeTitle(runtime: runtime, title: title)
                    // The thread's title cuts on a switch; it must never crossfade.
                    // Animated, the old and new titles ghost over each other
                    // mid-flight. Scroll progress still drives it per frame.
                    .animation(nil, value: title)
                HStack(spacing: 8) {
                    // Scripts and git actions; the "open in another app" menu is gone, the
                    // terminal covers it for this build.
                    if project != nil || git.isRepository {
                        ChromeCapsule {
                            if let project {
                                ScriptsMenu(project: project, runtime: runtime)
                            }
                            if git.isRepository {
                                if project != nil { ChromeDivider() }
                                GitActionsMenu(runtime: runtime, directory: directory, git: git)
                            }
                        }
                    }
                    // The terminal drawer's switch, and — only while the right sidebar is
                    // closed — a switch for each of its pages. Open, the column wears its
                    // own tabs and these would say the same thing twice.
                    ChromeCapsule {
                        ChromeIconButton(symbol: "terminal", isActive: runtime.isTerminalVisible, help: "Terminal" + ShortcutStore.hint(for: .toggleTerminal)) {
                            runtime.isTerminalVisible.toggle()
                        }
                        if !bar.isVisible {
                            ChromeDivider()
                            ChromeIconButton(symbol: "person.2", asset: "crew-mark", help: "Crew" + ShortcutStore.hint(for: .toggleCrewPanel)) {
                                bar.open(.crew)
                            }
                            ChromeDivider()
                            ChromeIconButton(symbol: "plusminus", help: "Changes" + ShortcutStore.hint(for: .toggleChanges)) {
                                runtime.clearDiffFocus()
                                bar.open(.changes)
                            }
                            ChromeDivider()
                            ChromeIconButton(symbol: "checklist", help: "Tasks") {
                                bar.open(.task)
                            }
                        }
                    }
                }
            }
        }
        // With the sidebar hidden the native window buttons sit over this row, so it steps clear of them.
        .padding(.leading, sidebarVisible ? 0 : Chrome.trafficLightsWidth + Chrome.trafficLightClearance)
        .padding(.horizontal, Chrome.chromeHorizontalPadding)
        .padding(.top, Chrome.chromeTopPadding)
        .animation(Chrome.panelSlide, value: sidebarVisible)
    }
}

/// The thread title in the chrome row, next to the branch picker. A real Button so the
/// extended title bar (which drags the window from this row's gaps) never swallows the
/// click: buttons take their presses while the gaps around them still move the window.
/// Always visible; the scroll-faded compact title it replaces stayed invisible and
/// untappable at rest.
private struct ChatChromeTitle: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let title: String

    @State private var isPresented = false
    @State private var isRenaming = false
    @State private var renameText = ""

    var body: some View {
        Button {
            renameText = title
            isPresented.toggle()
        } label: {
            Text(verbatim: title.isEmpty ? "New thread" : title)
                .zoomedFont(size: 15, weight: .semibold)
                .foregroundStyle(title.isEmpty ? Chrome.secondaryText : Chrome.primaryText)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.vertical, 6)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help("Thread title — click to copy or rename")
        .accessibilityLabel(Text("Thread title"))
        .accessibilityAddTraits(.isButton)
        // The popover hangs off the title's own width, so it opens under the words;
        // the row's leftover width (below) is not the button's, or a short title
        // opened its menu far off to the right, over the middle of the chat.
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu {
                PopoverSectionHeader("Thread title")
                PopoverItem("Copy title", symbol: "doc.on.doc") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(title, forType: .string)
                }
                PopoverItem("Rename…", symbol: "pencil") {
                    renameText = title
                    isRenaming = true
                }
            }
        }
        .alert("Rename thread", isPresented: $isRenaming) {
            TextField("Title", text: $renameText)
            Button("Rename") {
                model.rename(runtime.threadID, to: renameText)
            }
            Button("Cancel", role: .cancel) {}
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The sidebar's list in a popover, for a hidden sidebar: the same search, the same rows,
/// the same footer, at the sidebar's own width. Picking a thread closes it.
private struct ThreadsButton: View {
    @Environment(AppModel.self) private var model
    @State private var isPresented = false

    var body: some View {
        // With the list floating over the chat, the button brings the column back instead;
        // the floating panel is the list while the column is away.
        let floats = model.settings.sidebarFloats
        ChromeCircleButton(symbol: "list.bullet", help: floats ? "Show the sidebar" : "Threads") {
            if floats { model.sidebar.toggle() } else { isPresented.toggle() }
        }
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            SidebarView(inPopover: true, dismiss: { isPresented = false })
                .frame(width: model.sidebar.width, height: 560)
                .presentedChrome()
        }
    }
}

@MainActor
@Observable
final class GitStatusModel {
    private(set) var isRepository = false
    private(set) var status: GitStatus?
    private(set) var branches: [GitBranch] = []
    private(set) var remoteURL: URL?
    private(set) var activity: String?

    private struct Snapshot {
        var isRepository = false
        var status: GitStatus?
        var branches: [GitBranch] = []
        var remoteURL: URL?
        var fetchedAt = Date.now
    }

    /// The last read per directory, shared by every chat. Switching threads shows the branch at
    /// once and only runs git again when the read is stale, a turn changed files, or forced.
    private static var snapshots: [String: Snapshot] = [:]
    private static let freshness: TimeInterval = 30

    func refresh(_ directory: String, force: Bool = true) async {
        if let cached = Self.snapshots[directory] {
            apply(cached)
            if !force, Date.now.timeIntervalSince(cached.fetchedAt) < Self.freshness { return }
        }
        let git = Git(directory)
        var snapshot = Snapshot(isRepository: await git.isRepository())
        if snapshot.isRepository {
            async let status = git.status()
            async let branches = git.branches()
            async let remoteURL = git.remoteWebURL()
            snapshot.status = await status
            snapshot.branches = await branches
            snapshot.remoteURL = await remoteURL
        }
        snapshot.fetchedAt = .now
        Self.snapshots[directory] = snapshot
        apply(snapshot)
    }

    private func apply(_ snapshot: Snapshot) {
        isRepository = snapshot.isRepository
        status = snapshot.status
        branches = snapshot.branches
        remoteURL = snapshot.remoteURL
    }

    func perform(_ activity: String, _ work: () async throws -> Void) async -> Error? {
        self.activity = activity
        defer { self.activity = nil }
        do {
            try await work()
            return nil
        } catch {
            return error
        }
    }
}

/// The thread's permission level: how much the agent may do without asking.
private struct PermissionMenu: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread

    var body: some View {
        ChromeCircleMenu(symbol: thread.runtimeMode.symbol, help: "\(thread.runtimeMode.title) · \(thread.runtimeMode.summary)") {
            PopoverSectionHeader("Permissions")
            ForEach(RuntimeMode.allCases) { mode in
                PopoverItem(mode.title, symbol: mode.symbol, isChecked: thread.runtimeMode == mode) {
                    model.updateThread(thread.id) { $0.runtimeMode = mode }
                }
            }
        }
    }
}

private struct BranchMenu: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let directory: String
    let git: GitStatusModel

    @State private var isNamingBranch = false
    @State private var branchName = ""

    var body: some View {
        ChromeTextMenu(
            symbol: "arrow.triangle.branch",
            title: git.activity ?? git.status?.branch ?? "Detached",
            help: "Branch"
        ) {
            PopoverSectionHeader("Switch branch")
            ForEach(git.branches.prefix(25)) { branch in
                PopoverItem(
                    branch.name,
                    isChecked: branch.isCurrent,
                    isEnabled: !branch.isCurrent && !runtime.isRunning
                ) {
                    switchBranch(branch.name)
                }
            }
            PopoverDivider()
            PopoverItem("New branch…", symbol: "plus") {
                branchName = ""
                isNamingBranch = true
            }
            if runtime.thread?.worktreePath == nil, runtime.turns.isEmpty {
                PopoverItem("Use a new worktree for this thread", symbol: "square.stack.3d.up") {
                    Task { await model.createWorktree(for: runtime.threadID) }
                }
            }
        }
        .alert("New branch", isPresented: $isNamingBranch) {
            TextField("Branch name", text: $branchName)
            Button("Create") { createBranch() }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func switchBranch(_ name: String) {
        Task {
            let repository = Git(directory)
            if let error = await git.perform("Switching", { try await repository.switchBranch(name) }) {
                model.alert = AppAlert(title: "Could not switch branch", message: error.localizedDescription)
            }
            await git.refresh(directory)
        }
    }

    private func createBranch() {
        let name = branchName.trimmingCharacters(in: .whitespacesAndNewlines).replacingOccurrences(of: " ", with: "-")
        guard !name.isEmpty else { return }
        Task {
            let repository = Git(directory)
            if let error = await git.perform("Creating", { try await repository.createBranch(name) }) {
                model.alert = AppAlert(title: "Could not create branch", message: error.localizedDescription)
            }
            await git.refresh(directory)
        }
    }
}

private struct ScriptsMenu: View {
    @Environment(AppModel.self) private var model
    @Environment(\.chatColumnIsShown) private var isColumnShown
    let project: Project
    let runtime: ThreadRuntime

    @State private var editing: ScriptsEditorTarget?

    var body: some View {
        // Listed and numbered the way the Workflow menu lists them: the global scripts first,
        // so each keeps its ⌥⌘ number in every project (see `AppModel.scripts(for:)`).
        let global = model.globalScripts
        // The project whose lists are looked at once this column is on screen; none while it
        // is built behind the one shown.
        let shownProjectID: UUID? = isColumnShown ? project.id : nil
        ChromeMenuButton(symbol: "play", help: "Run a script") {
            if !global.isEmpty {
                PopoverSectionHeader("Global")
                ForEach(Array(global.enumerated()), id: \.element.id) { index, script in
                    scriptItem(script, number: index + 1)
                }
            }
            if !project.scripts.isEmpty {
                PopoverSectionHeader(project.name)
                ForEach(Array(project.scripts.enumerated()), id: \.element.id) { index, script in
                    scriptItem(script, number: global.count + index + 1)
                }
            }
            if !global.isEmpty || !project.scripts.isEmpty {
                PopoverDivider()
            }
            PopoverItem(project.scripts.isEmpty ? "Add a script…" : "Edit scripts…", symbol: project.scripts.isEmpty ? "plus" : "pencil") {
                editing = .project(project.id)
            }
            // The menu opening is a look at the lists: a file edited since shows as it is now.
            .onAppear { model.refreshScripts(for: runtime.threadID) }
            PopoverItem("Edit global scripts…", symbol: "globe") {
                editing = .global
            }
        }
        .sheet(item: $editing) { target in
            ScriptsEditor(target: target)
                .presentedChrome()
                // The panel is a sheet, so the Settings window's own backdrop is not under it:
                // it paints the same glass at the same setting, clipped to the same curve, so
                // the two panels read as one window's worth of chrome.
                .presentationCornerRadius(Chrome.windowCornerRadius)
                .presentationBackground { WindowBackdrop(opacity: model.settings.backdropOpacity) }
        }
        // The thread coming on screen, or moving to another project, is a look at its lists
        // too. A column built behind the one shown waits until it is shown itself.
        .task(id: shownProjectID) {
            guard shownProjectID != nil else { return }
            model.refreshScripts(for: runtime.threadID)
        }
        // The Workflow menu's Edit items, taken up by the chat on screen.
        .onChange(of: model.scriptsEditorRequest) { _, request in
            guard isColumnShown, let request else { return }
            model.scriptsEditorRequest = nil
            editing = request.target
        }
    }

    /// A script's row, with the ⌥⌘ chord that runs it from anywhere in the window.
    private func scriptItem(_ script: ProjectScript, number: Int) -> some View {
        PopoverItem(script.name, detail: ScriptShortcuts.chord(forNumber: number)?.description, symbol: script.symbol) {
            model.runScript(script, threadID: runtime.threadID)
        }
    }
}

private struct GitActionsMenu: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let directory: String
    let git: GitStatusModel

    @State private var isCommitting = false

    var body: some View {
        let changed = git.status?.changedFiles ?? 0
        ChromeMenuButton(symbol: "arrow.triangle.pull", help: changed == 0 ? "Commit, push and open pull requests" : "\(changed) changed files") {
            PopoverNote(changed == 1 ? "1 changed file" : "\(changed) changed files")
            PopoverDivider()
            PopoverItem("Commit…", symbol: "checkmark.circle", isEnabled: changed > 0) { isCommitting = true }
            PopoverItem("Push", symbol: "arrow.up.circle") { push() }
            PopoverItem("Create pull request", symbol: "arrow.triangle.pull") { openPullRequest() }
            if let url = git.remoteURL {
                PopoverDivider()
                PopoverItem("Open repository", symbol: "safari") { NSWorkspace.shared.open(url) }
            }
        }
        .sheet(isPresented: $isCommitting) {
            CommitSheet(runtime: runtime, directory: directory, git: git)
                .presentedChrome()
        }
    }

    private func push() {
        Task {
            let repository = Git(directory)
            if let error = await git.perform("Pushing", { try await repository.push() }) {
                model.alert = AppAlert(title: "Push failed", message: error.localizedDescription)
            }
            await git.refresh(directory)
        }
    }

    private func openPullRequest() {
        Task {
            let repository = Git(directory)
            var url: URL?
            let error = await git.perform("Opening pull request") {
                try await repository.push()
                let subject = (try? await repository.output(["log", "-1", "--pretty=%s"]))?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                let body = (try? await repository.output(["log", "-1", "--pretty=%b"]))?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                url = try await repository.createPullRequest(
                    title: subject.isEmpty ? "Update" : subject,
                    body: body.isEmpty ? "Opened from Bottega." : body
                )
            }
            if let error {
                model.alert = AppAlert(title: "Could not open a pull request", message: error.localizedDescription)
            } else if let url {
                NSWorkspace.shared.open(url)
            }
            await git.refresh(directory)
        }
    }
}

private struct CommitSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let runtime: ThreadRuntime
    let directory: String
    let git: GitStatusModel

    @State private var message = ""
    @State private var summary = ""
    @State private var isGenerating = false
    @State private var isCommitting = false
    @State private var pushesAfterCommit = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Commit changes")
                .zoomedFont(size: 17, weight: .semibold)
            if !summary.isEmpty {
                ScrollView {
                    Text(summary)
                        .zoomedFont(size: 11, design: .monospaced)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 110)
                .padding(10)
                .background(Chrome.overlay(0.05), in: .rect(cornerRadius: Chrome.blockCornerRadius, style: .continuous))
            }
            ZStack(alignment: .topLeading) {
                TextEditor(text: $message)
                    .zoomedFont(size: 13)
                    .scrollContentBackground(.hidden)
                    .padding(8)
                if message.isEmpty {
                    Text(isGenerating ? "Writing a message…" : "Commit message")
                        // The editor's size, so the placeholder sits where its text will.
                        .zoomedFont(.body)
                        .foregroundStyle(Chrome.secondaryText)
                        .padding(.horizontal, 13)
                        .padding(.vertical, 8)
                        .allowsHitTesting(false)
                }
            }
            .frame(minHeight: 130)
            .background(Chrome.overlay(0.05), in: .rect(cornerRadius: Chrome.blockCornerRadius, style: .continuous))
            HStack(spacing: 12) {
                // Nothing writes the message with text generation off or its provider signed
                // out, so the button says where to turn it on instead of doing nothing.
                let canGenerate = model.textEngine(preferring: runtime.thread?.provider) != nil
                Button {
                    Task { await generate() }
                } label: {
                    Label {
                        Text("Write for me")
                    } icon: {
                        if isGenerating {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Image(systemName: "sparkles")
                        }
                    }
                }
                .buttonStyle(.glass)
                .disabled(isGenerating || !canGenerate)
                .help(canGenerate ? "Write a commit message from these changes" : "Turn on text generation in Settings > Source control")
                Toggle("Push after committing", isOn: $pushesAfterCommit)
                    .toggleStyle(.checkbox)
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .buttonStyle(.glass)
                Button("Commit") { Task { await commit() } }
                    .buttonStyle(.glassProminent)
                    .disabled(message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isCommitting)
            }
        }
        .padding(20)
        .frame(width: 560)
        .task {
            let changes = await Git(directory).workingChanges()
            summary = changes.summary
            if message.isEmpty { await generate(changes: changes) }
        }
    }

    private func generate(changes: (summary: String, patch: String)? = nil) async {
        guard let engine = model.textEngine(preferring: runtime.thread?.provider) else { return }
        isGenerating = true
        defer { isGenerating = false }
        let resolved: (summary: String, patch: String)
        if let changes {
            resolved = changes
        } else {
            resolved = await Git(directory).workingChanges()
        }
        if let text = await TextGeneration.commitMessage(
            summary: resolved.summary,
            patch: resolved.patch,
            instructions: model.settings.commitInstructions,
            engine: engine,
            directory: URL(fileURLWithPath: directory)
        ) {
            message = text
        }
    }

    private func commit() async {
        isCommitting = true
        defer { isCommitting = false }
        let repository = Git(directory)
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        let pushes = pushesAfterCommit
        let error = await git.perform("Committing") {
            try await repository.commitAll(message: text)
            if pushes { try await repository.push() }
        }
        await git.refresh(directory)
        if let error {
            model.alert = AppAlert(title: "Commit failed", message: error.localizedDescription)
        } else {
            dismiss()
        }
    }
}

/// Edits one list of scripts. A project's: the scripts added in the app, with the ones its
/// `bottega.json` lists shown as the file has them, since the file is where those
/// change. Or the global list, which is its file, written back to it on Save.
private struct ScriptsEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let target: ScriptsEditorTarget

    @State private var scripts: [ProjectScript] = []
    /// The list's file, when it has one on disk.
    @State private var file: URL?
    /// The global file as it stood when the editor opened: a save over a file changed on
    /// disk since then is refused, rather than writing over the change.
    @State private var loadedStamp: ScriptFileStamp?
    @State private var saveError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(target == .global ? "Global scripts" : "Project scripts")
                .zoomedFont(size: 17, weight: .semibold)
            Text(note)
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            ScrollView {
                VStack(spacing: 10) {
                    ForEach($scripts) { $script in
                        if isReadOnly(script) {
                            FileScriptCard(script: script, fileName: file?.lastPathComponent ?? ProjectFile.names[0])
                        } else {
                            ScriptCard(script: $script) {
                                scripts.removeAll { $0.id == script.id }
                            }
                        }
                    }
                }
            }
            .frame(minHeight: 220)
            if let saveError {
                Text(verbatim: saveError)
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Button {
                    scripts.append(ProjectScript(name: "New script", command: ""))
                } label: {
                    Label("Add script", systemImage: "plus")
                }
                .buttonStyle(.glass)
                if let file {
                    Button("Open \(file.lastPathComponent)") { openFile(file) }
                        .buttonStyle(.glass)
                        .help("Opens the file in the app that edits JSON on this Mac")
                }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .buttonStyle(.glass)
                let incomplete = scripts.contains { !isReadOnly($0) && Self.isIncomplete($0) }
                Button("Save") { save() }
                    .buttonStyle(.glassProminent)
                    .disabled(incomplete)
                    .help(incomplete ? "Every script needs a command." : "Save these scripts")
            }
        }
        .padding(20)
        .frame(width: 580, height: 520)
        .onAppear { load() }
    }

    private var note: String {
        switch target {
        case .project:
            "Scripts run in a terminal in the thread's folder, which $BOTTEGA_PROJECT_ROOT names too. The ones the project's bottega.json lists (or a droppy-code.json left by the upstream app) follow that file as it changes, so they are edited there."
        case .global:
            "Every thread offers these in every project, ahead of its project's own scripts; ⌥⌘1 to ⌥⌘9 run the first nine. They are kept in bottega.json in Bottega's Application Support folder, which can be edited by hand too."
        }
    }

    /// A project's script its file lists: the file is where it changes. Every global script
    /// is the file's too, but this editor is where that file is written.
    private func isReadOnly(_ script: ProjectScript) -> Bool {
        guard case .project = target else { return false }
        return script.isFromFile
    }

    /// The list as it stands, its file read again first where it changed.
    private func load() {
        switch target {
        case .project(let id):
            model.refreshProjectScripts(id)
            scripts = model.project(id)?.scripts ?? []
            file = model.project(id).flatMap { ProjectFile.file(in: $0.url) }
        case .global:
            model.refreshGlobalScripts()
            let present = ProjectFile.globalFileIfPresent
            scripts = model.globalScripts
            file = present
            loadedStamp = ScriptFileStamp(file: present)
        }
    }

    private func save() {
        // A name this Mac has no symbol for saves as the default mark, which the row showed.
        let edited = scripts.map { script -> ProjectScript in
            var script = script
            script.symbol = ProjectScript.resolvedSymbol(script.symbol)
            return script
        }
        switch target {
        case .project(let id):
            // The file's scripts stay as the file has them now; the rows edited here are the
            // ones added in the app. A row the file has since taken over (a read while the
            // editor was open) is the file's now, and is kept once, as the file has it.
            model.updateProject(id) { project in
                let fromFile = project.scripts.filter(\.isFromFile)
                let taken = Set(fromFile.map(\.id))
                project.scripts = fromFile + edited.filter { !$0.isFromFile && !taken.contains($0.id) }
            }
            dismiss()
        case .global:
            guard ScriptFileStamp(file: ProjectFile.globalFileIfPresent) == loadedStamp else {
                saveError = "\(ProjectFile.names[0]) changed on disk since this editor opened. Close the editor and open it again to pick the change up."
                return
            }
            do {
                try model.saveGlobalScripts(edited)
                dismiss()
            } catch {
                saveError = error.localizedDescription
            }
        }
    }

    /// Opens the list's file. The global editor closes as it does, so the file stays the one
    /// place its list is being changed.
    private func openFile(_ file: URL) {
        NSWorkspace.shared.open(file)
        if target == .global { dismiss() }
    }

    /// A script with nothing to run. Saving is held until every row has a command.
    static func isIncomplete(_ script: ProjectScript) -> Bool {
        script.command.trimmingCharacters(in: .whitespaces).isEmpty
    }
}

/// A script the editor edits: one added in the app, or a global one.
private struct ScriptCard: View {
    @Binding var script: ProjectScript
    let remove: @MainActor () -> Void
    /// The window's text size: the symbol's column grows with the type beside it.
    @Environment(\.chatZoom) private var zoom

    var body: some View {
        let typed = script.symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        let shown = ProjectScript.resolvedSymbol(typed)
        let symbolIsKnown = typed.isEmpty || shown == typed
        ChromeCard {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    TextField("Name", text: $script.name)
                        .textFieldStyle(.roundedBorder)
                    Button(role: .destructive, action: remove) {
                        Image(systemName: "trash")
                    }
                    .buttonStyle(.borderless)
                }
                TextField("Command", text: $script.command)
                    .textFieldStyle(.roundedBorder)
                    .zoomedFont(size: 12, design: .monospaced)
                    // A script with no command used to be dropped on save
                    // without a word; the row says what it is missing.
                    .overlay {
                        if ScriptsEditor.isIncomplete(script) {
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .strokeBorder(Chrome.danger.opacity(0.65), lineWidth: 1)
                        }
                    }
                HStack(spacing: 8) {
                    // The mark as the menus will draw it: a name this Mac has no symbol for
                    // shows the default one, and saves as it.
                    Image(systemName: shown)
                        .zoomedFont(size: 12)
                        .frame(width: ChatZoom.scaled(16, by: zoom))
                    TextField("SF Symbol", text: $script.symbol)
                        .textFieldStyle(.roundedBorder)
                        .zoomedFont(size: 12, design: .monospaced)
                        .frame(maxWidth: 200)
                        .overlay {
                            if !symbolIsKnown {
                                RoundedRectangle(cornerRadius: 6, style: .continuous)
                                    .strokeBorder(Chrome.warning.opacity(0.65), lineWidth: 1)
                            }
                        }
                        .help(symbolIsKnown ? "The script's icon: any SF Symbol's name, such as hammer.fill" : "No SF Symbol has this name on this Mac, so the script shows the play mark")
                    Spacer(minLength: 8)
                    Toggle("Run when a worktree is created", isOn: $script.runOnWorktreeCreate)
                        .toggleStyle(.checkbox)
                }
            }
            .padding(14)
        }
    }
}

/// A script its project's file lists, as the editor shows it: read-only, since the file is
/// where it changes.
private struct FileScriptCard: View {
    let script: ProjectScript
    let fileName: String
    /// The window's text size: the symbol's column grows with the type beside it.
    @Environment(\.chatZoom) private var zoom

    var body: some View {
        ChromeCard {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 8) {
                    Image(systemName: script.symbol)
                        .zoomedFont(size: 12)
                        .frame(width: ChatZoom.scaled(16, by: zoom))
                    Text(verbatim: script.name)
                        .zoomedFont(size: 13, weight: .medium)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    Text(verbatim: "From \(fileName)")
                        .zoomedFont(size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                }
                Text(verbatim: script.command)
                    .zoomedFont(size: 12, design: .monospaced)
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(3)
                    .textSelection(.enabled)
                if script.runOnWorktreeCreate {
                    Text("Runs when a worktree is created")
                        .zoomedFont(size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
        }
    }
}
