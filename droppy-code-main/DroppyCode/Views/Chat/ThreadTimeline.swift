import AppKit
import SwiftUI

struct ThreadTimeline: View, Equatable {
    @Environment(AppModel.self) private var model
    @Environment(\.chatZoom) private var zoom
    @Environment(WindowLiveResize.self) private var liveResize
    let runtime: ThreadRuntime
    let scrollChrome: ChromeScrollModel
    let scrollState: TimelineScrollState
    let projectName: String?
    /// The folder the thread works in, for tool rows that resolve paths. Read once by the
    /// chat and handed down as a value, so rows never look the thread up themselves.
    let workingDirectory: String?
    /// Whether the thread's provider can rewind a conversation, for the revert controls.
    let supportsRewind: Bool
    /// Full chat-column height, so the rail stays centred when the composer
    /// or queue tab grows.
    let columnHeight: CGFloat
    /// Whether the rail is shown at all; a panel docked on the left takes its edge.
    var showsMinimap = true

    /// The chat re-renders whenever its thread changes (a title, the effort, a mode); the
    /// timeline only follows when what it was handed changed.
    nonisolated static func == (lhs: ThreadTimeline, rhs: ThreadTimeline) -> Bool {
        lhs.runtime === rhs.runtime
            && lhs.scrollChrome === rhs.scrollChrome
            && lhs.scrollState === rhs.scrollState
            && lhs.projectName == rhs.projectName
            && lhs.workingDirectory == rhs.workingDirectory
            && lhs.supportsRewind == rhs.supportsRewind
            && lhs.columnHeight == rhs.columnHeight
            && lhs.showsMinimap == rhs.showsMinimap
    }

    /// Everything the scroll view and the rows report while the reader scrolls: which blocks
    /// are on screen and whether the timeline follows new text. It lives in an object rather
    /// than in view state on purpose. Rows write to it as they cross the viewport's edges and
    /// the scroll view writes to it as content grows, and only the rail reads it, so none of
    /// that ever re-runs this body. The one thing it tells the body is the rare request to
    /// check its own layout (`repairLayout`).
    @State private var tracking = TimelineScrollTracking()
    /// The scroll position stays view state, never a property of an observable object. A
    /// `scrollTo` request is consumed by the scroll view writing the resolved position back
    /// through the binding; on an observable, that write's `willSet` re-ran this body while
    /// the property still held the pending request, so the scroll view enqueued it again,
    /// without end. `@State` applies the write first and re-renders after.
    @State private var position = ScrollPosition(edge: .bottom)
    @State private var viewportHeight: CGFloat = 0
    /// The scroll view's width, for the stepped layout width during a live resize.
    @State private var paneWidth: CGFloat = 0
    /// While the window is being dragged the content wraps to a width that moves in
    /// steps this wide, so rows re-measure a few times over a drag instead of every
    /// frame; the exact width lands when the drag ends.
    private static let resizeStep: CGFloat = 12
    private var layoutWidth: CGFloat? {
        guard liveResize.isReshaping, paneWidth > 0 else { return nil }
        return min(820 + 40, (paneWidth / Self.resizeStep).rounded(.down) * Self.resizeStep)
    }
    /// Which edge stays put when the content's height changes. At the conversation's end
    /// it is the bottom, so streaming text and the working line grow in place. Once the
    /// reader has scrolled up it is the top: a row expanded mid-thread then pushes what
    /// follows down and stays under the pointer, instead of the whole timeline lurching
    /// up by the expansion's height to keep the far-off bottom edge fixed.
    @State private var anchorsBottomOnGrowth = true
    /// Older history is about to be loaded above the viewport; that growth keeps the
    /// bottom fixed whatever the reader is doing, so the messages in view stay put.
    @State private var historyLoadPending = false
    /// Lazy-loading window: only the newest groups are materialized, so opening a long
    /// thread and scrolling through it stays instant no matter how much history it holds.
    @State private var visibleCount = TimelineWindow.firstPaint
    /// Changes to rebuild the lazy stack from scratch: the second repair for a viewport
    /// the stack has built nothing for (see `repairLayout`).
    @State private var stackGeneration = 0
    /// The rows' way to the reveal (see `revealEnd`), made once for the timeline's life.
    @State private var revealBox = TimelineRevealBox()
    /// The blocks as last built. This body re-runs for plenty of reasons the conversation
    /// knows nothing about (the reader starting or ending a scroll, the column resizing, a
    /// setting elsewhere), and partitioning every entry into blocks again each time is work
    /// the stack throws straight away. It lives in an object, so reusing a build is not a
    /// state write.
    private var blockCache: TimelineBlockCache { TimelineBlockCache.cache(for: runtime.threadID) }

    var body: some View {
        let entries = runtime.entries
        let heads = model.hydraHeads(of: runtime.threadID)
        // Nothing here reads an entry's content: the cache keys on identities and kinds,
        // so a streaming row redraws itself and never brings this body with it.
        let blocks = blockCache.blocks(
            for: entries,
            showReasoning: model.settings.showReasoning, isRunning: runtime.isRunning,
            hydraMergeID: runtime.isHydraMerging ? (runtime.hydraMergeNoteID ?? "hydra-merging") : nil,
            workingHeads: heads.compactMap { $0.hydra?.status == .running ? $0.hydra?.index : nil }
        )
        let hydraMentionPersonas = blockCache.mentionPersonas(for: heads)
        // A history still being read off the main thread is not an empty thread: the
        // prompt for a new one would flash for the frames before it lands.
        if runtime.isLoadingHistory {
            Color.clear
        } else if blocks.isEmpty && !runtime.isRunning {
            NewThreadPrompt(threadID: runtime.threadID, projectName: projectName)
                .onAppear {
                    scrollChrome.update(travel: 0)
                    scrollState.showsJumpButton = false
                }
        } else {
            timeline(blocks, hydraMentionPersonas: hydraMentionPersonas)
        }
    }

    private func timeline(_ blocks: [DisplayBlock], hydraMentionPersonas: [HydraPersona]) -> some View {
        // Only the newest window of blocks is rendered. Older history loads on demand,
        // so the view count stays bounded even for very long threads.
        let hidden = max(0, blocks.count - visibleCount)
        let visible = hidden == 0 ? blocks : Array(blocks.suffix(visibleCount))
        // The turns a row may offer to revert. Read here, once, so a turn record changing
        // (checkpoints, diffs, anchors) re-runs this body alone; rows get a plain flag that
        // only changes when their own answer does.
        let rewindable: Set<UUID>
        if supportsRewind && !runtime.isRunning {
            rewindable = Set(runtime.turns.lazy.map(\.id))
        } else {
            rewindable = []
        }
        // The rail floats over the timeline's leading gutter instead of taking layout
        // space, so the conversation stays centered exactly like the composer.
        // Ticks centre in the full column height, so the queue tab opening never moves them.
        return ZStack(alignment: .leading) {
            timelineScroll(visible: visible, hidden: hidden, rewindable: rewindable)
            // The rail fades with the slide of the panel that takes its edge.
            ZStack {
                if showsMinimap, blocks.count(where: \.hasUserMessage) > 1 {
                    // The rail starts under the title bar: nothing of it, not its hover
                    // tracking nor its scrub gesture, may sit over the window buttons and
                    // the chrome row at the top of the column.
                    TimelineMinimapColumn(
                        blocks: blocks,
                        tracking: tracking,
                        centerHeight: max(0, columnHeight - WindowChrome.titlebarHeight),
                        onNavigate: { id, animated in jump(to: id, in: blocks, animated: animated) }
                    )
                    .equatable()
                    .frame(width: 30)
                    .frame(maxHeight: .infinity)
                    .padding(.top, WindowChrome.titlebarHeight)
                    // The hover card reaches over the conversation instead of being painted under it.
                    .transition(.opacity)
                }
            }
            // The rail fades with the slide of the panel that takes its edge. Held mid-resize:
            // a fresh slide every frame lagged the whole chat behind the window.
            .animation(liveResize.isReshaping ? nil : Chrome.panelSlide, value: showsMinimap)
        }
        .environment(\.hydraMentionPersonas, hydraMentionPersonas)
    }

    /// Jump the timeline to a minimap block. A target above the loaded
    /// window first loads history down to it, then scrolls once layout
    /// exists. Jumping away from the latest unpins the follow behavior, so
    /// streaming text never yanks the reader back down.
    private func jump(to id: String, in blocks: [DisplayBlock], animated: Bool) {
        var needsExpand = false
        if let index = blocks.firstIndex(where: { $0.id == id }) {
            let firstVisible = blocks.count - visibleCount
            if index < firstVisible {
                visibleCount = blocks.count - index
                needsExpand = true
                historyLoadPending = true
            }
            tracking.isPinnedToBottom = (id == blocks.last?.id)
            anchorsBottomOnGrowth = tracking.isPinnedToBottom || needsExpand
        }
        let scroll = { @MainActor in position.scrollTo(id: id, anchor: .top) }
        if animated {
            if needsExpand {
                DispatchQueue.main.async { withAnimation(.smooth(duration: 0.35)) { scroll() } }
            } else {
                withAnimation(.smooth(duration: 0.35)) { scroll() }
            }
        } else {
            var transaction = Transaction()
            transaction.disablesAnimations = true
            if needsExpand {
                DispatchQueue.main.async { withTransaction(transaction) { scroll() } }
            } else {
                withTransaction(transaction) { scroll() }
            }
        }
    }

    /// Brings more older blocks into the window: one page, or up to `count`. The bottom
    /// stays held while they land above the viewport, so what the reader is looking at
    /// does not move.
    private func loadEarlier(to count: Int? = nil) {
        guard !historyLoadPending else { return }
        let target = count ?? visibleCount + TimelineWindow.page
        guard target > visibleCount else { return }
        historyLoadPending = true
        anchorsBottomOnGrowth = true
        visibleCount = target
    }

    /// A scroll that lands at once, for following growth the reader is already looking at.
    private static var unanimated: Transaction {
        var transaction = Transaction()
        transaction.disablesAnimations = true
        return transaction
    }

    /// Parses every finished reply's markdown off the main thread, so rows scrolling into
    /// view for the first time never parse on the frame. Cached texts are skipped.
    private func warmMarkdown() async {
        var texts: [String] = []
        for entry in runtime.entries {
            guard case .assistant(let message) = entry.item.content, !message.isStreaming else { continue }
            texts.append(message.text)
        }
        guard !texts.isEmpty else { return }
        try? await MarkdownView.warm(texts)
    }

    /// How a block arrives and leaves. It arrives softly, like everything in the app, and
    /// leaves at once: a row that lingered while it faded kept its height in the lazy
    /// stack a beat longer. A finished turn's block gets no transition at all: it comes
    /// back into the window when older history loads, and a fade over a block that can run
    /// to thousands of points is a whole-layer composite the renderer may draw as nothing.
    /// A turn that is only starting is a prompt and the working line: it arrives like a row.
    /// A work group past a few rows is the same kind of layer, so it gets none either; a
    /// small one still fades in.
    private static func transition(for block: DisplayBlock) -> AnyTransition {
        if case .turn(_, _, _, .some(_), _) = block { return .identity }
        if case .group(.work(_, let entries, _), _, _) = block, entries.count > softAppearLimit { return .identity }
        return rowTransition
    }

    /// The most rows a work group may hold and still arrive with the soft appear.
    private static let softAppearLimit = 8

    /// A row's own arrival and departure, inside a turn's block as in the stack.
    static let rowTransition: AnyTransition = .asymmetric(
        insertion: .modifier(active: SoftAppearModifier(isVisible: false), identity: SoftAppearModifier(isVisible: true))
            .animation(.softAppear),
        removal: .identity
    )

    private func timelineScroll(visible: [DisplayBlock], hidden: Int, rewindable: Set<UUID>) -> some View {
        // What the stack renders last: the newest question badge, or the newest block.
        // Its change is something arriving at the end of the conversation.
        let lastEntryID = runtime.questions.last.map { "question-" + $0.id } ?? visible.last?.id
        // The outline the rail resolves against, kept in step with what is laid out. Rows
        // coming and going is when the stack can lose its place; a look a beat from now
        // costs nothing and catches it whether or not any row says so.
        if tracking.setOutline(visible.map { ($0.id, $0.hasUserMessage) }) {
            tracking.armBlankWatch()
            // A new block at the end under a reader held there: the stack is told where
            // the viewport is, since the block may have landed below rows it built for
            // the viewport before, with the older rows still on screen above the fold.
            if tracking.isPinnedToBottom, !tracking.isArriving { tracking.armViewportRefresh() }
        }
        // This evaluation's own state behind the rows' reveal (see `revealEnd`).
        revealBox.action = revealEnd
        return ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Spacer(minLength: 0)
                if hidden > 0 {
                    // Older history loads itself as the reader nears the top, a page at a
                    // time, so scrolling back never stops at a button; the pill is still
                    // there to tap for anyone who gets to it first.
                    Button {
                        loadEarlier()
                    } label: {
                        Label("Show \(hidden) earlier messages", systemImage: "chevron.up")
                            .font(.chat(.callout, zoom: zoom))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 7)
                            .background(.quaternary.opacity(0.5), in: Capsule(style: .continuous))
                            .contentShape(.capsule)
                    }
                    .buttonStyle(.plain)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, TimelineMetrics.rowSpacing)
                    .onScrollVisibilityChange(threshold: 0.1) { isVisible in
                        if isVisible { loadEarlier() }
                    }
                }
                LazyVStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                    ForEach(visible) { block in
                        let context = RowContext(
                            workingDirectory: workingDirectory,
                            canRewind: block.turnID.map(rewindable.contains) ?? false
                        )
                        DisplayBlockView(block: block, runtime: runtime, context: context)
                            .equatable()
                            .id(block.id)
                            // Reported when a block enters or leaves the viewport, never per
                            // frame. (A geometry observer per block made the lazy stack
                            // re-measure every child on every scrolled frame.)
                            .onScrollVisibilityChange(threshold: 0.001) { isVisible in
                                tracking.setVisible(block.id, isVisible)
                            }
                            // A row the lazy stack throws away (scrolled far off, or gone
                            // from the conversation) reports nothing more, so it leaves
                            // the on-screen set here; the set is how the timeline knows
                            // when it is showing nothing at all.
                            .onDisappear { tracking.setVisible(block.id, false) }
                            .transition(Self.transition(for: block))
                    }
                    // The agent's questions end the conversation as badges until they are answered.
                    ForEach(runtime.questions) { request in
                        QuestionBadgeRow(request: request, runtime: runtime)
                            .id("question-" + request.id)
                            .transition(.softAppear)
                    }
                }
                .id(stackGeneration)
                .modifier(ScrollFreeze(tracking: tracking))
                // The end of the conversation. While it is on screen the reader is at the latest message.
                Color.clear
                    .frame(height: 1)
                    .onScrollVisibilityChange(threshold: 0.01) { isVisible in
                        guard scrollState.showsJumpButton == isVisible else { return }
                        withAnimation(.spring(response: 0.38, dampingFraction: 0.78)) {
                            scrollState.showsJumpButton = !isVisible
                        }
                    }
            }
            // The rows open their steps against this: an expansion at the end of the
            // conversation scrolls the timeline down to show what opened, in the tap's own
            // transaction, so the glide and the opening are one motion.
            .environment(\.revealTimelineEnd, revealBox)
            // The same column as the composer, so messages line up with its edges.
            .frame(maxWidth: 820, alignment: .leading)
            .padding(.horizontal, 20)
            .frame(width: layoutWidth, alignment: .leading)
            .padding(.top, Chrome.contentTopInset)
            .padding(.bottom, 18)
            // A short conversation still fills the pane, with its messages resting at the bottom.
            .frame(maxWidth: .infinity, minHeight: viewportHeight, alignment: .top)
            // Inside the content, so the scroll view hosting it can be found for the nudge
            // that tells the lazy stack where the viewport is (`refreshViewport`).
            .background(alignment: .top) {
                ScrollViewProbe { tracking.probe = $0 }
                    .frame(width: 0, height: 0)
            }
        }
        .id(runtime.threadID)
        // Torn down mid-scroll (a thread switch, a panel closed): the freeze lifts with it,
        // so the app never goes on hearing this timeline as scrolling.
        .onDisappear { tracking.endActivity() }
        // The thread opens at its end and stays there while its rows measure in.
        .onAppear { tracking.holdEnd(for: 1.5) }
        .scrollIndicators(.never)
        .scrollPosition($position)
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(.bottom, for: .alignment)
        .defaultScrollAnchor(anchorsBottomOnGrowth ? .bottom : .top, for: .sizeChanges)
        // One value per frame, and only when it moved: `onGeometryChange` fires solely
        // on change, and the settle below runs once when the resize ends.
        .onGeometryChange(for: CGFloat.self, of: Self.visibleHeight) { viewportHeight = $0 }
        .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { paneWidth = $0 }
        // No animation inside the timeline while the window is being dragged; a spring
        // restarted every frame is what made rows lag behind the window and land somewhere else.
        .transaction { transaction in if liveResize.isReshaping { transaction.animation = nil } }
        .onChange(of: liveResize.isReshaping) { _, active in
            guard !active else { return }
            settleAfterResize()
        }
        .onScrollPhaseChange { _, phase in
            // Fingers on the trackpad freeze the rows (no click can land then anyway); the
            // moment the scroll coasts or settles they come back, so a click that stops a
            // flick always reaches its target. Wheel scrolling reports no phases and is
            // caught by the movement below instead.
            tracking.notePhase(phase)
            if phase == .interacting {
                noteScrollMovement()
            } else {
                liftScrollFreeze()
            }
            // What was put off for the scroll runs now that it is over: a repair the blank
            // watch asked for, a refresh the drift asked for.
            if phase == .idle {
                if tracking.repairSkippedWhileScrolling {
                    tracking.repairSkippedWhileScrolling = false
                    tracking.armBlankWatch()
                }
                if tracking.refreshSkippedWhileScrolling {
                    tracking.refreshSkippedWhileScrolling = false
                    tracking.armViewportRefresh()
                }
            }
        }
        .onScrollGeometryChange(for: ScrollMetrics.self, of: ScrollMetrics.init(geometry:)) { old, new in
            scrollChrome.update(travel: new.travel)
            tracking.noteGeometry(offset: new.offset, distanceFromBottom: new.distanceFromBottom, travel: new.travel)
            // The timeline's own nudge (`refreshViewport`) moves the offset a point and back:
            // never the reader scrolling, and never a reason to freeze the rows.
            let nudging = tracking.isNudging
            tracking.noteNudgeSeen()
            // Movement with no phase behind it: wheel ticks. Content growing under a
            // resting reader is not scrolling.
            if !nudging, new.centerY != old.centerY, new.contentHeight == old.contentHeight, !tracking.isCoasting {
                noteScrollMovement()
            }
            // The offset moving by more than the anchor's own move is the reader scrolling: a
            // drag, a flick, or wheel ticks, which report no phase at all. Content growing
            // under a bottom-held reader moves the offset by exactly the growth, and under a
            // top-held one not at all; anything beyond that is the reader. Wheel ticks used
            // to count only in frames where the content stood still, and while a turn
            // streamed rows (or a card ticked) hardly any frame did: the reader's scroll went
            // unrecognised, the end stayed pinned, and the timeline snapped back to it under
            // them. (A snap to the end below moves the offset too, and lands pinned, which
            // is right.) Not while the thread is still arriving (see `holdEnd`): the offset
            // moving then is the layout settling, and reading it as a scroll unpinned the
            // end and left a thread opened at its bottom sitting some way up it.
            let anchorMove = anchorsBottomOnGrowth ? new.contentHeight - old.contentHeight : 0
            let readerMoved = abs((new.offset - old.offset) - anchorMove) > 1
            if readerMoved { tracking.noteReaderMovement() }
            let scrolled = tracking.isUserScrolling
                || (!nudging && !tracking.isArriving && readerMoved
                    && new.containerHeight == old.containerHeight && !tracking.isCoasting)
            let atRest = !tracking.isUserScrolling && !tracking.isCoasting
            if atRest, !scrolled {
                // The content changed shape under a resting reader and an anchor moved the
                // offset to follow: a turn folded, rows arrived or left, history landed
                // above. The lazy stack hears nothing of that and keeps the rows it had built
                // for where the viewport was. Growth the bottom anchor follows is the one
                // change that cannot carry the viewport off them, since the row that grew
                // is the row under it and the offset moves by exactly the growth; anything
                // else is drift, and drift adds up, since a fold under an animation moves
                // the viewport a little each frame. Past a fraction of the viewport, the
                // stack gets told where the viewport is now.
                let shrink = max(0, old.contentHeight - new.contentHeight)
                let drift = anchorsBottomOnGrowth
                    ? abs((new.offset - old.offset) - (new.contentHeight - old.contentHeight))
                    : abs(new.offset - old.offset)
                if tracking.noteDrift(shrink + drift, tolerance: Self.jumpTolerance(viewportHeight)) {
                    tracking.armViewportRefresh()
                }
                if shrink > 1 {
                    // Whatever the rows report, the layout gets checked a beat from now.
                    tracking.armBlankWatch()
                }
            } else if scrolled {
                // The reader's own scrolling tells the stack where the viewport is.
                tracking.resetDrift()
            }
            if new.distanceFromBottom < -1, atRest, new.contentHeight != old.contentHeight || new.containerHeight != old.containerHeight {
                // Only when the content or the container changed shape in this very frame: a
                // wheel bounce past the end moves only the offset and plays out on its own.
                // Past the end of the conversation, showing nothing: the content shrank under
                // the offset. A finished turn folds its whole transcript into one block, and
                // with the reader's place anchored at the top that left the viewport hanging
                // in empty space until they came back to the thread. Never a valid place to
                // be, whether or not the reader was following, so it snaps to the end.
                withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
            } else if historyLoadPending, new.contentHeight > old.contentHeight {
                // The history landed above the viewport with the bottom held; growth
                // anchors by the reader's position again from here.
                historyLoadPending = false
                let pinned = tracking.isPinnedToBottom || new.distanceFromBottom < 48
                if pinned != anchorsBottomOnGrowth { anchorsBottomOnGrowth = pinned }
                // A thread whose first rows fit the pane had nothing to scroll, and the
                // bottom anchor has no offset to hold when the history makes it tall:
                // the viewport stayed at the top of the newly tall content, on the oldest
                // rows. A reader at the end is put back on the end.
                if tracking.isPinnedToBottom, new.distanceFromBottom > 1 {
                    withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
                }
            } else if scrolled {
                // Only the reader's own scrolling decides whether the timeline follows new text.
                // Guarded, so measuring the scroll position never touches anything a body reads.
                let pinned = new.distanceFromBottom < 48
                if pinned != tracking.isPinnedToBottom { tracking.isPinnedToBottom = pinned }
                if pinned != anchorsBottomOnGrowth { anchorsBottomOnGrowth = pinned }
            } else if tracking.isPinnedToBottom, atRest, abs(new.distanceFromBottom) > 1 {
                // Pinned, at rest, and not at the end: the content grew or reflowed to a new
                // width, the chat box took height, or the scroll view lost its place (rows
                // measured after it anchored, a thread opened mid-animation) and is showing
                // empty space past the conversation. The end is where the reader is; the
                // timeline snaps back to it, never slides, so a new row lands in place and
                // fades in on its own. (Mid-resize this waits for the settle below: the
                // anchor already holds the bottom edge each frame.)
                guard !liveResize.isActive else { return }
                withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
            }
        }
        .task(id: runtime.threadID) {
            // The first frame shows only the newest few blocks, so a thread opens at once;
            // the rest of the initial window lands a beat later, above the viewport, while
            // the reader is already looking at the end.
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            // The window lands above the viewport over the next frames: still arriving.
            tracking.holdEnd(for: 1.0)
            loadEarlier(to: TimelineWindow.initial)
            // A thread that opened on nothing (its first rows never laid out where the
            // bottom anchor put the viewport) is caught a beat from now.
            tracking.armBlankWatch()
            await warmMarkdown()
        }
        .onChange(of: tracking.checkRequest) {
            // A coast whose end phase was never delivered is cleared by the phase watch,
            // which asks for this check: the app hears that scroll end here (a no-op when
            // nothing changed).
            tracking.reportActivity(timeline: runtime.threadID)
            repairLayout()
        }
        .onChange(of: FrontMonitor.shared.revision) {
            // The reader is back: the app came to the front or this timeline's window was
            // uncovered. Whatever the conversation did unseen, the timeline checks itself
            // now. Another window coming on screen, a popover or a tooltip opening over the
            // chat included, is not the reader coming back to this one.
            guard FrontMonitor.shared.concerns(tracking.probe?.window) else { return }
            tracking.noteReturnToFront()
        }
        .onChange(of: runtime.isRunning) { _, running in
            // Sending a message always brings the reader back to the conversation's end.
            guard running else {
                // The turn's end folds its steps into its answer, in the block the turn has
                // had all along. A reader at the end is put back on the end once that has
                // laid out, and the stack is told where the viewport is either way.
                if tracking.isPinnedToBottom {
                    DispatchQueue.main.async {
                        withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
                    }
                }
                tracking.armViewportRefresh()
                tracking.armBlankWatch()
                Task { await warmMarkdown() }
                return
            }
            // Whatever phase the last scroll was left in (a coast whose end never came)
            // is over: the turn's own scroll below must not be read as its continuation.
            tracking.notePhase(.idle)
            let wasAtEnd = tracking.isPinnedToBottom
            tracking.isPinnedToBottom = true
            anchorsBottomOnGrowth = true
            if wasAtEnd {
                // Already there: the message and the working line appear in place.
                withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
            } else {
                withAnimation(.easeOut(duration: 0.25)) { position.scrollTo(edge: .bottom) }
            }
            tracking.armViewportRefresh()
        }
        .onChange(of: tracking.refreshRequest) {
            refreshViewport()
        }
        .onChange(of: runtime.questions.count) { old, new in
            // The badge leaving shrinks the content under a held offset without
            // touching the blocks, so nothing arms the viewport refresh the rows
            // below need: tell the stack where the viewport is now, while the rows
            // above still cover it and no blank watch would fire.
            guard old != 0, new == 0 else { return }
            tracking.armViewportRefresh()
        }
        .onChange(of: scrollState.jumpRequest) {
            tracking.isPinnedToBottom = true
            anchorsBottomOnGrowth = true
            withAnimation(.smooth(duration: 0.35)) { position.scrollTo(edge: .bottom) }
        }
        .onChange(of: lastEntryID) {
            // A message, tool row or turn landing at the end of the conversation brings
            // a reader who was at the end down to it, whatever the stack made of the
            // growth; a reader who scrolled up to read is left there, the jump button
            // says something arrived. A question badge is the one row that fetches
            // anyone: it waits on the reader. History loading itself above the viewport
            // must not yank the reader down, and an active drag is never fought: the
            // next entry, or the geometry branch, catches up once it lets go. Growth
            // inside an existing row stays with the pinned machinery above; this is for
            // new rows only.
            guard !historyLoadPending, !tracking.isUserScrolling else { return }
            let isQuestion = lastEntryID?.hasPrefix("question-") == true
            guard tracking.isPinnedToBottom || isQuestion else { return }
            tracking.isPinnedToBottom = true
            anchorsBottomOnGrowth = true
            scrollState.showsJumpButton = false
            withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
        }
        .onChange(of: runtime.threadID) {
            // A new thread starts with a fresh window on its newest messages, and no
            // block from the old one is on screen any more.
            visibleCount = TimelineWindow.firstPaint
            anchorsBottomOnGrowth = true
            historyLoadPending = false
            tracking.clearVisible()
        }
    }

    /// The content just moved under the pointer. Freezes hover and hit-testing on the
    /// rows if they are not already, and (re)arms the watch that lifts the freeze once
    /// no frame has moved for 150ms. The flag, the timestamp and the watch all live on
    /// the tracking object, so noting a frame's movement writes no view state.
    private func noteScrollMovement() {
        tracking.lastMovementAt = CACurrentMediaTime()
        guard !tracking.isReaderScrolling else { return }
        tracking.isReaderScrolling = true
        tracking.reportActivity(timeline: runtime.threadID)
        tracking.settleWatch?.cancel()
        let tracking = tracking
        let timeline = runtime.threadID
        tracking.settleWatch = Task { @MainActor in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(80))
                if CACurrentMediaTime() - tracking.lastMovementAt >= 0.15 {
                    if tracking.isReaderScrolling { tracking.isReaderScrolling = false }
                    tracking.reportActivity(timeline: timeline)
                    return
                }
            }
        }
    }

    private func liftScrollFreeze() {
        tracking.settleWatch?.cancel()
        tracking.settleWatch = nil
        if tracking.isReaderScrolling { tracking.isReaderScrolling = false }
        tracking.reportActivity(timeline: runtime.threadID)
    }

    /// Puts the viewport back on the conversation when a layout pass has left it showing
    /// nothing. The lazy stack lays out rows for the viewport it was last told about, and
    /// nothing tells it when an anchor moves the offset under it: a finished turn folding
    /// its transcript into one block, the working line coming and going, rows reflowing to
    /// a new width. Left alone (the app in the back, another thread open) the conversation
    /// changes shape many times over, and the reader comes back to an empty pane that only
    /// their own scrolling fills in. This runs on request only: when no row has been on
    /// screen for a beat, when a scroll phase that never settled is cleared, and when the
    /// app or its window comes back to the front.
    ///
    /// Past the end, or pinned to the end and away from it, the timeline snaps to the end,
    /// as it does on any measured frame. On the conversation but showing nothing, the stack
    /// is told where the viewport is (`refreshViewport`). Still nothing a beat later, the
    /// stack is rebuilt and told again; still nothing, the timeline goes to the end and
    /// stays pinned there. Rows coming back on screen end the sequence wherever it is, and
    /// a fourth blank in a row is left alone.
    private func repairLayout() {
        guard viewportHeight > 0 else { return }
        guard !tracking.isUserScrolling, !tracking.isCoasting else {
            tracking.repairSkippedWhileScrolling = true
            return
        }
        let distance = tracking.distanceFromBottom
        if distance < -1 || (tracking.isPinnedToBottom && abs(distance) > 1) {
            withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
            // Rows on screen: the snap was all there was to do.
            guard tracking.showsNothing, tracking.blankRepairs < 3 else { return }
            tracking.armViewportRefresh()
        } else if tracking.showsNothing {
            switch tracking.blankRepairs {
            case 0:
                refreshViewport()
            case 1:
                stackGeneration += 1
                tracking.armViewportRefresh()
            case 2:
                tracking.isPinnedToBottom = true
                anchorsBottomOnGrowth = true
                withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
                tracking.armViewportRefresh()
            default:
                return
            }
        } else {
            return
        }
        // Blank, and something was just done about it: checked again a beat from now.
        tracking.blankRepairs += 1
        tracking.armBlankWatch()
    }

    /// A row is opening and pushing content below the fold, inside the tap's animation.
    /// With the end of the conversation on screen, the end is held through the growth
    /// (the bottom anchor follows the rows as they open) and the offset is sent to it in
    /// the same transaction, so the timeline glides down with the opening and lands on
    /// the new end, never short of it and never in a snap. A reader higher up is left
    /// where they are.
    private func revealEnd() {
        guard !scrollState.showsJumpButton else { return }
        tracking.isPinnedToBottom = true
        anchorsBottomOnGrowth = true
        position.scrollTo(edge: .bottom)
        // The glide is a scroll like any other: the shimmers and the card's staves rest,
        // and streamed rows are flushed at the scrolling pace, so its frames are its own.
        noteScrollMovement()
    }

    /// Drift smaller than this leaves the viewport over rows the lazy stack already built:
    /// the working line coming or going, a group collapsing. More can carry it off them.
    private static func jumpTolerance(_ viewportHeight: CGFloat) -> CGFloat {
        max(48, viewportHeight / 4)
    }

    /// One settle when the window stops resizing: bottom-anchored if the reader was at the
    /// end, otherwise the top anchor already held the top-most row each frame. Unanimated,
    /// so no glide restarts per frame mid-drag; the stack is told where the viewport is.
    private func settleAfterResize() {
        guard !tracking.isUserScrolling, !tracking.isCoasting else { return }
        if tracking.isPinnedToBottom {
            withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
        } else {
            tracking.armViewportRefresh()
        }
        tracking.armBlankWatch()
    }

    /// Tells the lazy stack where the viewport is. The stack builds rows for the viewport
    /// the scroll view last reported to it, and an anchor moving the offset under it reports
    /// nothing: a finished turn folding its steps into its answer, rows arriving or leaving,
    /// history landing above. The viewport can then sit over space the stack has built
    /// nothing for, showing nothing, and a request to scroll to where the scroll view
    /// already is changes nothing. A real scroll does: the scroll view moves a point, the
    /// stack builds for where it is, and it moves back once that is done. A reader at the end
    /// is put back on the end after that, since the rows just built may have measured
    /// differently from what the stack had guessed.
    private func refreshViewport() {
        guard viewportHeight > 0, !tracking.isNudging else { return }
        guard !tracking.isUserScrolling, !tracking.isCoasting else {
            tracking.refreshSkippedWhileScrolling = true
            return
        }
        let pinned = tracking.isPinnedToBottom && tracking.distanceFromBottom <= 1
        if tracking.beginNudge() {
            tracking.restoreNudge {
                if pinned {
                    withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
                }
            }
        } else if pinned, tracking.travel > 1 {
            // No scroll view to move (not in a window yet): the offset itself, a point up,
            // and the end again once the stack has built for it.
            let y = tracking.offset
            withTransaction(Self.unanimated) { position.scrollTo(y: y - 1) }
            DispatchQueue.main.async {
                withTransaction(Self.unanimated) { position.scrollTo(edge: .bottom) }
            }
        } else if pinned, tracking.endUnseen, !tracking.isArriving {
            // Nothing to scroll (the conversation is shorter than the pane) and the last
            // block has not shown itself: no point can move, so the stack is remade, the
            // one repair that reaches it.
            stackGeneration += 1
        }
    }

    private nonisolated static func visibleHeight(_ proxy: GeometryProxy) -> CGFloat {
        max(0, proxy.size.height - proxy.safeAreaInsets.top - proxy.safeAreaInsets.bottom)
    }
}

/// Scroll-driven facts about the timeline, kept out of view state so reporting them never
/// re-renders the conversation. Every write is guarded against no-ops: an observable write
/// notifies its readers whether or not the value moved. It also keeps watch over the
/// layout: which rows are on screen, whether a scroll phase has really ended, and when the
/// timeline should check that it is showing anything at all.
@MainActor
@Observable
final class TimelineScrollTracking: ScrollActivityReporter {
    /// Whether new text keeps the timeline at the conversation's end. No view body reads
    /// it, only the scroll closures, so a pin flip must never be able to re-render the
    /// timeline.
    @ObservationIgnored var isPinnedToBottom = true
    /// True while the reader drags or flicks the timeline. Rows neither hover nor hit-test
    /// then: with the pointer resting over content that slides under it, SwiftUI otherwise
    /// re-hit-tests every row's hover region each frame and the rows flip their hover state
    /// (and animate it) as they pass — a fifth of the main thread's scroll-time work.
    /// Observed on purpose: only the leaf modifier that applies the freeze (`ScrollFreeze`)
    /// reads it, so its flips re-render that node and never the timeline's body.
    var isReaderScrolling = false
    /// Watches for the scroll to settle once movement has been seen, so the freeze lifts
    /// 150ms after the last frame that moved, for wheel and trackpad alike.
    @ObservationIgnored var settleWatch: Task<Void, Never>?
    /// True while the reader drags or flicks the timeline, as opposed to it following new text.
    @ObservationIgnored var isUserScrolling = false
    /// When the content last moved, for lifting the scroll freeze.
    @ObservationIgnored var lastMovementAt: CFTimeInterval = 0
    /// True while a flick coasts or a programmatic scroll animates: movement that must
    /// not freeze the rows, since a click may land any moment.
    @ObservationIgnored var isCoasting = false
    /// The message the reader is on, for the rail: the sent message at or above the block
    /// in the middle of what is on screen. Written only when it changes, so the rail
    /// re-renders when the lit tick moves and never else.
    private(set) var activeBlockID: String?
    /// The blocks as laid out, in order, and which of them are sent messages.
    @ObservationIgnored private var outlineIDs: [String] = []
    @ObservationIgnored private var outlineSet: Set<String> = []
    @ObservationIgnored private var userBlockIDs: Set<String> = []
    /// The blocks currently intersecting the viewport, from their own visibility callbacks.
    @ObservationIgnored private var visibleIDs: Set<String> = []
    /// The offset, the distance from the end and the travel from the top as last measured,
    /// for a repair to read without waiting for another frame, and when the offset last moved.
    @ObservationIgnored private(set) var offset: CGFloat = 0
    @ObservationIgnored private(set) var distanceFromBottom: CGFloat = 0
    @ObservationIgnored private(set) var travel: CGFloat = 0
    @ObservationIgnored private var lastOffsetChangeAt: CFTimeInterval = 0
    /// When the offset last moved by more than the anchor's own move (see the timeline's
    /// `readerMoved`): the reader's drag, flick or coast, never content growing under a
    /// held end. What the phase watch below waits to stop.
    @ObservationIgnored private var lastReaderMovementAt: CFTimeInterval = 0
    /// When the phase being watched began, for the ceiling on it.
    @ObservationIgnored private var phaseStartedAt: CFTimeInterval = 0
    @ObservationIgnored private var phaseWatch: Task<Void, Never>?
    /// A repair or a viewport refresh that was skipped mid-scroll, to run once it ends.
    @ObservationIgnored var repairSkippedWhileScrolling = false
    @ObservationIgnored var refreshSkippedWhileScrolling = false
    @ObservationIgnored private var blankWatch: Task<Void, Never>?
    /// How many repairs the current blank has had, so they escalate and then stop. Any
    /// row coming on screen starts the count over.
    @ObservationIgnored var blankRepairs = 0
    /// Bumped when the timeline should look at its own layout (`ThreadTimeline.repairLayout`).
    private(set) var checkRequest = 0
    /// Bumped when the timeline should tell the lazy stack where the viewport is
    /// (`ThreadTimeline.refreshViewport`).
    private(set) var refreshRequest = 0
    /// A view inside the scroll view's content. The scroll view is found through it when a
    /// nudge is due, so a thread switch that swaps the scroll view needs no bookkeeping.
    @ObservationIgnored weak var probe: NSView?
    /// The nudge under way: 0 at rest, 1 moved a point, 2 the scroll view has reported the
    /// move (so the stack has built for it), 3 moving back.
    @ObservationIgnored private var nudgePhase = 0
    @ObservationIgnored private var nudgeOrigin = NSPoint.zero
    @ObservationIgnored private var refreshPending = false
    /// Refreshes come in bursts (one change sets off a few as rows measure); a burst that
    /// never settles is cut off, so a stack that will not build is left alone.
    @ObservationIgnored private var refreshesInBurst = 0
    @ObservationIgnored private var burstStartedAt: CFTimeInterval = 0
    /// How far the offset and the content have moved under the viewport since the stack
    /// last built for where it is.
    @ObservationIgnored private var drift: CGFloat = 0

    var isNudging: Bool { nudgePhase != 0 }

    /// Rows to show, and none of them on screen.
    var showsNothing: Bool {
        !outlineIDs.isEmpty && visibleIDs.isDisjoint(with: outlineSet)
    }

    /// The last block has never reported itself on screen: the end of the conversation,
    /// with older rows possibly still showing above it, has not been drawn.
    var endUnseen: Bool {
        guard let last = outlineIDs.last else { return false }
        return !visibleIDs.contains(last)
    }

    /// Returns whether the outline changed.
    @discardableResult
    func setOutline(_ blocks: [(id: String, hasUserMessage: Bool)]) -> Bool {
        let ids = blocks.map(\.id)
        guard ids != outlineIDs else { return false }
        outlineIDs = ids
        outlineSet = Set(ids)
        userBlockIDs = Set(blocks.filter(\.hasUserMessage).map(\.id))
        resolveActive()
        return true
    }

    func setVisible(_ id: String, _ isVisible: Bool) {
        if isVisible {
            guard visibleIDs.insert(id).inserted else { return }
            // Something is on screen: no blank to repair, and the next one starts fresh.
            blankWatch?.cancel()
            blankWatch = nil
            blankRepairs = 0
        } else {
            guard visibleIDs.remove(id) != nil else { return }
            // The last row left the screen. A replacement normally lands within the
            // frame; the watch catches the case where nothing does.
            if showsNothing { armBlankWatch() }
        }
        resolveActive()
    }

    func clearVisible() {
        visibleIDs.removeAll()
        if activeBlockID != nil { activeBlockID = nil }
    }

    /// Until when the timeline is still arriving: a thread just opened, or history just
    /// landed above the viewport. The offset moves on its own in those frames (the initial
    /// anchor, rows measuring in, the window growing), and none of it is the reader
    /// scrolling: the end stays pinned until the layout has settled or a real scroll
    /// phase says otherwise.
    @ObservationIgnored private var arrivingUntil: CFTimeInterval = 0

    var isArriving: Bool { CACurrentMediaTime() < arrivingUntil }

    func holdEnd(for seconds: TimeInterval) {
        arrivingUntil = max(arrivingUntil, CACurrentMediaTime() + seconds)
    }

    /// The reader moved the offset this frame (see the timeline's `readerMoved`).
    func noteReaderMovement() {
        lastReaderMovementAt = CACurrentMediaTime()
    }

    func noteGeometry(offset: CGFloat, distanceFromBottom: CGFloat, travel: CGFloat) {
        if offset != self.offset { lastOffsetChangeAt = CACurrentMediaTime() }
        self.offset = offset
        self.distanceFromBottom = distanceFromBottom
        self.travel = travel
    }

    /// The timeline this object last reported under, so the report can be withdrawn when
    /// the timeline goes: by then the column's `runtime.threadID` may already name the
    /// next thread.
    @ObservationIgnored private var reportedTimeline: UUID?

    /// Tells the app whether this timeline is moving under its reader: the drag or the
    /// wheel (`isReaderScrolling`) and the coast after a flick (`isCoasting`). Motion
    /// elsewhere pauses on it (see `ScrollActivity`).
    func reportActivity(timeline: UUID) {
        if let reported = reportedTimeline, reported != timeline {
            ScrollActivity.shared.setScrolling(false, timeline: reported)
        }
        reportedTimeline = timeline
        ScrollActivity.shared.setScrolling(isScrollingNow, timeline: timeline, reporter: self)
    }

    /// What this timeline reports (see `ScrollActivity`): the drag or wheel, and the coast.
    var isScrollingNow: Bool { isReaderScrolling || isCoasting }

    /// The timeline is gone (a thread switch, a panel closed): whatever it reported is
    /// withdrawn, mid-drag or mid-coast alike, so the app never goes on hearing it scroll.
    func endActivity() {
        settleWatch?.cancel()
        settleWatch = nil
        phaseWatch?.cancel()
        phaseWatch = nil
        if isReaderScrolling { isReaderScrolling = false }
        isCoasting = false
        isUserScrolling = false
        if let reported = reportedTimeline {
            ScrollActivity.shared.setScrolling(false, timeline: reported)
        }
    }

    /// A phase's end is not always delivered: content growing under an animated scroll
    /// cuts it short, and the app going to the back mid-flick loses the last event. Left
    /// as reported, the timeline would count itself as moving for good and never snap or
    /// repair again. A coast whose reader has not moved the offset for 400ms is over,
    /// whatever was said (content growing under a held end moves the offset too, and used
    /// to keep a cut-short coast alive for as long as a turn streamed: no snap, no repair,
    /// and a running turn the reader could not see until the thread was reopened); a drag
    /// whose finger has not moved for 2s is taken as lost the same way. Three seconds is
    /// the ceiling on any phase. The layout gets checked once it is over.
    func notePhase(_ phase: ScrollPhase) {
        isUserScrolling = phase == .interacting || phase == .decelerating
        isCoasting = phase == .decelerating || phase == .animating
        if isUserScrolling { drift = 0 }
        phaseWatch?.cancel()
        phaseWatch = nil
        guard isCoasting || isUserScrolling else { return }
        let now = CACurrentMediaTime()
        lastOffsetChangeAt = now
        lastReaderMovementAt = now
        phaseStartedAt = now
        let still: CFTimeInterval = phase == .interacting ? 2.0 : 0.4
        phaseWatch = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(200))
                guard let self, !Task.isCancelled else { return }
                let now = CACurrentMediaTime()
                guard now - lastReaderMovementAt >= still || now - phaseStartedAt >= 3.0 else { continue }
                isUserScrolling = false
                isCoasting = false
                if let reported = reportedTimeline, !isReaderScrolling {
                    ScrollActivity.shared.setScrolling(false, timeline: reported)
                }
                checkRequest += 1
                return
            }
        }
    }

    /// Looks a beat from now for any row on screen; none, with rows to show, asks the
    /// timeline to repair its layout. Re-armed by every call; a row reporting itself
    /// visible disarms it.
    func armBlankWatch() {
        blankWatch?.cancel()
        blankWatch = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard let self, !Task.isCancelled, showsNothing else { return }
            checkRequest += 1
        }
    }

    /// The reader is back in front of the timeline: whatever it did unseen, it checks
    /// itself now, with a full run of repairs available, and the stack is told where the
    /// viewport is whatever the rows report.
    func noteReturnToFront() {
        blankRepairs = 0
        checkRequest += 1
        armBlankWatch()
        armViewportRefresh()
    }

    // MARK: Telling the stack where the viewport is

    /// Adds a frame's drift; true once it amounts to more than `tolerance`, which starts
    /// the count over.
    func noteDrift(_ amount: CGFloat, tolerance: CGFloat) -> Bool {
        drift += amount
        guard drift > tolerance else { return false }
        drift = 0
        return true
    }

    func resetDrift() {
        drift = 0
    }

    /// Asks the timeline to tell the lazy stack where the viewport is, a beat from now:
    /// once for however many asks land in the meantime, after any nudge under way, and
    /// never more than a handful in a row.
    func armViewportRefresh() {
        drift = 0
        guard !refreshPending else { return }
        let now = CACurrentMediaTime()
        if now - burstStartedAt > 2 {
            burstStartedAt = now
            refreshesInBurst = 0
        }
        guard refreshesInBurst < 8 else { return }
        refreshesInBurst += 1
        refreshPending = true
        Task { @MainActor [weak self] in
            // A nudge under way finishes first; its own measurements are what this one is for.
            for _ in 0..<12 {
                guard let self, isNudging else { break }
                try? await Task.sleep(for: .milliseconds(17))
            }
            guard let self else { return }
            refreshPending = false
            refreshRequest += 1
        }
    }

    private var scrollView: NSScrollView? {
        guard let probe, probe.window != nil else { return nil }
        return probe.enclosingScrollView
    }

    /// Moves the scroll view a point: up, or down at the very top. The move is the scroll
    /// view's own, so it reports it to the lazy stack the way it reports the reader's
    /// scrolling. Returns false when there is no scroll view to move or nowhere to move it.
    func beginNudge() -> Bool {
        guard nudgePhase == 0, let scrollView else { return false }
        let clip = scrollView.contentView
        let origin = clip.bounds.origin
        for step: CGFloat in [-1, 1] {
            let wanted = NSRect(origin: NSPoint(x: origin.x, y: origin.y + step), size: clip.bounds.size)
            let target = clip.constrainBoundsRect(wanted).origin
            guard abs(target.y - origin.y) > 0.25 else { continue }
            nudgeOrigin = origin
            nudgePhase = 1
            clip.scroll(to: target)
            scrollView.reflectScrolledClipView(clip)
            return true
        }
        return false
    }

    /// The scroll view has reported its geometry since the nudge: the stack has built for it.
    func noteNudgeSeen() {
        if nudgePhase == 1 { nudgePhase = 2 }
    }

    /// Moves back to where the nudge started, once the scroll view has reported the move
    /// (or after a few beats regardless), then runs `completion` for whatever should follow.
    func restoreNudge(then completion: @escaping @MainActor () -> Void) {
        Task { @MainActor [weak self] in
            for _ in 0..<4 {
                try? await Task.sleep(for: .milliseconds(17))
                guard let self, nudgePhase == 1 else { break }
            }
            guard let self, nudgePhase == 1 || nudgePhase == 2 else { return }
            nudgePhase = 3
            if let scrollView {
                let clip = scrollView.contentView
                let wanted = NSRect(origin: nudgeOrigin, size: clip.bounds.size)
                clip.scroll(to: clip.constrainBoundsRect(wanted).origin)
                scrollView.reflectScrolledClipView(clip)
            }
            completion()
            try? await Task.sleep(for: .milliseconds(17))
            nudgePhase = 0
        }
    }

    /// Takes the block in the middle of the on-screen run — a reply filling the viewport is
    /// that block on its own — and lights the sent message at or above it.
    private func resolveActive() {
        let onScreen = outlineIDs.enumerated().filter { visibleIDs.contains($0.element) }
        guard !onScreen.isEmpty else { return }
        let middle = onScreen[onScreen.count / 2].offset
        let resolved = outlineIDs[...middle].last { userBlockIDs.contains($0) }
        if resolved != activeBlockID { activeBlockID = resolved }
    }
}

/// A view of no size inside the scroll view's content, through which the scroll view is
/// found (`TimelineScrollTracking.beginNudge`).
private struct ScrollViewProbe: NSViewRepresentable {
    let onResolve: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        ProbeView(onResolve: onResolve)
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        (nsView as? ProbeView)?.onResolve = onResolve
    }

    private final class ProbeView: NSView {
        var onResolve: (NSView) -> Void

        init(onResolve: @escaping (NSView) -> Void) {
            self.onResolve = onResolve
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { nil }

        /// Only a probe: never in the way of a click or a cursor update.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window != nil { onResolve(self) }
        }
    }
}

/// Bumps when the app comes to the front or one of its windows is uncovered: the moments
/// a reader comes back to a timeline that has been changing unseen. Which window it was
/// is kept with the bump: the app coming to the front concerns every timeline, a window
/// being uncovered only the timelines in it. Every window ordered in posts the same
/// occlusion change, a popover's or a tooltip's included, and a timeline that took those
/// for its own window being uncovered checked itself each time one opened over the chat:
/// a snap to the end, and its scroll view moved a point and back, under a reader who had
/// not gone anywhere.
@MainActor
@Observable
final class FrontMonitor {
    static let shared = FrontMonitor()
    private(set) var revision = 0
    /// Whether anyone can see the app: it is frontmost and one of its windows is on screen.
    /// What the decorations that run without end follow, so a hidden or backgrounded
    /// window animates nothing.
    private(set) var isVisible = true
    /// The window the last bump was for, or none when the app itself came to the front.
    @ObservationIgnored private weak var uncovered: NSWindow?
    @ObservationIgnored private var isAppWide = false
    @ObservationIgnored private var observers: [any NSObjectProtocol] = []

    private init() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.bump(for: nil)
                self?.refreshVisibility()
            }
        })
        observers.append(center.addObserver(
            forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshVisibility() }
        })
        observers.append(center.addObserver(
            forName: NSWindow.didChangeOcclusionStateNotification, object: nil, queue: .main
        ) { [weak self] note in
            // Delivered on the main queue; the window is read on the main actor it belongs to.
            nonisolated(unsafe) let object = note.object
            MainActor.assumeIsolated {
                self?.refreshVisibility()
                guard let window = object as? NSWindow, window.occlusionState.contains(.visible) else { return }
                self?.bump(for: window)
            }
        })
    }

    /// Whether the last bump brought the reader back to a timeline in `window`. A timeline
    /// in no window yet has nothing to check.
    func concerns(_ window: NSWindow?) -> Bool {
        if isAppWide { return true }
        guard let uncovered, let window else { return false }
        return uncovered === window
    }

    private func bump(for window: NSWindow?) {
        uncovered = window
        isAppWide = window == nil
        revision += 1
    }

    private func refreshVisibility() {
        let visible = NSApp.isActive && NSApp.windows.contains { $0.isVisible && $0.occlusionState.contains(.visible) }
        if visible != isVisible { isVisible = visible }
    }
}

/// Scroll measurements, built by a plain initializer rather than an inline closure.
/// Deliberately minimal: every stored field is compared every scroll frame, and any
/// write to view state here would re-render the timeline mid-scroll.
private struct ScrollMetrics: Equatable {
    var contentHeight: CGFloat
    var containerHeight: CGFloat
    var distanceFromBottom: CGFloat
    var travel: CGFloat
    /// The viewport's vertical centre in the content: the sign that it moved.
    var centerY: CGFloat
    /// The raw offset, which only a scroll changes; the centre moves with the viewport's height too.
    var offset: CGFloat

    init(geometry: ScrollGeometry) {
        centerY = geometry.visibleRect.midY
        offset = geometry.contentOffset.y
        contentHeight = geometry.contentSize.height
        containerHeight = geometry.containerSize.height
        let insets = geometry.contentInsets.top + geometry.contentInsets.bottom
        // Never more than the content can actually scroll. While the queue tab folds,
        // the bottom inset shrinks a frame ahead of the filler that keeps a short
        // conversation at the pane's bottom, so the bottom anchor briefly lifts empty
        // space under the chrome; the veil would flash over nothing.
        let excess = max(0, geometry.contentSize.height + insets - geometry.containerSize.height)
        travel = min(max(0, geometry.contentOffset.y + geometry.contentInsets.top), excess)
        if geometry.contentSize.height + insets <= geometry.containerSize.height + 1 {
            distanceFromBottom = 0
        } else {
            distanceFromBottom = geometry.contentSize.height + geometry.contentInsets.bottom
                - geometry.contentOffset.y - geometry.containerSize.height
        }
    }
}

enum TimelineGroup: Identifiable, Equatable {
    case single(TimelineEntry)
    /// A run of tool entries. `startsCollapsed` is true when reply text follows
    /// the run, so past work renders as one tappable summary line above the answer.
    case work(id: String, entries: [TimelineEntry], startsCollapsed: Bool)
    /// The heads a turn sent out (its `.agent` tool calls), in order. Always the last
    /// group of a run and never part of a work run: the heads stay in view as a list of
    /// their own below the turn's steps and replies, whether or not the steps are folded.
    case heads(id: String, entries: [TimelineEntry])

    var id: String {
        switch self {
        case .single(let entry): entry.id
        case .work(let id, _, _): id
        case .heads(let id, _): id
        }
    }

    /// Whether the entry is a head the turn sent out: a tool call of the `.agent` kind.
    @MainActor
    static func isHead(_ entry: TimelineEntry) -> Bool {
        if case .tool(let call) = entry.item.content, call.kind == .agent { return true }
        return false
    }

    @MainActor
    static func build(_ entries: [TimelineEntry], showReasoning: Bool) -> [TimelineGroup] {
        // Positions of replies. A work run sitting before one has been moved on from, so it
        // starts collapsed. Kind only, never the text: reading streaming content here made the
        // whole timeline regroup on every token. A reply row is created by its first delta.
        var replyIndices: [Int] = []
        for (index, entry) in entries.enumerated() where entry.kind == .assistant {
            replyIndices.append(index)
        }
        var groups: [TimelineGroup] = []
        var work: [TimelineEntry] = []
        var workStart = 0
        var heads: [TimelineEntry] = []

        func flushWork() {
            guard let first = work.first else { return }
            let workEnd = workStart + work.count - 1
            let collapsed = replyIndices.contains { $0 > workEnd }
            groups.append(.work(id: "work-\(first.id)", entries: work, startsCollapsed: collapsed))
            work.removeAll()
        }

        for (index, entry) in entries.enumerated() {
            switch entry.kind {
            case .tool:
                // A head sent out never joins the work run: it goes to the heads list at
                // the end, and the commands around it stay one run.
                if isHead(entry) {
                    heads.append(entry)
                    continue
                }
                if let last = work.last, last.turnID != entry.turnID { flushWork() }
                if work.isEmpty { workStart = index }
                work.append(entry)
            case .reasoning:
                // Thinking lives behind the working indicator's chevron, never as a row of its own.
                continue
            default:
                flushWork()
                groups.append(.single(entry))
            }
        }
        flushWork()
        if let first = heads.first {
            groups.append(.heads(id: "heads-\(first.id)", entries: heads))
        }
        return groups
    }
}

/// A turn is one block of the timeline from the moment it starts. While it runs, the block
/// shows its prompt, its steps and replies as rows, and the working line; when it ends it
/// folds to its final response plus its file summary, so the chat stays clean, and the
/// chevron re-opens the full steps. One block throughout, so rows arriving and leaving
/// while the turn runs, and the fold at its end, change what the block shows and never
/// which blocks the lazy stack holds: the stack keeps its place through all of it.
/// Entries without a turn render as plain groups.
///
/// Equatable, so a rebuilt timeline can tell an unchanged block from a changed one and
/// skip its row. Entries compare by identity: what a row shows of an entry it observes
/// itself, so the same entry object always means the same row.
enum DisplayBlock: Identifiable, Equatable {
    /// A turn, with its entries in order (thinking and the end marker left out): folded
    /// once it has a summary, running (or ended without its marker) until then.
    /// `showsWorking` is the working line at the end of the running turn.
    case turn(id: String, turnID: UUID, entries: [TimelineEntry], summary: TurnSummary?, showsWorking: Bool)
    /// A plain group, with the facts its row needs from the turn it belongs to: the turn's
    /// summary on the turn's last reply, and whether the turn produced a reply on its end marker.
    case group(TimelineGroup, summary: TurnSummary?, hasReply: Bool)
    /// The working line on its own, for a running turn whose latest entries belong to no
    /// turn. Never built from the entries: the timeline appends it while waiting on a reply.
    case working(turnID: UUID?, liveWork: [TimelineEntry])
    /// The team's work on its way to the remote once the lead's turn is over (see
    /// `AppModel.autoMergeHydraWork`). Never built from the entries either: the timeline
    /// appends its pill while the merge runs and drops it when the note about the outcome
    /// lands, the pill morphing in place into the merged report. Carries the id the
    /// outcome note will land under (`ThreadRuntime.hydraMergeNoteID`), so the note's
    /// block takes this one's identity and the pill morphs into the merged report.
    case merging(id: String)
    /// The heads still out on the lead's behalf once its turn is over, by their roster
    /// index in the order they went. Never built from the entries: the timeline appends
    /// it while any head works and drops it when the last one reports back.
    case headsWorking(heads: [Int])

    var id: String {
        switch self {
        case .turn(let id, _, _, _, _): id
        case .group(let group, _, _): group.id
        case .working(let turnID, _): "working-\(turnID?.uuidString ?? "")"
        case .merging(let id): id
        case .headsWorking: "hydra-heads-working"
        }
    }

    /// The turn the block belongs to, for the revert controls.
    var turnID: UUID? {
        switch self {
        case .turn(_, let turnID, _, _, _): turnID
        case .group(.single(let entry), _, _): entry.turnID
        case .group(.work, _, _), .group(.heads, _, _), .working, .merging, .headsWorking: nil
        }
    }

    /// Whether the block holds a message the user sent (a turn prompt or a
    /// standalone user message). Only these get rail ticks.
    var hasUserMessage: Bool {
        switch self {
        case .turn(_, _, let entries, _, _): entries.contains { $0.kind == .user }
        case .group(.single(let entry), _, _): entry.kind == .user
        case .group(.work, _, _), .group(.heads, _, _), .working, .merging, .headsWorking: false
        }
    }

    /// The merge pill's state when this block is the pill: the merge under way, or the
    /// note about its outcome. The note lands under the merging block's own id (see
    /// `ThreadRuntime.hydraMergeNoteID`), so the block keeps its identity across the
    /// change and one row morphs from the one state to the other.
    @MainActor
    var hydraMergePhase: HydraMergePhase? {
        switch self {
        case .merging: return .merging
        case .group(.single(let entry), _, _):
            guard case .user(let message) = entry.item.content, Self.isMergeOutcome(message) else { return nil }
            return .outcome(message)
        default: return nil
        }
    }

    /// What a build leaves behind besides its blocks: where the last run began. The runs
    /// before it are what an append never touches, so the cache keeps their blocks and
    /// builds the rest again (see `TimelineBlockCache`).
    struct Built {
        var blocks: [DisplayBlock]
        /// How many blocks at the front come from runs before the last one. The synthetic
        /// blocks at the end never count.
        var stableCount: Int
        /// The index of the entry the last run begins with.
        var lastRunStart: Int
    }

    @MainActor
    static func build(_ entries: [TimelineEntry], meta: TimelineMeta, showReasoning: Bool, isRunning: Bool, hydraMergeID: String?, workingHeads: [Int] = []) -> [DisplayBlock] {
        build(entries, from: 0, keeping: [], meta: meta, showReasoning: showReasoning, isRunning: isRunning, hydraMergeID: hydraMergeID, workingHeads: workingHeads).blocks
    }

    /// The blocks for the entries from `start` on, after `kept`: the blocks the entries
    /// before `start` built last time. Building from 0 with nothing kept is the full build.
    @MainActor
    static func build(_ entries: [TimelineEntry], from start: Int, keeping kept: ArraySlice<DisplayBlock>, meta: TimelineMeta, showReasoning: Bool, isRunning: Bool, hydraMergeID: String?, workingHeads: [Int]) -> Built {
        // Partition into contiguous runs sharing one turnID (nil groups together), so every
        // turn becomes one block. A run is a range of indices: no entry is copied to find it.
        var runs: [(turnID: UUID?, range: Range<Int>)] = []
        for index in start..<entries.count {
            let turnID = entries[index].turnID
            if let last = runs.last, last.turnID == turnID {
                runs[runs.count - 1].range = last.range.lowerBound..<(index + 1)
            } else {
                runs.append((turnID, index..<(index + 1)))
            }
        }
        // While a turn runs and no reply has started, the working line ends its block. The
        // moment the reply it is waiting on arrives, the line goes and the reply takes its
        // place; it comes back below when the agent moves on to another step.
        let waiting = isRunning && entries.last?.kind != .assistant
        var blocks = Array(kept)
        blocks.reserveCapacity(kept.count + runs.count + 3)
        var stableCount = blocks.count
        for (index, run) in runs.enumerated() {
            let isLast = index == runs.count - 1
            if isLast { stableCount = blocks.count }
            if let turnID = run.turnID {
                // Thinking lives behind the working line's chevron, never as a row of its own.
                let rows = entries[run.range].filter { $0.kind != .turnEnd && $0.kind != .reasoning }
                let summary = meta.summaryByTurn[turnID]
                let showsWorking = waiting && summary == nil && isLast
                // Named by the turn and the run's first row: a note filed under no turn
                // landing mid-turn splits the turn into two runs, and two blocks under one
                // id left the second, the running tail, undrawn.
                let runID = entries[run.range.lowerBound].id
                blocks.append(.turn(id: "turn-\(turnID.uuidString)-\(runID)", turnID: turnID, entries: rows, summary: summary, showsWorking: showsWorking))
            } else {
                for group in TimelineGroup.build(Array(entries[run.range]), showReasoning: showReasoning) {
                    blocks.append(.group(group, summary: meta.summary(for: group), hasReply: meta.hasReply(for: group)))
                }
            }
        }
        if waiting, !(blocks.last?.carriesWorkingLine ?? false) {
            blocks.append(.working(turnID: entries.last?.turnID, liveWork: []))
        }
        // Heads out on the lead's behalf once its turn is over: their row says who is at
        // work until they report back. While the lead itself runs, its working line and
        // the heads' own rows already say so.
        if !workingHeads.isEmpty, !isRunning {
            blocks.append(.headsWorking(heads: workingHeads))
        }
        // The merge begins once the lead's turn is over; while it runs, its pill ends the
        // timeline, and stays if the user starts the lead on something else meanwhile.
        // The pill shares the merged report's frame, so when the note about the outcome
        // lands the merging state morphs into it in place. The outcome and the pill never
        // show together: the note and the flag clearing land in one pass, and this guard
        // covers the beat between them, so there is no second row.
        if let hydraMergeID, !Self.endsWithMergeOutcome(entries) {
            blocks.append(.merging(id: hydraMergeID))
        }
        return Built(blocks: blocks, stableCount: stableCount, lastRunStart: runs.last?.range.lowerBound ?? start)
    }

    private var carriesWorkingLine: Bool {
        if case .turn(_, _, _, _, true) = self { return true }
        return false
    }

    /// Whether the timeline already ends with the merge's outcome note, so the merging
    /// pill has already become the merged report and must not linger as a second row.
    /// Merge notes are Hydra's own (no heads behind them) and titled "Hydra …"; a head's
    /// landing or patch note leads with the head's name instead.
    @MainActor
    private static func endsWithMergeOutcome(_ entries: [TimelineEntry]) -> Bool {
        guard let last = entries.last, case .user(let message) = last.item.content else { return false }
        return isMergeOutcome(message)
    }

    /// Whether a Hydra note is about a merge's outcome. Merge notes are Hydra's own (no
    /// heads behind them) and titled "Hydra …"; a head's landing or patch note leads with
    /// the head's name instead.
    static func isMergeOutcome(_ message: UserMessage) -> Bool {
        guard message.isFromHydra, (message.hydraHeads ?? []).isEmpty else { return false }
        let title = message.text.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? ""
        return title.hasPrefix("Hydra ")
    }
}

/// The last build of the timeline's blocks, with everything that build read. Entries are
/// compared by identity (a pointer each) and by kind, never by content, so a body run that
/// changed none of them gets its blocks back instead of building them again, and the body
/// never observes a streaming row through the key. The per-turn meta is the cache's own,
/// built with the blocks. Nothing else reaches the blocks: they are handed straight to
/// the stack, whose rows compare them.
@MainActor
final class TimelineBlockCache {
    /// One cache per open thread, kept across the timeline view being rebuilt, so coming
    /// back to a chat reuses its blocks; the runtime keeps the entries alive so their
    /// identities stay valid.
    @MainActor private static var shared: [UUID: TimelineBlockCache] = [:]
    @MainActor private static var order: [UUID] = []
    @MainActor static func cache(for threadID: UUID) -> TimelineBlockCache {
        if let existing = shared[threadID] {
            order.removeAll { $0 == threadID }
            order.append(threadID)
            return existing
        }
        let created = TimelineBlockCache()
        shared[threadID] = created
        order.append(threadID)
        while order.count > 24 {
            let oldest = order.removeFirst()
            shared.removeValue(forKey: oldest)
        }
        return created
    }

    private struct Key: Equatable {
        var entries: [ObjectIdentifier]
        /// The fold: a turn's block folds the moment its end marker lands. The marker is an
        /// entry of its own, so the identities say so already; the count says so again for
        /// the price of one integer, should a marker ever be written into a row in place.
        var turnEnds: Int
        var isRunning: Bool
        /// The merge's row comes and goes on this alone; no entry changes for it.
        var hydraMergeID: String?
        /// The heads' row likewise: it comes and goes as heads start and finish.
        var workingHeads: [Int]
        var showReasoning: Bool

        /// Whether this key is `old` with entries appended and nothing else changed.
        func extends(_ old: Key) -> Bool {
            entries.count > old.entries.count
                && isRunning == old.isRunning
                && hydraMergeID == old.hydraMergeID
                && workingHeads == old.workingHeads
                && showReasoning == old.showReasoning
                && entries.starts(with: old.entries)
        }
    }

    private var key: Key?
    private var built = DisplayBlock.Built(blocks: [], stableCount: 0, lastRunStart: 0)
    private var meta = TimelineMeta()
    /// The mention list, once per roster of heads: heads come and go far more rarely
    /// than this body runs.
    private var personaHeads: [Int?] = []
    private var personas: [HydraPersona] = []
    #if DEBUG
    private var tailBuilds = 0
    #endif

    func blocks(for entries: [TimelineEntry], showReasoning: Bool, isRunning: Bool, hydraMergeID: String?, workingHeads: [Int] = []) -> [DisplayBlock] {
        var identities: [ObjectIdentifier] = []
        identities.reserveCapacity(entries.count)
        var turnEnds = 0
        for entry in entries {
            identities.append(ObjectIdentifier(entry))
            if entry.kind == .turnEnd { turnEnds += 1 }
        }
        let wanted = Key(
            entries: identities,
            turnEnds: turnEnds,
            isRunning: isRunning,
            hydraMergeID: hydraMergeID,
            workingHeads: workingHeads,
            showReasoning: showReasoning
        )
        if wanted == key { return built.blocks }
        if let key, wanted.extends(key),
           let tail = buildTail(entries, appendedFrom: key.entries.count, showReasoning: showReasoning, isRunning: isRunning, hydraMergeID: hydraMergeID, workingHeads: workingHeads) {
            built = tail
        } else {
            meta = TimelineMeta.build(entries)
            built = DisplayBlock.build(entries, from: 0, keeping: [], meta: meta, showReasoning: showReasoning, isRunning: isRunning, hydraMergeID: hydraMergeID, workingHeads: workingHeads)
        }
        key = wanted
        return built.blocks
    }

    /// The append fast path: entries arrived and nothing else changed. Every run but the
    /// last is as it was, and so are its blocks: a turn block that is not the last carries
    /// no working line, and a plain group reads nothing beyond its own run. So the last run
    /// and the synthetic blocks after it are built again, over the meta grown by the new
    /// entries, and the rest is kept. Nil when a new end marker folds a kept turn block;
    /// that build starts over.
    private func buildTail(_ entries: [TimelineEntry], appendedFrom appended: Int, showReasoning: Bool, isRunning: Bool, hydraMergeID: String?, workingHeads: [Int]) -> DisplayBlock.Built? {
        var grown = meta
        for entry in entries[appended...] {
            grown.add(entry)
            if entry.kind == .turnEnd, case .turnEnd(let summary) = entry.item.content,
               built.blocks[..<built.stableCount].contains(where: { $0.turnID == summary.turnID }) {
                return nil
            }
        }
        let tail = DisplayBlock.build(entries, from: built.lastRunStart, keeping: built.blocks[..<built.stableCount], meta: grown, showReasoning: showReasoning, isRunning: isRunning, hydraMergeID: hydraMergeID, workingHeads: workingHeads)
        #if DEBUG
        // The fast path must be invisible: now and then, hold it against a full build.
        // A difference is a bug in the fast path, not a reason to crash a chat: it is
        // logged, and the full build stands in.
        tailBuilds += 1
        if tailBuilds % 20 == 0 {
            let full = DisplayBlock.build(entries, meta: grown, showReasoning: showReasoning, isRunning: isRunning, hydraMergeID: hydraMergeID, workingHeads: workingHeads)
            if tail.blocks != full {
                print("ThreadTimeline: the tail build differed from a full build after \(entries.count) entries")
                return nil
            }
        }
        #endif
        meta = grown
        return tail
    }

    /// The personas the composer can mention, by the heads' roster indices. Memoized on
    /// them, so the list is built when a head comes or goes and not on every pass.
    func mentionPersonas(for heads: [ChatThread]) -> [HydraPersona] {
        let indices = heads.map { $0.hydra?.index }
        if indices == personaHeads { return personas }
        var seen = Set<String>()
        personas = heads.compactMap { $0.hydra?.persona }.filter { seen.insert($0.name).inserted }
        personaHeads = indices
        return personas
    }
}

/// Lazy-loading window for the timeline: only the newest groups are materialized, so
/// the number of live views stays bounded even for very long threads.
/// Opening a thread builds and lays out every block in the window before the first
/// frame — the bottom anchor needs the content's full height, and a block costs several
/// milliseconds — so the window opens with a handful, grows to its resting size a beat
/// later, and then a page at a time as the reader scrolls back.
enum TimelineWindow {
    static let firstPaint = 6
    static let initial = 24
    static let page = 24
}

/// Per-turn facts computed once per timeline pass, so rows never scan the thread to
/// find their own summary. `kind`, `id` and `turnID` are fixed for a row's lifetime,
/// and finished turn summaries never change, so building this never costs renders
/// while streaming.
struct TimelineMeta {
    var lastAssistantIDByTurn: [UUID: String] = [:]
    var summaryByTurn: [UUID: TurnSummary] = [:]
    var turnsWithReply: Set<UUID> = []

    @MainActor
    static func build(_ entries: [TimelineEntry]) -> TimelineMeta {
        var meta = TimelineMeta()
        for entry in entries { meta.add(entry) }
        return meta
    }

    /// Takes one more entry, in timeline order. Only an end marker's content is read, and
    /// that never changes once written.
    @MainActor
    mutating func add(_ entry: TimelineEntry) {
        switch entry.kind {
        case .assistant:
            if let turnID = entry.turnID {
                lastAssistantIDByTurn[turnID] = entry.id
                turnsWithReply.insert(turnID)
            }
        case .turnEnd:
            if case .turnEnd(let summary) = entry.item.content {
                summaryByTurn[summary.turnID] = summary
            }
        default:
            break
        }
    }

    /// The turn's summary, shown on the turn's last reply only. A dictionary lookup,
    /// so rows never scan the thread.
    @MainActor
    func summary(for group: TimelineGroup) -> TurnSummary? {
        guard case .single(let entry) = group, entry.kind == .assistant,
              let turnID = entry.turnID, lastAssistantIDByTurn[turnID] == entry.id else { return nil }
        return summaryByTurn[turnID]
    }

    /// Whether a turn-end marker's turn produced a reply; when it did, the duration lives
    /// on the reply's hover line instead.
    @MainActor
    func hasReply(for group: TimelineGroup) -> Bool {
        guard case .single(let entry) = group, entry.kind == .turnEnd, let turnID = entry.turnID else { return false }
        return turnsWithReply.contains(turnID)
    }
}

/// What a row needs from outside its block, as plain values. Rows never read the app
/// model or the runtime's turn list themselves, so a change anywhere else in the app
/// never re-renders them; only a change to these values does.
struct RowContext: Equatable {
    var workingDirectory: String?
    /// Whether the block's turn can be reverted right now.
    var canRewind: Bool
}

private struct DisplayBlockView: View, Equatable {
    let block: DisplayBlock
    let runtime: ThreadRuntime
    let context: RowContext

    nonisolated static func == (lhs: DisplayBlockView, rhs: DisplayBlockView) -> Bool {
        lhs.block == rhs.block && lhs.runtime === rhs.runtime && lhs.context == rhs.context
    }

    private static func splitTurnEntries(_ entries: [TimelineEntry]) -> (userEntries: [TimelineEntry], content: [TimelineEntry]) {
        var userEntries: [TimelineEntry] = []
        var content: [TimelineEntry] = []
        content.reserveCapacity(entries.count)
        for entry in entries {
            if entry.kind == .user { userEntries.append(entry) } else { content.append(entry) }
        }
        return (userEntries, content)
    }

    var body: some View {
        // The merge pill in either state: the block keeps its id from the merge's first
        // stage to its outcome note, so this one branch keeps the row's identity and the
        // pill morphs rather than being swapped for another.
        if let phase = block.hydraMergePhase {
            HydraMergeRow(runtime: runtime, phase: phase)
        } else {
            switch block {
            case .group(let group, let summary, let hasReply):
                TimelineGroupView(group: group, runtime: runtime, summary: summary, hasReply: hasReply, context: context)
            case .turn(_, let turnID, let entries, let summary, let showsWorking):
                // The fold is a change of what the block shows, in the same frame, with no
                // transition: a fade over a block that can run to thousands of points is a
                // whole-layer composite the renderer may draw as nothing.
                if let summary {
                    let split = Self.splitTurnEntries(entries)
                    TurnFinishedBlock(
                        runtime: runtime,
                        turnID: turnID,
                        summary: summary,
                        userEntries: split.userEntries,
                        content: split.content,
                        workingDirectory: context.workingDirectory,
                        canUndo: context.canRewind
                    )
                    .transition(.identity)
                } else {
                    TurnRunningBlock(runtime: runtime, entries: entries, showsWorking: showsWorking, context: context)
                        .transition(.identity)
                }
            case .working(_, let liveWork):
                WorkingBlockView(runtime: runtime, liveWork: liveWork, workingDirectory: context.workingDirectory)
            case .merging:
                // Routed through `hydraMergePhase` above; never reached.
                EmptyView()
            case .headsWorking(let heads):
                HydraHeadsWorkingRow(heads: heads, runtime: runtime)
            }
        }
    }
}

/// A turn while it runs (or one that ended without its marker, the app having quit under
/// it): its prompt, its steps and replies, and anything sent to steer it, as rows of their
/// own in order, each arriving like a row of the stack, and the working line at the end
/// while the agent is between replies. The turn's trailing tool run is not a row of its own
/// meanwhile: the working line carries it (its summary beside the spinner, its steps behind
/// the chevron) until a reply follows, when it comes back as a collapsed group above the answer.
private struct TurnRunningBlock: View {
    let runtime: ThreadRuntime
    let entries: [TimelineEntry]
    let showsWorking: Bool
    let context: RowContext
    /// The partition as last built, keyed on the entries' identities: grouping is by kind
    /// only, so it changes when a row comes or goes and never while one streams.
    @State private var groupCache = TurnGroupCache()

    var body: some View {
        var groups = groupCache.groups(for: entries)
        // The heads the turn sent out come last from the build; they render below the
        // steps and replies and above the working line, never inside the working line.
        var heads: TimelineGroup?
        if case .heads? = groups.last {
            heads = groups.removeLast()
        }
        var liveWork: [TimelineEntry] = []
        if showsWorking, case .work(_, let entries, false)? = groups.last {
            liveWork = entries
            groups.removeLast()
        }
        // A plain stack: a lazy stack inside a lazy row resolved its own visibility on every
        // scrolled frame for nothing, the row is on screen whole or not at all.
        return VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
            ForEach(groups) { group in
                TurnRow(group: group, runtime: runtime, context: context)
                    .equatable()
                    .transition(ThreadTimeline.rowTransition)
            }
            if let heads {
                TurnRow(group: heads, runtime: runtime, context: context)
                    .equatable()
                    .transition(ThreadTimeline.rowTransition)
            }
            if showsWorking {
                WorkingBlockView(runtime: runtime, liveWork: liveWork, workingDirectory: context.workingDirectory)
                    .transition(ThreadTimeline.rowTransition)
            }
        }
    }
}

/// A running turn's last partition, with the identities it was built from. An object, so
/// handing the groups back is not a state write.
@MainActor
private final class TurnGroupCache {
    private var identities: [ObjectIdentifier] = []
    private var groups: [TimelineGroup] = []

    func groups(for entries: [TimelineEntry]) -> [TimelineGroup] {
        let wanted = entries.map(ObjectIdentifier.init)
        if wanted != identities {
            groups = TimelineGroup.build(entries, showReasoning: false)
            identities = wanted
        }
        return groups
    }
}

/// One row of a running turn. Equatable, so a new entry re-renders its own row and none
/// of the rows above it.
private struct TurnRow: View, Equatable {
    let group: TimelineGroup
    let runtime: ThreadRuntime
    let context: RowContext

    nonisolated static func == (lhs: TurnRow, rhs: TurnRow) -> Bool {
        lhs.group == rhs.group && lhs.runtime === rhs.runtime && lhs.context == rhs.context
    }

    var body: some View {
        // A running turn has no summary yet and no end marker: nothing for a row to carry
        // from the turn beyond the group itself.
        TimelineGroupView(group: group, runtime: runtime, summary: nil, hasReply: false, context: context)
    }
}

struct TimelineGroupView: View {
    let group: TimelineGroup
    let runtime: ThreadRuntime
    /// The turn's summary, when this group is the turn's last reply.
    let summary: TurnSummary?
    /// Whether the turn produced a reply, for a turn-end marker.
    let hasReply: Bool
    let context: RowContext

    var body: some View {
        switch group {
        case .single(let entry):
            switch entry.kind {
            case .user: UserMessageRow(entry: entry, runtime: runtime, canRevert: context.canRewind)
            case .assistant: AssistantMessageRow(entry: entry, summary: summary, runtime: runtime)
            case .reasoning: EmptyView()
            case .tool: WorkGroup(entries: [entry], runtime: runtime, workingDirectory: context.workingDirectory)
            case .plan: PlanCard(entry: entry, runtime: runtime)
            case .todos: TodoListRow(entry: entry)
            case .notice: NoticeRow(entry: entry)
            case .turnEnd:
                if case .turnEnd(let summary) = entry.item.content {
                    TurnEndRow(summary: summary, hasReply: hasReply)
                }
            }
        case .work(_, let entries, let startsCollapsed):
            WorkGroup(entries: entries, runtime: runtime, workingDirectory: context.workingDirectory, startsCollapsed: startsCollapsed)
        case .heads(_, let entries):
            HydraHeadsList(entries: entries, runtime: runtime, workingDirectory: context.workingDirectory)
        }
    }
}

/// The one line a running turn shows: the head panel's progress card, with the task
/// breathing, the stave bar filling with the turn's steps, the time in its pill and the
/// chevron opening the thinking and the run's steps beneath it.
private struct WorkingIndicator: View {
    let runtime: ThreadRuntime
    let startedAt: Date
    let seed: UInt64
    /// The running turn's thinking. When there is any, a chevron opens it beneath the indicator.
    let thinkingSteps: [ThinkingStep]
    /// The tool run in progress, folded into this line while it runs.
    let liveWork: [TimelineEntry]
    let turnEntries: [TimelineEntry]
    /// The card with the bar (see `AppSettings.showsWorkingCard`); off, the one-line badge.
    let showsCard: Bool
    var workingDirectory: String?
    @Environment(\.chatZoom) private var zoom
    @Environment(\.revealTimelineEnd) private var revealBox
    @State private var now = Date.now
    @State private var isExpanded = false

    /// The one motion for whatever changes on the line: the words, the chevron.
    private static let change = Animation.smooth(duration: 0.3)
    /// The panel's card needs room for the bar; a badge hugging its words would leave the bar no width.
    private static let cardWidth: CGFloat = 380

    var body: some View {
        let elapsed = now.timeIntervalSince(startedAt)
        let word = WorkingWords.word(seed: seed, elapsedSeconds: Int64(max(0, elapsed)))
        let label = liveWork.isEmpty ? "\(word)…" : WorkGroupSummary.text(for: liveWork)
        let canExpand = !thinkingSteps.isEmpty || !liveWork.isEmpty
        VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
            Button {
                guard canExpand else { return }
                withAnimation(.snappy(duration: 0.24)) {
                    isExpanded.toggle()
                    if isExpanded { revealBox?.action() }
                }
            } label: {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        HydraWorkingTitle(text: label, isRunning: true, alignment: .leading)
                            .frame(maxWidth: showsCard ? .infinity : nil, alignment: .leading)
                        HydraElapsedTime(startedAt: startedAt, finishedAt: nil, isRunning: true)
                        // The chevron only once there is something to open: an empty slot
                        // held for it left the time short of the card's edge, as if misaligned.
                        // The card's `canExpand` animation glides the time over when it comes.
                        if canExpand {
                            Image(systemName: "chevron.right")
                                .font(.chat(.caption2, weight: .semibold, zoom: zoom))
                                .foregroundStyle(.tertiary)
                                .rotationEffect(.degrees(isExpanded ? 90 : 0))
                                .transition(.opacity)
                        }
                    }
                    // The bar is the card's; the badge is the line above alone, hugging its words.
                    if showsCard {
                        HydraProgressBar(runtime: runtime, startedAt: startedAt, finishedAt: nil, status: .running, tint: Chrome.accent, entries: turnEntries)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, showsCard ? 11 : 8)
                .frame(width: showsCard ? Self.cardWidth : nil)
                .background {
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(.quaternary.opacity(0.32))
                }
                .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                .geometryGroup()
            }
            .buttonStyle(.plain)
            .help(canExpand ? (isExpanded ? "Hide these steps" : "Show these steps") : "")
            .accessibilityHint(canExpand ? Text("Shows what the agent is doing") : Text(""))

            if isExpanded, canExpand {
                if !thinkingSteps.isEmpty {
                    VStack(alignment: .leading, spacing: TimelineMetrics.rowSpacing) {
                        ForEach(Array(thinkingSteps.enumerated()), id: \.offset) { _, step in
                            MarkdownView(text: step.text, isStreaming: step.isStreaming).equatable()
                        }
                    }
                    .foregroundStyle(.secondary)
                    .environment(\.markdownPointSize, 12)
                    .environment(\.markdownDimmed, true)
                    .padding(.leading, TimelineMetrics.iconWidth + TimelineMetrics.iconSpacing)
                    .transition(.softAppear)
                }
                if !liveWork.isEmpty {
                    WorkSteps(entries: liveWork, runtime: runtime, workingDirectory: workingDirectory)
                        .transition(.softAppear)
                }
            }
        }
        .animation(Self.change, value: label)
        .animation(Self.change, value: canExpand)
        .font(.chat(.callout, zoom: zoom))
        .task(id: FrontMonitor.shared.isVisible) {
            // The elapsed-time label ticks only while someone can see it.
            guard FrontMonitor.shared.isVisible else { return }
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                now = .now
            }
        }
    }
}

/// One piece of the running turn's thinking, and whether it is still arriving.
struct ThinkingStep: Equatable {
    var text: String
    var isStreaming: Bool
}

/// Whether the reader has scrolled away from the latest message, shared with the composer that shows the jump button.
@MainActor
@Observable
final class TimelineScrollState {
    var showsJumpButton = false
    private(set) var jumpRequest = 0

    func jumpToLatest() {
        jumpRequest += 1
    }
}

/// What a row calls as it opens, inside its own animation, so the timeline can follow the
/// opening down to the end of the conversation (see `ThreadTimeline.revealEnd`). A box
/// with an identity, like `PopoverCloseBox`: a bare closure in the environment would tell
/// every row it is out of date on every update of the timeline; the box is made once and
/// only its action is renewed. Outside a timeline it does nothing.
@MainActor
final class TimelineRevealBox: Equatable {
    var action: () -> Void = {}

    nonisolated static func == (lhs: TimelineRevealBox, rhs: TimelineRevealBox) -> Bool {
        lhs === rhs
    }
}

private struct RevealTimelineEndKey: EnvironmentKey {
    static let defaultValue: TimelineRevealBox? = nil
}

extension EnvironmentValues {
    var revealTimelineEnd: TimelineRevealBox? {
        get { self[RevealTimelineEndKey.self] }
        set { self[RevealTimelineEndKey.self] = newValue }
    }
}

/// The working block's row: the indicator with the running turn's thinking behind its
/// chevron. The timeline appends and drops the block, so this holds no condition of its own
/// and no transition: the row arrives and leaves exactly like every other block. The indicator
/// is one row tall, like a reply, so the reply that takes its line begins where its text was.
private struct WorkingBlockView: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    /// The tool run in progress, shown on the working line instead of as a group.
    let liveWork: [TimelineEntry]
    let workingDirectory: String?

    var body: some View {
        let turnEntries = Self.turnEntries(of: runtime)
        WorkingIndicator(
            runtime: runtime,
            startedAt: runtime.turnStartedAt ?? .now,
            seed: WorkingWords.seed(runtime.threadID.uuidString),
            thinkingSteps: model.settings.showReasoning ? thinking : [],
            liveWork: liveWork,
            turnEntries: turnEntries,
            showsCard: model.settings.showsWorkingCard,
            workingDirectory: workingDirectory
        )
    }

    /// The running turn's rows, in order: the tool calls and replies since the turn's
    /// prompt, for the card's bar. Walks back from the end and stops at the previous turn.
    private static func turnEntries(of runtime: ThreadRuntime) -> [TimelineEntry] {
        guard let turnID = runtime.entries.last.flatMap(\.turnID) else { return [] }
        var rows: [TimelineEntry] = []
        for entry in runtime.entries.reversed() {
            guard let entryTurn = entry.turnID else { continue }
            guard entryTurn == turnID else { break }
            rows.append(entry)
        }
        return rows.reversed()
    }

    /// The running turn's thinking. Kind and turn are fixed at creation, so they are checked before
    /// any content is read.
    private var thinking: [ThinkingStep] {
        guard let turnID = runtime.entries.last.flatMap(\.turnID) else { return [] }
        // Walks back from the end and stops at the previous turn, so streamed thinking never rescans the thread.
        var steps: [ThinkingStep] = []
        for entry in runtime.entries.reversed() {
            guard let entryTurn = entry.turnID else { continue }
            guard entryTurn == turnID else { break }
            guard entry.kind == .reasoning, case .reasoning(let block) = entry.item.content, !block.text.isEmpty else { continue }
            steps.append(ThinkingStep(text: block.text, isStreaming: block.isStreaming))
        }
        return steps.reversed()
    }
}

/// Applies the scroll freeze to the rows from a leaf of its own: the flag lives on the
/// tracking object and only this modifier reads it, so its flips re-render nothing but
/// this node, never the timeline's body.
private struct ScrollFreeze: ViewModifier {
    let tracking: TimelineScrollTracking

    func body(content: Content) -> some View {
        content.allowsHitTesting(!tracking.isReaderScrolling)
    }
}
