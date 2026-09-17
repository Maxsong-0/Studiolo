import AppKit
import SwiftUI

struct SidebarView: View {
    @Environment(AppModel.self) private var model
    /// Shown from the chat's Threads button while the sidebar is hidden: the same list in a
    /// popover, without the window buttons' clearance or the window-dragging background, and
    /// picking a thread closes it.
    var inPopover = false
    var dismiss: (() -> Void)? = nil

    @State private var search = ""
    @State private var renaming: ChatThread?
    @State private var renameText = ""
    /// The thread whose delete popover is open, by id: the popover's binding compares two
    /// UUIDs and never reaches into a thread snapshot that may since have changed hands.
    @State private var pendingDeletionID: UUID?
    /// The thread whose archive question is open, by id, the same way.
    @State private var pendingArchiveID: UUID?
    /// A delete the popover confirmed, carried out once the popover has closed (see
    /// `confirmDeletion` and `deletePopover`).
    @State private var confirmedDeletion: ConfirmedDeletion?
    /// Whether a delete popover's content is on screen, from its appearance to its
    /// disappearance: a confirmed delete waits for it to go. One flag serves every row,
    /// since only one delete popover is ever open.
    @State private var isDeletePopoverShown = false
    /// Carries context-menu intents (rename, delete) outside any row's @State, so an AppKit
    /// menu's callbacks never retain a row's state storage. The menu posts a value-type
    /// request here (holding this box weakly); the list takes it up below.
    @State private var menuRequests = SidebarMenuRequests()
    /// The live reorder, exactly the queue's: the grabbed row follows the pointer, its
    /// helpers with it, and neighbours slide across as its centre passes theirs.
    @State private var drag = RowDrag<UUID>()
    /// Every row's height by its item id. A thread's slot is its own row plus the helper
    /// rows under it, so a dragged row crosses a neighbour with helpers in one go. Kept out
    /// of observation like the frames below: every row reports its height as the list lays
    /// out, and only a drag ever reads them, so a report must not re-run the list.
    @State private var rowHeights = RowHeights()
    /// The list's frame in the window, for a settling row's glide: it is clipped to the
    /// list and heads for its edge when its new place is out of view. Kept out of
    /// observation the way a row's frame is: resizing updates it, only a settle reads it.
    @State private var listFrame = FrameHolder()
    /// Every open thread row's frame in the list, by thread id, for the one drag gesture
    /// below to find the row under the pointer at the grab. Kept out of observation like
    /// the heights: the rows report as they lay out, only a grab reads them.
    @State private var rowFrames = RowFrames()
    /// The grabbed row's peers and their slot heights, taken once at the grab and again
    /// after each swap, so a pointer move never rebuilds the list to find them.
    @State private var dragPeers: DragPeers?
    /// The built list, kept while its inputs stand (see `SidebarItemCache`): a render
    /// that changed no thread, project or setting hands back the same items.
    @State private var itemCache = SidebarItemCache()
    /// While the list scrolls its rows take no hover and no clicks, the way the timeline's
    /// do: a click cannot land on a moving row anyway, and skipping the hover states keeps
    /// a scroll at one layout pass per frame.
    @State private var isListScrolling = false
    /// Settles `isListScrolling` back to false 150 ms after the last scroll movement, so
    /// wheel ticks (which report no scroll phase) re-arm the freeze instead of flickering it.
    @State private var scrollSettle: Task<Void, Never>?

    private var query: String {
        search.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The list's own coordinate space: the rows report their frames in it and the drag
    /// gesture reads its locations in it, whatever the scroll position.
    private static let listSpace = "sidebar-list"

    var body: some View {
        // Where every thread sits, worked out once per pass: the grouping and the list below
        // are rebuilt only when this changes, not on every field change of every thread.
        let placements = threadPlacements
        let helpers = helpersByParent(placements: placements)
        VStack(alignment: .leading, spacing: 0) {
            // Clears the native window buttons that float over the sidebar's top corner.
            if !inPopover {
                Color.clear
                    .frame(height: Chrome.trafficLightDiameter)
                    .padding(.top, Chrome.trafficLightTop)
            }

            HStack(spacing: 6) {
                SidebarSearchField(
                    text: $search,
                    prompt: "Search threads",
                    onSubmit: {
                        if let first = searchResults(helpers: helpers).first?.threads.first {
                            model.selectedThreadID = first.id
                        }
                    },
                    onMove: { moveSearchSelection(by: $0, helpers: helpers) }
                )
                ActivityViewToggle(isOn: Binding(
                    get: { model.settings.sidebarActivityView },
                    set: { model.settings.sidebarActivityView = $0 }
                ))
            }
            .padding(.horizontal, Chrome.listInset)
            .padding(.top, inPopover ? 12 : 14)

            ScrollView(.vertical) {
                LazyVStack(alignment: .leading, spacing: 1) {
                    if query.isEmpty {
                        // One list for both layouts: a thread keeps its row when the layout changes, so the
                        // row grows or shrinks and slides to its new place instead of being replaced.
                        ForEach(listItems(placements: placements, helpers: helpers)) { item in
                            itemView(item)
                                .transition(.sidebarRow)
                        }
                    } else {
                        searchList(helpers: helpers)
                    }
                }
                .allowsHitTesting(!isListScrolling)
                // Adding or deleting a thread opens and closes its space with the same motion as every other row.
                .animation(Chrome.panelSlide, value: model.threads.count)
                .animation(Chrome.panelSlide, value: model.settings.settledCollapsed)
                .padding(.horizontal, Chrome.listInset)
                .padding(.top, 12)
                .padding(.bottom, 8)
                .coordinateSpace(name: Self.listSpace)
                // One drag gesture for the whole list, ahead of every row's own click: the
                // row under the pointer at the grab is the one that moves (a settled or
                // helper row reports no frame, so a drag from one goes nowhere), and a
                // plain click still reaches the row. Off while searching: nothing to reorder.
                .highPriorityGesture(
                    DragGesture(minimumDistance: 4, coordinateSpace: .named(Self.listSpace))
                        .onChanged { value in
                            guard let id = drag.id ?? rowFrames.thread(at: value.startLocation) else { return }
                            dragChanged(id, translation: value.translation.height)
                        }
                        .onEnded { _ in dragEnded() },
                    isEnabled: query.isEmpty
                )
            }
            .scrollIndicators(.never)
            .onGeometryChange(for: CGRect.self, of: Self.windowFrame) { listFrame.note($0) }
            .onScrollPhaseChange { _, phase in
                if phase == .idle {
                    scrollSettle?.cancel()
                    isListScrolling = false
                } else {
                    noteListScroll()
                }
            }
            .onScrollGeometryChange(for: CGFloat.self, of: { $0.contentOffset.y }) { old, new in
                if old != new { noteListScroll() }
            }

            VStack(alignment: .leading, spacing: 1) {
                SidebarRow(title: "Add project", action: { addProject() }) {
                    SidebarIconBadge { SidebarSymbol("plus") }
                }
                SidebarRow(title: "Settings", action: { WindowManager.shared.showSettings() }) {
                    SidebarIconBadge { SidebarSymbol("gear", scale: 1.15) }
                        .overlay(alignment: .topTrailing) {
                            if UpdateChecker.shared.updateAvailable {
                                UpdateAvailableDot()
                            }
                        }
                }
            }
            .padding(.horizontal, Chrome.listInset)
            .padding(.bottom, 12)
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .background { if !inPopover { WindowDragArea() } }
        // Read only in the popover: the list itself has no reason to re-render on selection.
        .onChange(of: inPopover ? model.selectedThreadID : nil) { if inPopover { dismiss?() } }
        .onChange(of: menuRequests.rename) { _, thread in
            guard let thread else { return }
            menuRequests.rename = nil
            renameText = thread.title
            renaming = thread
        }
        .onChange(of: menuRequests.delete) { _, thread in
            guard let thread else { return }
            menuRequests.delete = nil
            if model.settings.confirmBeforeDeleting {
                pendingDeletionID = thread.id
            } else {
                model.delete(thread.id)
            }
        }
        // The shortcut's question lands on the thread's own row.
        .onChange(of: model.archiveRequest) { _, request in
            guard let request, model.thread(request.threadID) != nil else { return }
            model.archiveRequest = nil
            pendingArchiveID = request.threadID
        }
    }

    /// The delete question, asked in a popover on the row itself rather than a sheet over
    /// the window. Clicking away or pressing Escape keeps the thread.
    ///
    /// The binding is built on the thread's id alone, and the confirmed delete does not run
    /// while the popover is up: the item closes the popover and the delete waits for its
    /// content to have gone (see `confirmDeletion`). Before, the delete ran a turn after
    /// the close, which could be while the popover was still going: it removed the row the
    /// popover hangs on, and in the Threads popover the whole list with it (the delete
    /// selects a neighbour, which dismisses the list); the popover's dismissal then read
    /// this binding on a view already torn down and crashed in the getter.
    private func deletePopover<Row: View>(for thread: ChatThread, on row: Row) -> some View {
        let threadID = thread.id
        return row.popover(
            isPresented: Binding(
                get: { pendingDeletionID == threadID },
                set: { if !$0, pendingDeletionID == threadID { pendingDeletionID = nil } }
            ),
            arrowEdge: .trailing
        ) {
            DeleteThreadPopover(thread: model.thread(threadID) ?? thread) { removeWorktree in
                confirmDeletion(of: threadID, removeWorktree: removeWorktree)
            }
            .onAppear { isDeletePopoverShown = true }
            .onDisappear {
                isDeletePopoverShown = false
                carryOutConfirmedDeletion()
            }
        }
    }

    /// The rename field, asked in a popover on the row itself like the delete question.
    /// Clicking away keeps the thread's title.
    private func renamePopover<Row: View>(for thread: ChatThread, on row: Row) -> some View {
        let threadID = thread.id
        return row.popover(
            isPresented: Binding(
                get: { renaming?.id == threadID },
                set: { if !$0 { renaming = nil } }
            ),
            arrowEdge: .bottom
        ) {
            RenameThreadPopover(title: renameText, onRename: { model.rename(threadID, to: $0); renaming = nil }, onCancel: { renaming = nil })
                .presentedChrome()
        }
    }

    /// The popover's answer. Its item has closed the popover (through the binding above)
    /// and calls here a turn later, so the popover may be on its way out or already gone:
    /// the delete runs now if it is gone, and when it goes otherwise. Clicking away
    /// instead confirms nothing.
    private func confirmDeletion(of id: UUID, removeWorktree: Bool) {
        confirmedDeletion = ConfirmedDeletion(threadID: id, removeWorktree: removeWorktree)
        pendingDeletionID = nil
        pendingArchiveID = nil
        if !isDeletePopoverShown { carryOutConfirmedDeletion() }
    }

    /// The shortcut's question on a row: archive, which Return answers, or delete. The
    /// delete goes the delete popover's way (see `confirmDeletion`), so the row the
    /// question hangs on is gone only once the popover is.
    private func archivePopover<Row: View>(for thread: ChatThread, on row: Row) -> some View {
        let threadID = thread.id
        return row.popover(
            isPresented: Binding(
                get: { pendingArchiveID == threadID },
                set: { if !$0, pendingArchiveID == threadID { pendingArchiveID = nil } }
            ),
            arrowEdge: .trailing
        ) {
            ArchiveThreadPopover(
                thread: model.thread(threadID) ?? thread,
                onArchive: {
                    pendingArchiveID = nil
                    withAnimation(Chrome.panelSlide) { model.archive(threadID) }
                },
                onDelete: { confirmDeletion(of: threadID, removeWorktree: false) }
            )
            .onAppear { isDeletePopoverShown = true }
            .onDisappear {
                isDeletePopoverShown = false
                carryOutConfirmedDeletion()
            }
        }
    }



    /// Runs the delete the popover confirmed, once the popover has closed and a turn
    /// later: the model changes outside the view update that removed the presentation,
    /// and the rows it takes away are no longer anchoring anything. A popover closed by a
    /// click away has nothing confirmed and does nothing here.
    private func carryOutConfirmedDeletion() {
        guard let confirmed = confirmedDeletion else { return }
        confirmedDeletion = nil
        let model = model
        Task { @MainActor in
            model.delete(confirmed.threadID, removeWorktree: confirmed.removeWorktree)
        }
    }

    private struct ConfirmedDeletion {
        let threadID: UUID
        let removeWorktree: Bool
    }

    @ViewBuilder
    private func itemView(_ item: SidebarItem) -> some View {
        switch item.kind {
        case .gap(let height):
            Color.clear.frame(height: height)
        case .project(let project, let count):
            ProjectRow(snapshot: project, count: count)
        case .header(let title, let isFirst):
            ActivityHeader(title: title, isFirst: isFirst)
        case .settledHeader(let count, let isFirst):
            SettledHeader(count: count, isFirst: isFirst)
        case .thread(let thread, let projectName, let peers, let hasHelpers):
            reorderableRow(thread, projectName: projectName, peers: peers, hasHelpers: hasHelpers)
                .onGeometryChange(for: CGRect.self, of: { $0.frame(in: .named(Self.listSpace)) }) { frame in
                    rowHeights.note(item.id, frame.height)
                    // A settled row keeps its place among the settled; there is nothing to reorder.
                    if !thread.isSettled { rowFrames.note(thread.id, frame) }
                }
                // A row that has left the list (scrolled out, settled, deleted) is not there to grab.
                .onDisappear { rowFrames.forget(thread.id) }
        case .helper(let thread, let isLast):
            archivePopover(for: thread, on: deletePopover(for: thread, on: SidebarHelperRow(
                snapshot: thread,
                isLast: isLast,
                menuRequests: menuRequests,
                onFold: { fold(thread.parentThreadID) },
                onRename: {
                    // The live record: the item's snapshot does not follow a title change.
                    let live = model.thread(thread.id) ?? thread
                    renameText = live.title
                    renaming = live
                },
                onDelete: {
                    if model.settings.confirmBeforeDeleting {
                        pendingDeletionID = thread.id
                    } else {
                        model.delete(thread.id)
                    }
                }
            ).equatable()))
            .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { rowHeights.note(item.id, $0) }
            .modifier(RidesWithDraggedParent(parentID: thread.parentThreadID, drag: drag))
        case .helperStub(let parent, let helpers):
            HelperStubRow(parentSnapshot: parent, helperSnapshots: helpers) { fold(parent.id) }
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { rowHeights.note(item.id, $0) }
                .modifier(RidesWithDraggedParent(parentID: parent.id, drag: drag))
        }
    }

    /// Where every thread sits in the list (see `ThreadPlacement`): the key the grouping and
    /// the list below are kept under. Kept while a cheap fingerprint of the threads stands,
    /// so an unrelated re-render (selection, hover) does not rebuild placements for
    /// hundreds of threads.
    @State private var placementCache = PlacementCache()
    private var threadPlacements: [ThreadPlacement] {
        placementCache.placements(for: model.threads)
    }

    /// A scroll movement (re)arms the list's scroll freeze: rows take no hover or clicks
    /// for 150 ms after the last movement, covering wheel ticks, which report no phase.
    private func noteListScroll() {
        isListScrolling = true
        scrollSettle?.cancel()
        scrollSettle = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }
            isListScrolling = false
        }
    }

    /// Every helper by the thread it hangs under, worked out once for the whole list. Asking
    /// the model per row walked the library once per row, so a long list did that work for
    /// every row in it; the rows are handed their own helpers instead. Kept while the
    /// placements stand: a render that moved no thread reuses the last grouping.
    private func helpersByParent(placements: [ThreadPlacement]) -> [UUID: [ChatThread]] {
        itemCache.helpers(for: placements) {
            var grouped: [UUID: [ChatThread]] = [:]
            // Heads count while they are still in the lead's panel too (see `AppModel.helpers(of:)`).
            for thread in model.threads where !thread.isArchived && (!thread.isInPanel || thread.isHydraHead) {
                guard let parentID = thread.parentThreadID else { continue }
                grouped[parentID, default: []].append(thread)
            }
            // Newest first, like the threads around them (see `AppModel.helpers(of:)`).
            return grouped.mapValues { $0.sorted { $0.createdAt > $1.createdAt } }
        }
    }

    /// The list in the current layout, built only when something that places a row has
    /// changed: where the threads sit (`ThreadPlacement`), the projects, which threads need
    /// attention (they head the activity layout), the layout flag or the settled fold. A
    /// title or a head's progress note reaches its row through the model's per-thread cell
    /// (see `SidebarThreadRow.thread`) without the list being built again.
    private func listItems(placements: [ThreadPlacement], helpers: [UUID: [ChatThread]]) -> [SidebarItem] {
        let activity = model.settings.sidebarActivityView
        let key = SidebarItemCache.Key(
            placements: placements,
            projects: model.projects,
            attention: activity ? Set(model.threads.filter { needsAttention($0) }.map(\.id)) : [],
            activityView: activity,
            settledCollapsed: model.settings.settledCollapsed
        )
        return itemCache.items(for: key) {
            activity ? activityItems(helpers: helpers) : projectItems(helpers: helpers)
        }
    }

    /// The helpers under a thread: one small row each, or a single line naming them while
    /// they are folded away.
    private func helperItems(under thread: ChatThread, helpers: [ChatThread]) -> [SidebarItem] {
        // A settled thread is one small line; what it spawned comes back when it reopens.
        guard !thread.isSettled, !helpers.isEmpty else { return [] }
        if thread.foldsHelpers {
            return [SidebarItem(id: "helpers-\(thread.id)", kind: .helperStub(parent: thread, helpers: helpers))]
        }
        return helpers.map { helper in
            SidebarItem(id: helper.id.uuidString, kind: .helper(helper, isLast: helper.id == helpers.last?.id))
        }
    }

    private func fold(_ parentID: UUID?) {
        guard let parentID else { return }
        withAnimation(Chrome.panelSlide) { model.toggleHelpersFold(parentID) }
    }

    private nonisolated static func windowFrame(_ proxy: GeometryProxy) -> CGRect {
        proxy.frame(in: .named(GenieAnimator.coordinateSpace))
    }

    // MARK: Adding projects

    /// New projects join from here: a folder picker, one project per folder, then a
    /// thread in the last one added. Adding persists through the model.
    private func addProject() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.prompt = "Add project"
        panel.message = "Choose a folder for your agents to work in."
        guard panel.runModal() == .OK else { return }
        var added: Project?
        for url in panel.urls { added = model.addProject(at: url) }
        if let added { model.newThread(in: added) }
    }

    // MARK: Project layout

    private func projectItems(helpers: [UUID: [ChatThread]]) -> [SidebarItem] {
        var items: [SidebarItem] = []
        for (index, project) in model.projects.enumerated() {
            if index > 0 {
                items.append(SidebarItem(id: "gap-\(project.id)", kind: .gap(Chrome.groupGap)))
            }
            let projectThreads = model.threads(in: project)
            // The count a folded project shows: its threads and everything under them.
            let count = projectThreads.reduce(0) { $0 + 1 + (helpers[$1.id]?.count ?? 0) }
            items.append(SidebarItem(id: "project-\(project.id)", kind: .project(project, count: count)))
            guard project.isExpanded else { continue }
            // The settled threads close the project's list (see `threads(in:)`), a small
            // step below the ones still open, under a header that folds them away.
            // Pinned threads lead each project's open list; settled ones never do.
            let collapsed = model.settings.settledCollapsed
            var hasOpen = false
            var reachedSettled = false
            var openThreads = projectThreads.filter { !$0.isSettled }
            openThreads = openThreads.filter(\.isPinned) + openThreads.filter { !$0.isPinned }
            let orderedThreads = openThreads + projectThreads.filter(\.isSettled)
            for thread in orderedThreads {
                if thread.isSettled, !reachedSettled {
                    reachedSettled = true
                    if hasOpen {
                        items.append(SidebarItem(id: "settled-gap-\(project.id)", kind: .gap(ThreadRowMetrics.settledGap)))
                    }
                    let settledCount = orderedThreads.count(where: \.isSettled)
                    if settledCount > 0 {
                        items.append(SidebarItem(id: "settled-header-\(project.id)", kind: .settledHeader(count: settledCount, isFirst: false)))
                    }
                    if collapsed { break }
                }
                hasOpen = hasOpen || !thread.isSettled
                let own = helpers[thread.id] ?? []
                items.append(SidebarItem(
                    id: SidebarItem.id(for: thread),
                    kind: .thread(thread, projectName: nil, peers: nil, hasHelpers: !thread.isSettled && !own.isEmpty)
                ))
                items.append(contentsOf: helperItems(under: thread, helpers: own))
            }
        }
        return items
    }

    // MARK: Activity layout

    private func activityItems(helpers: [UUID: [ChatThread]]) -> [SidebarItem] {
        let shown = model.threads.filter { !$0.isArchived && !$0.isInPanel }
        // A helper whose parent is here sits under it (see helperItems); one whose parent is
        // gone or archived stands on its own.
        let shownIDs = Set(shown.map(\.id))
        let active = shown.filter { $0.parentThreadID.map { !shownIDs.contains($0) } ?? true }
        // A settled thread stays among the settled whatever it is up to.
        let attention = Self.placed(active.filter { needsAttention($0) && !$0.isSettled })
        let rest = active.filter { !needsAttention($0) || $0.isSettled }
        var items: [SidebarItem] = []
        if !attention.isEmpty {
            items.append(SidebarItem(id: "attention", kind: .header("Needs attention", isFirst: true)))
            items.append(contentsOf: activityThreadItems(attention, helpers: helpers))
        }
        // Open pinned threads sit above the day groups, in the activity sort's order.
        let open = rest.filter { !$0.isSettled }
        let pinned = Self.placed(open.filter(\.isPinned))
        if !pinned.isEmpty {
            items.append(SidebarItem(id: "day-Pinned", kind: .header("Pinned", isFirst: items.isEmpty)))
            items.append(contentsOf: activityThreadItems(pinned, helpers: helpers))
        }
        for group in Self.activityGroups(open.filter { !$0.isPinned }) {
            // Without an attention section the day header is the first row, so it takes the tighter top padding.
            items.append(SidebarItem(id: "day-\(group.title)", kind: .header(group.title, isFirst: items.isEmpty)))
            items.append(contentsOf: activityThreadItems(group.threads, helpers: helpers))
        }
        // The settled threads sit under everything, whatever day they were last active,
        // latest settled first, under a header that folds them away. They keep their
        // place: no dragging among them.
        let settled = rest.filter(\.isSettled).sorted(by: AppModel.settledFirst)
        if !settled.isEmpty {
            items.append(SidebarItem(id: "settled", kind: .settledHeader(count: settled.count, isFirst: items.isEmpty)))
            if !model.settings.settledCollapsed {
                for thread in settled {
                    items.append(SidebarItem(
                        id: SidebarItem.id(for: thread),
                        kind: .thread(thread, projectName: model.project(thread.projectID)?.name ?? "", peers: nil, hasHelpers: false)
                    ))
                }
            }
        }
        return items
    }

    /// Threads of one activity group, each with the helpers under it. A thread can be dragged
    /// to another place within its own group; its helpers follow it.
    private func activityThreadItems(_ threads: [ChatThread], helpers: [UUID: [ChatThread]]) -> [SidebarItem] {
        let peers = threads.map(\.id)
        return threads.flatMap { thread in
            let own = helpers[thread.id] ?? []
            return [SidebarItem(
                id: thread.id.uuidString,
                kind: .thread(thread, projectName: model.project(thread.projectID)?.name ?? "", peers: peers, hasHelpers: !thread.isSettled && !own.isEmpty)
            )] + helperItems(under: thread, helpers: own)
        }
    }

    private func needsAttention(_ thread: ChatThread) -> Bool {
        guard let runtime = model.existingRuntime(for: thread.id) else { return false }
        return !runtime.approvals.isEmpty || !runtime.questions.isEmpty
    }

    private struct ActivityGroup {
        let title: String
        var threads: [ChatThread]
    }

    /// Threads grouped under Today, Yesterday, a weekday within the week, then a date.
    private static func activityGroups(_ threads: [ChatThread]) -> [ActivityGroup] {
        let calendar = Calendar.current
        var groups: [ActivityGroup] = []
        for thread in threads.sorted(by: { $0.updatedAt > $1.updatedAt }) {
            let title = dayTitle(for: thread.updatedAt, calendar: calendar)
            if groups.last?.title == title {
                groups[groups.count - 1].threads.append(thread)
            } else {
                groups.append(ActivityGroup(title: title, threads: [thread]))
            }
        }
        return groups.map { ActivityGroup(title: $0.title, threads: placed($0.threads)) }
    }

    /// Newest first, except threads dragged into place, which keep that place for the rest of their day.
    private static func placed(_ threads: [ChatThread]) -> [ChatThread] {
        let calendar = Calendar.current
        func order(_ thread: ChatThread) -> Double? {
            guard let order = thread.activityOrder, let day = thread.activityOrderDay,
                  calendar.isDate(day, inSameDayAs: thread.updatedAt) else { return nil }
            return order
        }
        return threads.sorted { lhs, rhs in
            switch (order(lhs), order(rhs)) {
            case let (left?, right?) where left != right: return left < right
            // Threads that became active after a reorder have no place yet and stay on top.
            case (nil, .some): return true
            case (.some, nil): return false
            default: return lhs.updatedAt > rhs.updatedAt
            }
        }
    }

    private static func dayTitle(for date: Date, calendar: Calendar) -> String {
        if calendar.isDateInToday(date) { return "Today" }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: .now)).day ?? 0
        if days < 7 { return date.formatted(.dateTime.weekday(.wide)) }
        if calendar.isDate(date, equalTo: .now, toGranularity: .year) {
            return date.formatted(.dateTime.month(.wide).day())
        }
        return date.formatted(.dateTime.month(.wide).day().year())
    }

    // MARK: Search

    @ViewBuilder
    private func searchList(helpers: [UUID: [ChatThread]]) -> some View {
        let results = searchResults(helpers: helpers)
        if results.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("No results")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText.opacity(0.92))
                Text("Try a thread title or a project name.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
            .padding(.horizontal, 8)
            .padding(.top, 14)
        } else {
            ForEach(Array(results.enumerated()), id: \.element.project.id) { index, result in
                if index > 0 {
                    Color.clear.frame(height: Chrome.groupGap)
                }
                ProjectRow(snapshot: result.project, count: result.count, togglesExpansion: false)
                // Keyed like the main list, so a thread settled from here changes rows too.
                ForEach(result.threads, id: \.sidebarItemID) { thread in
                    threadRow(thread, projectName: nil, hasHelpers: !thread.isSettled && !(helpers[thread.id] ?? []).isEmpty)
                }
            }
        }
    }

    /// Up and down in the search field walk the results without leaving the keyboard; the
    /// row highlights exactly as a clicked one does.
    private func moveSearchSelection(by offset: Int, helpers: [UUID: [ChatThread]]) {
        guard !query.isEmpty else { return }
        let results = searchResults(helpers: helpers).flatMap(\.threads)
        guard !results.isEmpty else { return }
        guard let current = model.selectedThreadID,
              let index = results.firstIndex(where: { $0.id == current }) else {
            model.selectedThreadID = (offset < 0 ? results.last : results.first)?.id
            return
        }
        let next = min(max(index + offset, 0), results.count - 1)
        model.selectedThreadID = results[next].id
    }

    /// The projects and threads matching the query, kept for the query and library they
    /// were found in: the field's submit, arrows and list all ask, and only a keystroke or
    /// a thread change looks again. Titles are matched lowercased, from a table the cache
    /// keeps per thread, so a keystroke lowercases the query once and nothing else.
    private func searchResults(helpers: [UUID: [ChatThread]]) -> [SidebarSearchResult] {
        let key = SidebarItemCache.SearchKey(threads: model.threads, projects: model.projects, query: query)
        return itemCache.search(for: key) { titles in
            let needle = query.lowercased()
            return model.projects.compactMap { project in
                let all = model.threads(in: project).flatMap { [$0] + (helpers[$0.id] ?? []) }
                let threads = project.name.lowercased().contains(needle)
                    ? all
                    : all.filter { titles[$0.id]?.contains(needle) ?? $0.title.lowercased().contains(needle) }
                return threads.isEmpty ? nil : SidebarSearchResult(project: project, threads: threads, count: all.count)
            }
        }
    }

    // MARK: Rows

    /// Drag a row up or down the list and it moves live: within its project in the project
    /// layout, within its group in the activity layout. The list's one gesture drives it
    /// (see `body`), ahead of the row's own click, so a drag never selects the thread on
    /// release; a plain click still does. The row only lifts and follows here.
    private func reorderableRow(_ thread: ChatThread, projectName: String?, peers: [UUID]?, hasHelpers: Bool) -> some View {
        let isDragged = drag.id == thread.id
        return threadRow(thread, projectName: projectName, hasHelpers: hasHelpers, isDragged: isDragged)
            .offset(y: isDragged ? drag.visualOffset : 0)
            .zIndex(isDragged ? 1 : 0)
    }

    // MARK: Reorder

    /// Neighbours sliding out of the grabbed row's way.
    private static let slide = Animation.spring(response: 0.28, dampingFraction: 0.82)

    /// The pointer has moved `translation` since the grab. The grabbed row follows it
    /// exactly and `RowDrag` swaps it past every neighbour whose centre it has crossed.
    private func dragChanged(_ id: UUID, translation: CGFloat) {
        if drag.id != id {
            drag = RowDrag(id: id)
            // Taken once at the grab: the peers and their slots stand until a swap.
            dragPeers = DragPeers(peers(of: id), heights: slotHeights)
            NSCursor.closedHand.push()
        }
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) { drag.translation = translation }

        guard let snapshot = dragPeers else { return }
        let (order, group) = (snapshot.order, snapshot.group)
        let moved = drag.settle(order: order, heights: snapshot.heights, fallbackHeight: Chrome.rowHeight + 1) { neighbour, placeAfter in
            // The neighbour slides and the grabbed row's slot moves in the same animation
            // as its compensation, so it stays put under the pointer while the list flows
            // around it.
            withAnimation(Self.slide) {
                if let group {
                    model.moveInActivity(id, to: neighbour, placeAfter: placeAfter, among: group)
                } else {
                    model.moveThread(id, to: neighbour, placeAfter: placeAfter)
                }
            }
        }
        if moved {
            // The row's peers as the list stands now, not as it stood at the grab: every
            // swap reorders them. The slots are by id and stand.
            dragPeers = DragPeers(peers(of: id), heights: slotHeights)
            NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
        }
    }

    private func dragEnded() {
        guard drag.id != nil else { return }
        NSCursor.pop()
        dragPeers = nil
        // The offset animates from wherever the pointer let go to the row's slot, so the
        // row settles instead of snapping.
        withAnimation(.spring(response: 0.32, dampingFraction: 0.8)) { drag = RowDrag() }
    }

    /// The threads a row can be reordered among, in their current order: its activity group
    /// (returned as `group` too, for the move) or its project's top-level threads.
    private func peers(of id: UUID) -> (order: [UUID], group: [UUID]?) {
        if model.settings.sidebarActivityView {
            let placements = threadPlacements
            for item in listItems(placements: placements, helpers: helpersByParent(placements: placements)) {
                if case .thread(let thread, _, let peers, _) = item.kind, thread.id == id, let peers {
                    return (peers, peers)
                }
            }
            return ([], nil)
        }
        guard let thread = model.thread(id), let project = model.project(thread.projectID) else { return ([], nil) }
        return (model.threads(in: project).filter { !$0.isSettled }.map(\.id), nil)
    }

    /// Each peer's slot: its row, the helper rows or folded line under it, and the list's
    /// spacing after each.
    private func slotHeights(for order: [UUID]) -> [UUID: CGFloat] {
        let grouped = helpersByParent(placements: threadPlacements)
        var heights: [UUID: CGFloat] = [:]
        for id in order {
            var height = (rowHeights.values[id.uuidString] ?? Chrome.rowHeight) + 1
            let helpers = grouped[id] ?? []
            if !helpers.isEmpty {
                if model.thread(id)?.foldsHelpers == true {
                    height += (rowHeights.values["helpers-\(id)"] ?? ThreadRowMetrics.helperHeight) + 1
                } else {
                    for helper in helpers {
                        height += (rowHeights.values[helper.id.uuidString] ?? ThreadRowMetrics.helperHeight) + 1
                    }
                }
            }
            heights[id] = height
        }
        return heights
    }

    /// What a drag reuses between pointer moves: the grabbed row's peers in their order,
    /// the group they form in the activity layout, and each peer's slot height.
    private struct DragPeers {
        let order: [UUID]
        let group: [UUID]?
        let heights: [UUID: CGFloat]

        init(_ peers: (order: [UUID], group: [UUID]?), heights: ([UUID]) -> [UUID: CGFloat]) {
            order = peers.order
            group = peers.group
            self.heights = heights(peers.order)
        }
    }

    private func threadRow(_ thread: ChatThread, projectName: String?, hasHelpers: Bool, isDragged: Bool = false) -> some View {
        // A thread with helpers under it gets the fold button; a settled one shows none.
        // `.equatable()` skips the parent's re-evaluation for rows whose value inputs did
        // not change; the row still observes the model's per-thread cells inside its body.
        return archivePopover(for: thread, on: renamePopover(for: thread, on: deletePopover(for: thread, on: SidebarThreadRow(
            snapshot: thread,
            projectName: projectName,
            menuRequests: menuRequests,
            isDragged: isDragged,
            // The popover is another window: no glide can cross into the main one from it.
            listFrame: inPopover ? nil : listFrame,
            onToggleFold: hasHelpers ? { fold(thread.id) } : nil,
            onRename: {
                // The live record: the item's snapshot does not follow a title change.
                let live = model.thread(thread.id) ?? thread
                renameText = live.title
                renaming = live
            },
            onDelete: {
                if model.settings.confirmBeforeDeleting {
                    pendingDeletionID = thread.id
                } else {
                    model.delete(thread.id)
                }
            }
        ).equatable())))
    }
}

/// The shortcut's question: Archive on the right, lit and answered by Return, Delete on
/// the left. A click away asks nothing.
private struct ArchiveThreadPopover: View {
    let thread: ChatThread
    let onArchive: () -> Void
    let onDelete: () -> Void

    var body: some View {
        PopoverMenu {
            PopoverSectionHeader("Archive this thread?")
            PopoverNote("“\(thread.title)” leaves the list and stays in Archived. Delete removes it and its history; files in your project stay as they are.")
            HStack(spacing: 8) {
                Button("Delete", role: .destructive, action: onDelete)
                    .buttonStyle(.glass)
                    .tint(.red)
                Spacer(minLength: 8)
                Button("Archive", action: onArchive)
                    .buttonStyle(.glassProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .controlSize(.small)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
        }
        .frame(width: 300)
    }
}

/// What the delete popover asks: the thread by name, what stays, and the destructive choice
/// as a red row like the ellipsis menu's, with a second one for a thread's worktree.
private struct DeleteThreadPopover: View {
    let thread: ChatThread
    let onDelete: (_ removeWorktree: Bool) -> Void

    var body: some View {
        PopoverMenu {
            PopoverSectionHeader("Delete this thread?")
            PopoverNote("“\(thread.title)” and its history will be removed. Files in your project stay as they are.")
            PopoverDivider()
            PopoverItem("Delete thread", symbol: "trash", isDestructive: true) { onDelete(false) }
            if thread.worktreePath != nil {
                PopoverItem("Delete thread and worktree", symbol: "trash", isDestructive: true) { onDelete(true) }
            }
        }
        .frame(width: 280)
    }
}

/// What the rename popover asks: the title field and the choice, on the row itself
/// like the delete question. Clicking away keeps the thread's title.
private struct RenameThreadPopover: View {
    let title: String
    let onRename: (String) -> Void
    let onCancel: () -> Void

    @State private var text: String
    @FocusState private var isFocused: Bool

    init(title: String, onRename: @escaping (String) -> Void, onCancel: @escaping () -> Void) {
        self.title = title
        self.onRename = onRename
        self.onCancel = onCancel
        _text = State(initialValue: title)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Rename thread")
                .font(.system(size: 13, weight: .semibold))
            TextField("Title", text: $text)
                .textFieldStyle(.roundedBorder)
                .focused($isFocused)
                .onSubmit { onRename(text) }
            HStack {
                Spacer()
                Button("Cancel", action: onCancel)
                    .buttonStyle(.bordered)
                Button("Rename") { onRename(text) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .controlSize(.regular)
        }
        .padding(14)
        .frame(width: 300)
        .onAppear { isFocused = true }
    }
}

/// One entry of the sidebar list. Thread entries use the thread's id in both layouts, and a
/// settled thread's row is an entry of its own: the open row leaves and the settled one
/// arrives, with a ghost gliding between them, instead of one row morphing on the spot.
private struct SidebarItem: Identifiable {
    enum Kind {
        case gap(CGFloat)
        /// A project, with what a folded one shows: its threads and their helpers.
        case project(Project, count: Int)
        case header(String, isFirst: Bool)
        /// The Settled section's header: the title, how many settled threads it holds,
        /// and whether they are folded away. The collapsed flag lives in settings.
        case settledHeader(count: Int, isFirst: Bool)
        /// A thread, with its project's name in the activity layout, the threads it can be
        /// reordered among, and whether it has helpers to fold away.
        case thread(ChatThread, projectName: String?, peers: [UUID]?, hasHelpers: Bool)
        /// A helper under its parent thread; the last one ends the connector.
        case helper(ChatThread, isLast: Bool)
        /// A parent's folded helpers, as one line that unfolds them.
        case helperStub(parent: ChatThread, helpers: [ChatThread])
    }

    let id: String
    let kind: Kind

    static func id(for thread: ChatThread) -> String {
        thread.sidebarItemID
    }
}

private extension ChatThread {
    var sidebarItemID: String {
        isSettled ? "settled-\(id.uuidString)" : id.uuidString
    }
}

private extension AnyTransition {
    /// A row leaves at once and arrives once its neighbours have mostly made room, so rows moving past
    /// each other never draw over one another. These two fades are deliberately quick and
    /// not the settle flight: on a settle or reopen the ghost is the motion, and the
    /// departing row has to be gone under it at once rather than linger as a fading
    /// remnant while the list closes up.
    static var sidebarRow: AnyTransition {
        .asymmetric(
            insertion: .opacity.animation(.easeIn(duration: 0.18).delay(0.12)),
            removal: .opacity.animation(.easeOut(duration: 0.06))
        )
    }
}

private struct ProjectRow: View {
    @Environment(AppModel.self) private var model
    /// The project as the list last built it; `project` below is the live record.
    let snapshot: Project
    /// The row reads its own project through the model's per-project cell, so a change to
    /// it re-renders this row alone and a change to any other project does not reach it.
    /// The snapshot is the fallback for a project that has just been removed.
    private var project: Project { model.project(snapshot.id) ?? snapshot }
    /// What a folded project shows: its threads and the helpers under them, counted by the
    /// list once for every row rather than here, per row, over the whole library.
    let count: Int
    var togglesExpansion = true

    @State private var isMenuPresented = false

    var body: some View {
        // The global switch in Settings activates every project at once; only a project
        // explicitly switched here keeps its own choice, and shows dimmed while off.
        let isActive = model.settings.isProjectActive(project.id)
        SidebarRow(
            title: project.name,
            accessoryWidth: 40,
            action: {
                guard togglesExpansion else { return }
                withAnimation(Chrome.panelSlide) {
                    model.updateProject(project.id) { $0.isExpanded.toggle() }
                }
            },
            icon: {
                SidebarIconBadge { SidebarSymbol("folder.fill") }
            },
            accessory: { hovering in
                if hovering || isMenuPresented {
                    HStack(spacing: 0) {
                        RowActionsButton(actions: actions(isActive: isActive), isPresented: $isMenuPresented)
                        Button {
                            model.newThread(in: project)
                        } label: {
                            RowAccessoryIcon("plus")
                        }
                        .buttonStyle(.plain)
                        .help("New thread in \(project.name)")
                    }
                } else if !project.isExpanded, count > 0 {
                    Text(verbatim: "\(count)")
                        .font(.system(size: 11).monospacedDigit())
                        .foregroundStyle(Chrome.secondaryText)
                        .padding(.trailing, 4)
                }
            }
        )
        .opacity(isActive ? 1 : 0.55)
        // Snapshots only: the menu builder must not capture the row, so the AppKit menu it
        // builds cannot pin the row's state storage (and its responder with it) after dismiss.
        .contextMenu { [project, model] in
            let projectSnapshot = project
            let modelSnapshot = model
            let activeSnapshot = modelSnapshot.settings.isProjectActive(projectSnapshot.id)
            let overrideSnapshot = modelSnapshot.settings.hasProjectActivationOverride(for: projectSnapshot.id)
            RowActionMenuButtons(actions: Self.contextMenuActions(
                project: projectSnapshot,
                isActive: activeSnapshot,
                hasOverride: overrideSnapshot,
                model: modelSnapshot
            ))
        }
    }

    /// The context menu's items, built from value snapshots with weak model captures: the
    /// AppKit menu outlives the right-click, so its callbacks must not retain the row.
    /// (The ellipsis popover keeps `actions(isActive:)`; it is not an AppKit menu.)
    private static func contextMenuActions(project: Project, isActive: Bool, hasOverride: Bool, model: AppModel) -> [RowAction] {
        let projectID = project.id
        var items = [
            RowAction(title: "New thread", symbol: "square.and.pencil") { [weak model, project] in
                guard let model else { return }
                model.newThread(in: project)
            },
            RowAction(title: "New thread in worktree", symbol: "square.stack.3d.up") { [weak model, project] in
                guard let model else { return }
                model.newThread(in: project, workspace: .worktree)
            },
            RowAction(title: "Reveal in Finder", symbol: "folder", startsGroup: true) { [weak model, path = project.path] in
                guard model != nil else { return }
                Workspace.revealInFinder(path)
            },
            RowAction(title: "Remove project", symbol: "trash", isDestructive: true, startsGroup: true) { [weak model, project] in
                guard let model else { return }
                model.removeProject(project)
            },
        ]
        items.append(RowAction(
            title: isActive ? "Deactivate project" : "Activate project",
            symbol: "power",
            startsGroup: true
        ) { [weak model, projectID, isActive] in
            guard let model else { return }
            model.settings.setProjectActive(!isActive, for: projectID)
        })
        if hasOverride {
            items.append(RowAction(title: "Follow global setting", symbol: "arrow.uturn.backward") { [weak model, projectID] in
                guard let model else { return }
                model.settings.clearProjectActivationOverride(for: projectID)
            })
        }
        return items
    }

    private func actions(isActive: Bool) -> [RowAction] {
        var items = [
            RowAction(title: "New thread", symbol: "square.and.pencil") { model.newThread(in: project) },
            RowAction(title: "New thread in worktree", symbol: "square.stack.3d.up") {
                model.newThread(in: project, workspace: .worktree)
            },
            RowAction(title: "Reveal in Finder", symbol: "folder", startsGroup: true) { Workspace.revealInFinder(project.path) },
            RowAction(title: "Remove project", symbol: "trash", isDestructive: true, startsGroup: true) {
                model.removeProject(project)
            },
        ]
        items.append(RowAction(
            title: isActive ? "Deactivate project" : "Activate project",
            symbol: "power",
            startsGroup: true
        ) {
            model.settings.setProjectActive(!isActive, for: project.id)
        })
        if model.settings.hasProjectActivationOverride(for: project.id) {
            items.append(RowAction(title: "Follow global setting", symbol: "arrow.uturn.backward") {
                model.settings.clearProjectActivationOverride(for: project.id)
            })
        }
        return items
    }
}

private enum ThreadRowMetrics {
    static let detailedVerticalPadding: CGFloat = 7
    /// A two-line activity row: title, project line and their padding.
    static let detailedHeight: CGFloat = 48
    /// A helper's row under its parent: one small line, in either layout.
    static let helperHeight: CGFloat = 24
    /// A settled thread's row: one small grey line, in either layout.
    static let settledHeight: CGFloat = 24
    /// The step between a project's last open thread and its settled ones.
    static let settledGap: CGFloat = 4
    /// Where the check sits: at the front of a settled row, and beside the ellipsis at the
    /// trailing end of an open one. The settled title starts past the check.
    static let settledCheckInset: CGFloat = 6
    static let openCheckInset: CGFloat = 26
    /// Where the fold button sits on a thread with helpers: past the ellipsis and the check
    /// (or the archive button) at the trailing end.
    static let foldInset: CGFloat = 46
    static let settledTitleInset: CGFloat = 22
    /// The connector's column: the dotted line runs down it, under the parent's badge.
    static let connectorWidth: CGFloat = 28
    static let connectorLineX: CGFloat = 18
}

/// A helper under the thread it was spawned from: one small line, joined to its parent by a
/// dotted connector that runs down the rows and ends at the last. The connector folds the
/// helpers away; the folded line (HelperStubRow) unfolds them.
private struct SidebarHelperRow: View, Equatable {
    @Environment(AppModel.self) private var model
    /// The helper as the list last built it; `thread` below is the live record.
    let snapshot: ChatThread
    /// The row reads its own thread through the model's per-thread cell, so a change to it
    /// re-renders this row alone and a change to any other thread does not reach it. The
    /// snapshot is the fallback for a thread that has just been deleted.
    private var thread: ChatThread { model.thread(snapshot.id) ?? snapshot }
    /// The last helper under its parent: the connector ends at this row.
    let isLast: Bool
    /// Carries rename/delete intents out of the AppKit menu without retaining row state.
    /// Held strongly: the box is the list's, not the row's, so holding it pins no row
    /// state, and a weak one could go nil under a menu still on screen.
    let menuRequests: SidebarMenuRequests
    let onFold: () -> Void
    let onRename: () -> Void
    let onDelete: () -> Void

    @State private var isHovering = false
    @State private var isMenuPresented = false

    var body: some View {
        // Through the thread's own cell: a selection change re-renders the two rows it
        // touches, not every row in the list.
        let isSelected = model.isSelected(thread.id)
        let showsActions = isHovering || isMenuPresented
        // A helper goes out of sight the moment its parent's ghost takes off, so it never
        // lingers under the rows closing up.
        let isHidden = RowGlideAnimator.shared.isHiding(thread.parentThreadID ?? thread.id)
        let shape = RoundedRectangle(cornerRadius: Chrome.rowCornerRadius, style: .continuous)
        HStack(spacing: 0) {
            HelperConnector(endsHere: isLast, action: onFold)
                .frame(width: ThreadRowMetrics.connectorWidth, height: ThreadRowMetrics.helperHeight)
            Button {
                model.selectedThreadID = thread.id
            } label: {
                HStack(spacing: 6) {
                    // A head keeps its glyph, so the team reads at a glance under its lead.
                    if let head = thread.hydra {
                        HydraGlyph(persona: head.persona, size: 12, status: head.status)
                    }
                    Text(verbatim: thread.title)
                        .font(.system(size: 12, weight: isSelected || thread.hasUnread ? .medium : .regular))
                        .foregroundStyle(Chrome.primaryText.opacity(isSelected ? 0.96 : 0.8))
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 4)
                }
                .padding(.leading, 6)
                .padding(.trailing, Chrome.rowHorizontalPadding + (showsActions ? 26 : 18))
                .frame(height: ThreadRowMetrics.helperHeight)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background {
                    shape
                        .fill(isSelected ? Chrome.overlay(0.12) : (isHovering ? Chrome.overlay(0.06) : Color.clear))
                        .animation(Chrome.hover, value: isSelected)
                }
                .contentShape(shape)
            }
            .buttonStyle(.plain)
            .overlay(alignment: .trailing) {
                Group {
                    if showsActions {
                        RowActionsButton(actions: makeActions(), isPresented: $isMenuPresented)
                    } else {
                        ActivityStatus(thread: thread)
                    }
                }
                .padding(.trailing, 6)
            }
            .onHover { hovering in
                withAnimation(Chrome.hover) { isHovering = hovering }
                if hovering { model.warmDocuments([thread.id]) }
            }
            // Snapshots only: the menu builder must not capture the row, so the AppKit menu
            // it builds cannot pin the row's state storage (and its responder with it).
            .contextMenu { [thread, model, menuRequests] in
                let threadSnapshot = thread
                let modelSnapshot = model
                let requestsSnapshot = menuRequests
                let settleFirstSnapshot = !threadSnapshot.isHelper && modelSnapshot.settings.threadFinishAction == .settle
                RowActionMenuButtons(actions: ThreadActions.makeContextMenu(
                    model: modelSnapshot,
                    thread: threadSnapshot,
                    settleFirst: settleFirstSnapshot,
                    requests: requestsSnapshot
                ))
            }
            .accessibilityAddTraits(isSelected ? [.isSelected, .isButton] : .isButton)
        }
        // A helper goes out of sight the moment its parent's ghost takes off, so it never
        // lingers under the rows closing up.
        .opacity(isHidden ? 0 : 1)
    }

    /// The ellipsis popover's items. Kept on the instance: a popover is not an AppKit menu.

    static func == (lhs: SidebarHelperRow, rhs: SidebarHelperRow) -> Bool {
        lhs.snapshot.id == rhs.snapshot.id
            && lhs.snapshot.title == rhs.snapshot.title
            && lhs.snapshot.updatedAt == rhs.snapshot.updatedAt
            && lhs.snapshot.isPinned == rhs.snapshot.isPinned
            && lhs.snapshot.isSettled == rhs.snapshot.isSettled
            && lhs.isLast == rhs.isLast
    }

    private func makeActions() -> [RowAction] {
        ThreadActions.make(model: model, thread: thread, onRename: onRename, onDelete: onDelete)
    }
}

/// A parent's folded helpers as one small line: the connector's stub, a chevron and their
/// count, with a pulse when any of them is still working. Clicking it unfolds them.
private struct HelperStubRow: View {
    @Environment(AppModel.self) private var model
    /// The parent as the list last built it; `parent` below is the live record.
    let parentSnapshot: ChatThread
    /// Handed in by the list, which groups every helper once per pass; the body reads each
    /// one's live record through the model's per-thread cell.
    let helperSnapshots: [ChatThread]
    let action: () -> Void

    @State private var isHovering = false

    /// The row reads its parent through the model's per-thread cell, so a change to it
    /// re-renders this row alone and a change to any other thread does not reach it. The
    /// snapshot is the fallback for a thread that has just been deleted.
    private var parent: ChatThread { model.thread(parentSnapshot.id) ?? parentSnapshot }

    var body: some View {
        // Each helper's live record, read once per render through its own cell.
        let helpers = helperSnapshots.map { model.thread($0.id) ?? $0 }
        let count = helpers.count
        let working = helpers.contains { helper in
            let runtime = model.existingRuntime(for: helper.id)
            return runtime?.isRunning == true || runtime?.isHydraMerging == true
        }
        // A team of heads is called that; a mix, or merges alone, stays "helpers".
        let noun = helpers.allSatisfy(\.isHydraHead) ? "head" : "helper"
        // A helper goes out of sight the moment its parent's ghost takes off, so it never
        // lingers under the rows closing up.
        let isHidden = RowGlideAnimator.shared.isHiding(parentSnapshot.id)
        let shape = RoundedRectangle(cornerRadius: Chrome.rowCornerRadius, style: .continuous)
        Button(action: action) {
            HStack(spacing: 0) {
                HelperConnectorShape(endsHere: true)
                    .stroke(HelperConnector.color(hovering: isHovering), style: HelperConnector.style)
                    .frame(width: ThreadRowMetrics.connectorWidth, height: ThreadRowMetrics.helperHeight)
                HStack(spacing: 5) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 8, weight: .bold))
                    Text(verbatim: count == 1 ? "1 \(noun)" : "\(count) \(noun)s")
                        .font(.system(size: 11))
                    Spacer(minLength: 4)
                    if working {
                        MiniSpinner(cellSize: 2.4)
                            .padding(.trailing, 4)
                    }
                }
                .foregroundStyle(Chrome.secondaryText.opacity(isHovering ? 1 : 0.85))
                .padding(.leading, 6)
                .padding(.trailing, Chrome.rowHorizontalPadding)
                .frame(height: ThreadRowMetrics.helperHeight)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background {
                    shape.fill(isHovering ? Chrome.overlay(0.06) : Color.clear)
                }
                .contentShape(shape)
            }
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(count == 1 ? "Show the \(noun)" : "Show \(count) \(noun)s")
        .accessibilityLabel(Text(count == 1 ? "1 folded \(noun)" : "\(count) folded \(noun)s"))
        // A helper goes out of sight the moment its parent's ghost takes off, so it never
        // lingers under the rows closing up.
        .opacity(isHidden ? 0 : 1)
    }
}

/// A helper row or the folded line under a thread being dragged moves with it, lifted the
/// same way, so the thread and its helpers travel as one block.
private struct RidesWithDraggedParent: ViewModifier {
    let parentID: UUID?
    let drag: RowDrag<UUID>

    func body(content: Content) -> some View {
        let rides = parentID != nil && drag.id == parentID
        content
            .offset(y: rides ? drag.visualOffset : 0)
            .zIndex(rides ? 1 : 0)
    }
}

/// The dotted connector beside a helper's row: down from the row above, and a tick to the
/// row. Where the helpers end it stops at the tick. The whole column folds the helpers away.
private struct HelperConnector: View {
    let endsHere: Bool
    let action: () -> Void

    @State private var isHovering = false

    static let style = StrokeStyle(lineWidth: 1, lineCap: .round, dash: [0.1, 3.6])

    static func color(hovering: Bool) -> Color {
        Chrome.secondaryText.opacity(hovering ? 0.9 : 0.45)
    }

    var body: some View {
        Button(action: action) {
            HelperConnectorShape(endsHere: endsHere)
                .stroke(Self.color(hovering: isHovering), style: Self.style)
                .animation(Chrome.hover, value: isHovering)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .help("Hide helpers")
        .accessibilityLabel(Text("Hide helpers"))
    }
}

private struct HelperConnectorShape: Shape {
    let endsHere: Bool

    func path(in rect: CGRect) -> Path {
        var path = Path()
        let x = ThreadRowMetrics.connectorLineX
        let midY = rect.midY
        path.move(to: CGPoint(x: x, y: rect.minY))
        path.addLine(to: CGPoint(x: x, y: endsHere ? midY : rect.maxY))
        path.move(to: CGPoint(x: x, y: midY))
        path.addLine(to: CGPoint(x: rect.maxX - 2, y: midY))
        return path
    }
}

/// A thread in either sidebar layout. In the project layout it is one line with the thread's badge; in
/// the activity layout the project it belongs to shows underneath. It is the same row in both, so
/// switching layouts animates its height and contents rather than swapping one row for another.
/// Settled, it is one small grey line with a green check at the front in either layout: a row of its
/// own that the open row's ghost glides down to (see RowGlideAnimator).
private struct SidebarThreadRow: View, Equatable {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    /// The thread as the list last built it; `thread` below is the live record.
    let snapshot: ChatThread
    /// The row reads its own thread through the model's per-thread cell, so a change to it
    /// re-renders this row alone and a change to any other thread does not reach it. The
    /// snapshot is the fallback for a thread that has just been deleted.
    private var thread: ChatThread { model.thread(snapshot.id) ?? snapshot }
    /// The project shown under the title in the activity layout; nil in the project layout.
    let projectName: String?
    /// Carries rename/delete intents out of the AppKit menu without retaining row state.
    /// Held strongly: the box is the list's, not the row's, so holding it pins no row
    /// state, and a weak one could go nil under a menu still on screen.
    let menuRequests: SidebarMenuRequests
    /// Lifted and following the pointer in a reorder.
    var isDragged = false
    /// The list's frame in the window, for the glide; nil where no glide can show.
    var listFrame: FrameHolder?
    /// Folds the helpers under this thread away or brings them back; nil when it has none.
    var onToggleFold: (() -> Void)?
    let onRename: () -> Void
    let onDelete: () -> Void

    @State private var isHovering = false
    @State private var isMenuPresented = false
    /// The check was just clicked: it pops green as the row takes off.
    @State private var isSettling = false
    @State private var windowFrame = FrameHolder()

    var body: some View {
        // Through the thread's own cell (see `AppModel.isSelected`): a selection change
        // re-renders the row that lost it and the row that gained it, no other. The
        // runtime read below is this thread's alone; `existingRuntime` observes nothing
        // of the model, and the runtime's fields are observed per runtime.
        let isSelected = model.isSelected(thread.id)
        let isSettled = thread.isSettled
        let isDetailed = projectName != nil && !isSettled
        let showsActions = (isHovering || isMenuPresented) && !isDragged
        let showsFold = showsActions && onToggleFold != nil
        let settles = model.settings.threadFinishAction == .settle
        // The check shows at the trailing end while the thread is open (beside the ellipsis,
        // on hover) and sits at the front once it has settled.
        let showsCheck = isSettled || (settles && (showsActions || isSettling))
        // While the row's ghost is in the air the row itself stays out of sight, and shows
        // again just as the ghost lands on it.
        let isHidden = RowGlideAnimator.shared.isHiding(thread.id)
        // Read here, in the body, so the row follows the merge as it moves from stage to stage.
        let mergeStage = Self.mergeStage(of: model.existingRuntime(for: thread.id))
        let shape = RoundedRectangle(cornerRadius: Chrome.rowCornerRadius, style: .continuous)
        Button {
            model.selectedThreadID = thread.id
        } label: {
            ThreadRowFace(
                thread: thread,
                projectName: projectName,
                isSelected: isSelected,
                trailingClearance: Self.trailingClearance(isSettled: isSettled, isDetailed: isDetailed, showsActions: showsActions || isSettling, showsFold: showsFold),
                mergeStage: mergeStage
            ) {
                ThreadBadge(thread: thread)
            }
            .background {
                // Only the fill animates with selection; the row's place is never animated from here.
                shape
                    .fill(thread.isPinned && !isSelected ? Chrome.danger.opacity(0.10) : Chrome.overlay(Self.fill(isSelected: isSelected, isHovering: isHovering)))
                    .animation(Chrome.hover, value: isSelected)
            }
            .overlay {
                if thread.isPinned && isSelected {
                    shape.fill(Chrome.danger.opacity(0.06))
                }
            }
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .overlay(alignment: isSettled ? .leading : .trailing) {
            if showsCheck {
                SettleCheck(isSettled: isSettled, isPopping: isSettling) {
                    if isSettled { reopen() } else { settle() }
                }
                .padding(.leading, isSettled ? ThreadRowMetrics.settledCheckInset : 0)
                .padding(.trailing, isSettled ? 0 : ThreadRowMetrics.openCheckInset)
            }
        }
        .overlay(alignment: .trailing) {
            Group {
                if showsActions {
                    HStack(spacing: 0) {
                        if !settles, !isSettled {
                            Button {
                                archiveWithGenie()
                            } label: {
                                RowAccessoryIcon("archivebox")
                            }
                            .buttonStyle(.plain)
                            .help("Archive thread")
                        }
                        RowActionsButton(actions: makeActions(), isPresented: $isMenuPresented)
                    }
                } else if isDetailed || isSettled {
                    ActivityStatus(thread: thread)
                } else {
                    RelativeTimeLabel(date: thread.updatedAt)
                }
            }
            .padding(.trailing, 6)
        }
        // The helpers under the thread fold away and come back from here, as they do from the
        // connector beside them. The button sits before the check (or the archive) and the ellipsis.
        .overlay(alignment: .trailing) {
            if showsFold, let onToggleFold {
                let folded = thread.foldsHelpers
                Button(action: onToggleFold) {
                    RowAccessoryIcon(folded ? "chevron.right" : "chevron.down")
                }
                .buttonStyle(.plain)
                .padding(.trailing, ThreadRowMetrics.foldInset)
                .help(folded ? "Show helpers" : "Hide helpers")
                .accessibilityLabel(Text(folded ? "Show helpers" : "Hide helpers"))
            }
        }
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
            // The pointer resting on a row usually means a click is coming: decode its history now.
            if hovering { model.warmDocuments([thread.id]) }
        }
        .onGeometryChange(for: CGRect.self, of: Self.windowFrame) { frame in
            // Scrolling moves every row every frame: the frame is tracked only while a
            // settle or reopen can need it — while this row is dragged, under the pointer
            // (a settle starts from the row under it), or while a glide is in the air (the
            // arriving row reports where the ghost lands). `glides` is read in the event
            // closure, not the body, so it observes nothing.
            guard isDragged || isHovering || !RowGlideAnimator.shared.glides.isEmpty else { return }
            windowFrame.note(frame)
            // A row that has just arrived where a ghost of it is headed tells the ghost where to land.
            RowGlideAnimator.shared.land(threadID: thread.id, settled: snapshot.isSettled, at: frame)
        }
        .opacity(isHidden ? 0 : 1)
        // Hiding is instant, since the ghost takes over the row's frame in the same pass;
        // showing eases in over the 0.12 s handover window in which the ghost still covers
        // the row (see `RowGlideAnimator`).
        .animation(isHidden ? nil : .easeOut(duration: 0.12), value: isHidden)
        // Lifted: a touch larger with a shadow, over an opaque fill so the rows sliding
        // underneath never show through. The queue's rows lift the same way.
        .background {
            if isDragged {
                shape
                    .fill(Chrome.overlay(0.12))
                    .shadow(color: .black.opacity(0.28), radius: 10, y: 4)
            }
        }
        .scaleEffect(isDragged ? 1.02 : 1)
        .animation(.spring(response: 0.25, dampingFraction: 0.8), value: isDragged)
        // Snapshots only: the menu builder must not capture the row (its frames, its settle
        // handlers), so the AppKit menu it builds cannot pin row state after dismiss. Settle
        // and reopen go through finishRequest, which the row takes up with its glide below.
        .contextMenu { [thread, model, menuRequests] in
            let threadSnapshot = thread
            let modelSnapshot = model
            let requestsSnapshot = menuRequests
            let settleFirstSnapshot = !threadSnapshot.isHelper && modelSnapshot.settings.threadFinishAction == .settle
            RowActionMenuButtons(actions: ThreadActions.makeContextMenu(
                model: modelSnapshot,
                thread: threadSnapshot,
                settleFirst: settleFirstSnapshot,
                requests: requestsSnapshot
            ))
        }
        // A settle or reopen asked for from the menu, the shortcut or the palette lands here,
        // so it looks the same as a click on the check.
        .onChange(of: model.finishRequest) { _, request in
            guard let request, request.threadID == thread.id, request.reopens == isSettled else { return }
            model.finishRequest = nil
            if request.reopens { reopen() } else { settle() }
        }
        .accessibilityAddTraits(isSelected ? [.isSelected, .isButton] : .isButton)
        .accessibilityValue(Text(isSettled ? "Settled" : ""))
    }

    private static func fill(isSelected: Bool, isHovering: Bool) -> Double {
        if isSelected { return 0.12 }
        return isHovering ? 0.06 : 0
    }

    /// Value inputs only: closures and the frame holder are ignored, so the parent's
    /// re-evaluation skips rows whose thread did not change. Live state still reaches the
    /// body through the model's per-thread cells.
    static func == (lhs: SidebarThreadRow, rhs: SidebarThreadRow) -> Bool {
        lhs.snapshot.id == rhs.snapshot.id
            && lhs.snapshot.title == rhs.snapshot.title
            && lhs.snapshot.updatedAt == rhs.snapshot.updatedAt
            && lhs.snapshot.isPinned == rhs.snapshot.isPinned
            && lhs.snapshot.isSettled == rhs.snapshot.isSettled
            && lhs.projectName == rhs.projectName
            && lhs.isDragged == rhs.isDragged
            && (lhs.onToggleFold != nil) == (rhs.onToggleFold != nil)
    }

    /// The room the title leaves for what sits at the row's trailing end: the check and the
    /// ellipsis on hover, the status or the time otherwise.
    private static func trailingClearance(isSettled: Bool, isDetailed: Bool, showsActions: Bool, showsFold: Bool = false) -> CGFloat {
        if showsActions { return (isSettled ? 30 : 52) + (showsFold ? 20 : 0) }
        return isDetailed || isSettled ? 18 : 52
    }

    /// Built when the ellipsis shows or the context menu opens, never on a plain render.
    private func makeActions() -> [RowAction] {
        ThreadActions.make(model: model, thread: thread, onRename: onRename, onDelete: onDelete, onSettle: settle, onReopen: reopen)
    }

    private nonisolated static func windowFrame(_ proxy: GeometryProxy) -> CGRect {
        proxy.frame(in: .named(GenieAnimator.coordinateSpace))
    }

    // MARK: Settling

    /// The check pops green, the note sounds and the row takes off, all on the click:
    /// the ghost's own check carries the pop through the flight (see `RowGlideView`),
    /// so nothing waits for it to land first.
    private func settle() {
        guard !isSettling, !thread.isSettled else { return }
        if model.settings.settleSound { SettleChime.play() }
        withAnimation(.spring(response: 0.3, dampingFraction: 0.45)) { isSettling = true }
        glide(.settle)
        model.settleAnimated(thread.id, sounds: false)
    }

    private func reopen() {
        guard thread.isSettled else { return }
        glide(.reopen)
        model.reopenAnimated(thread.id)
    }

    /// Sends the row's ghost on its way: from this row's frame, looking like this row, to
    /// where the thread's other row will be (or the list's edge, when that is out of view),
    /// looking like that one. Launched just before the thread changes, so the new row can
    /// report its frame to the ghost as it arrives.
    private func glide(_ direction: RowGlideAnimator.Glide.Direction) {
        guard let listFrame else { return }
        let from = windowFrame.frame
        let bounds = listFrame.frame
        let isSelected = model.selectedThreadID == thread.id
        let settled = direction == .settle
        // The thread as its other row will show it: the settled one keeps its place at the
        // bottom unpinned and read; the reopened one is open again.
        var arriving = thread
        arriving.isSettled = settled
        if settled {
            arriving.isPinned = false
            arriving.hasUnread = false
        }
        let arrivalHeight = settled ? ThreadRowMetrics.settledHeight : (projectName != nil ? ThreadRowMetrics.detailedHeight : Chrome.rowHeight)
        var fallback = CGRect(
            x: from.minX,
            y: settled ? bounds.maxY : bounds.minY - arrivalHeight,
            width: from.width,
            height: arrivalHeight
        )
        // With the settled section folded there is no row to land on, so the ghost
        // heads for the `Settled` header's line and fades out there.
        if settled, model.settings.settledCollapsed, let header = RowGlideAnimator.shared.settledHeaderFrame, bounds.intersects(header) {
            fallback = CGRect(x: from.minX, y: header.maxY - arrivalHeight, width: from.width, height: arrivalHeight)
        }
        let animator = RowGlideAnimator.shared
        let badge = ThreadGhostBadge(thread: thread, runtime: model.existingRuntime(for: thread.id))
        // The faces leave the same room at the trailing end as the rows they stand in for: the
        // departing row is hovered, with its buttons out; the arriving one shows its status.
        let departureClearance = Self.trailingClearance(isSettled: thread.isSettled, isDetailed: projectName != nil, showsActions: true)
        let arrivalClearance = Self.trailingClearance(isSettled: settled, isDetailed: !settled && projectName != nil, showsActions: false)
        guard let departure = animator.render(
            ThreadRowFace(thread: thread, projectName: projectName, isSelected: isSelected, trailingClearance: departureClearance) { badge },
            size: from.size, colorScheme: colorScheme
        ), let arrival = animator.render(
            ThreadRowFace(thread: arriving, projectName: projectName, isSelected: isSelected, trailingClearance: arrivalClearance) { badge },
            size: CGSize(width: from.width, height: arrivalHeight), colorScheme: colorScheme
        ) else { return }
        animator.launch(
            threadID: thread.id,
            direction: direction,
            from: from,
            fallback: fallback,
            bounds: bounds,
            departure: departure,
            arrival: arrival,
            departureFill: Self.fill(isSelected: isSelected, isHovering: isHovering),
            arrivalFill: Self.fill(isSelected: isSelected, isHovering: false)
        )
    }

    /// What the team's merge is doing, with an ellipsis, while it runs; nil otherwise.
    static func mergeStage(of runtime: ThreadRuntime?) -> String? {
        guard let runtime, runtime.isHydraMerging else { return nil }
        return (runtime.hydraMergeStage ?? "Merging") + "\u{2026}"
    }

    // MARK: Archiving

    private func archiveWithGenie() {
        let isSelected = model.selectedThreadID == thread.id
        // The ghost mirrors the row's label — same text, badge, padding and
        // translucent fill — so the row hands over to it without a visible change.
        let fill = Chrome.overlay(isSelected ? 0.12 : 0.06)
        let shape = RoundedRectangle(cornerRadius: Chrome.rowCornerRadius, style: .continuous)
        let badge = ThreadGhostBadge(thread: thread, runtime: model.existingRuntime(for: thread.id))
        GenieAnimator.shared.launch(frame: windowFrame.frame, colorScheme: colorScheme) {
            ThreadRowFace(thread: thread, projectName: projectName, isSelected: isSelected, trailingClearance: 52) { badge }
                .background { shape.fill(fill) }
        }
        withAnimation(Chrome.panelSlide) { model.archive(thread.id) }
    }
}

/// What a thread row shows: the badge, title and project line while the thread is open, one
/// small grey title with room for the check once it has settled. The live row and its ghosts
/// draw the same face, so a ghost takes over from the row without a visible change.
private struct ThreadRowFace<Badge: View>: View {
    let thread: ChatThread
    /// The project under the title in the activity layout; nil in the project layout.
    let projectName: String?
    let isSelected: Bool
    /// The room left at the trailing end for the row's status, time or buttons.
    let trailingClearance: CGFloat
    /// What the team's merge is doing, shown in the project line's place while it runs.
    var mergeStage: String? = nil
    /// The badge at the front of an open row in the project layout.
    @ViewBuilder let badge: Badge

    var body: some View {
        let isSettled = thread.isSettled
        let isDetailed = projectName != nil && !isSettled
        HStack(spacing: 8) {
            if !isDetailed, !isSettled {
                badge
                    .frame(width: Chrome.iconSize)
                    .transition(.opacity)
            }
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: thread.title)
                    .font(.system(size: isSettled ? 12 : 13, weight: !isSettled && (isSelected || thread.hasUnread) ? .medium : .regular))
                    .foregroundStyle(Chrome.primaryText.opacity(Self.titleOpacity(isSettled: isSettled, isSelected: isSelected)))
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let projectName, !isSettled {
                    HStack(spacing: 4) {
                        Image(systemName: thread.worktreePath == nil ? "folder" : "arrow.triangle.branch")
                            .font(.system(size: 10))
                        Text(verbatim: mergeStage ?? projectName)
                            .font(.system(size: 12))
                            .lineLimit(1)
                    }
                    .foregroundStyle(Chrome.secondaryText)
                    .transition(.opacity)
                }
            }
            Spacer(minLength: 4)
        }
        .padding(.leading, Chrome.rowHorizontalPadding + (isSettled ? ThreadRowMetrics.settledTitleInset : 0))
        .padding(.trailing, Chrome.rowHorizontalPadding + trailingClearance)
        .padding(.vertical, isDetailed ? ThreadRowMetrics.detailedVerticalPadding : 0)
        .frame(minHeight: isSettled ? ThreadRowMetrics.settledHeight : Chrome.rowHeight)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// A settled title is grey; selected, it comes up a little so the selection reads.
    private static func titleOpacity(isSettled: Bool, isSelected: Bool) -> Double {
        if isSettled { return isSelected ? 0.75 : 0.55 }
        return isSelected ? 1 : 0.92
    }
}

/// The badge as a ghost carries it: the same marks as ThreadBadge, from a snapshot of the
/// runtime, with the spinner held still because a ghost is an image and cannot animate.
private struct ThreadGhostBadge: View {
    let thread: ChatThread
    let needsInput: Bool
    let isRunning: Bool

    init(thread: ChatThread, runtime: ThreadRuntime?) {
        self.thread = thread
        needsInput = !(runtime?.approvals.isEmpty ?? true) || !(runtime?.questions.isEmpty ?? true)
        isRunning = runtime?.isRunning == true || runtime?.isHydraMerging == true
    }

    var body: some View {
        SidebarIconBadge {
            if needsInput {
                SidebarSymbol("hand.raised.fill")
                    .foregroundStyle(Chrome.orange)
            } else if isRunning {
                MiniSpinner(cellSize: 2.4, isStill: true)
            } else if thread.isPinned {
                SidebarSymbol("pin.fill", scale: 0.9)
                    .foregroundStyle(Chrome.danger.opacity(0.85))
            } else {
                ProviderIcon(provider: thread.provider, size: 14)
            }
        }
    }
}

/// The check on a thread row. Open, it shows on hover at the trailing end and settles the
/// thread; settled, it is the green mark at the front of the row, and clicking it reopens
/// the thread. Hovering a settled one outlines it, the way a box about to be unticked would.
private struct SettleCheck: View {
    let isSettled: Bool
    /// Just clicked: filled green and popped, while the thread is about to settle.
    let isPopping: Bool
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        let isDone = isSettled || isPopping
        let filled = isPopping || (isSettled && !isHovering)
        Button(action: action) {
            Image(systemName: filled ? "checkmark.circle.fill" : "checkmark.circle")
                .font(Chrome.inlineIconFont)
                .foregroundStyle(isDone || isHovering ? Chrome.success.opacity(isSettled && !isHovering ? 0.8 : 1) : Chrome.secondaryText)
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 20, height: 20)
                .contentShape(.rect)
                .scaleEffect(isPopping ? 1.35 : 1)
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(isSettled ? "Reopen thread" : "Settle thread")
        .accessibilityLabel(Text(isSettled ? "Reopen thread" : "Settle thread"))
    }
}

private struct ThreadBadge: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread

    var body: some View {
        let runtime = model.existingRuntime(for: thread.id)
        let needsInput = !(runtime?.approvals.isEmpty ?? true) || !(runtime?.questions.isEmpty ?? true)
        let mergeStage = SidebarThreadRow.mergeStage(of: runtime)
        let isRunning = runtime?.isRunning == true || mergeStage != nil
        SidebarIconBadge {
            if needsInput {
                SidebarSymbol("hand.raised.fill")
                    .foregroundStyle(Chrome.orange)
            } else if isRunning {
                MiniSpinner(cellSize: 2.4)
                    .help(mergeStage ?? "")
            } else if thread.isPinned {
                SidebarSymbol("pin.fill", scale: 0.9)
                    .foregroundStyle(Chrome.danger.opacity(0.85))
            } else {
                ProviderIcon(provider: thread.provider, size: 14)
            }
        }
        .overlay(alignment: .topTrailing) {
            if thread.hasUnread, !needsInput, !isRunning {
                Circle()
                    .fill(thread.lastStatus == .failed ? Chrome.danger : Chrome.accent)
                    .frame(width: 7, height: 7)
                    .offset(x: 2.5, y: -2.5)
            }
        }
    }
}

/// Carries sidebar context-menu intents outside any row's @State. An AppKit context menu
/// outlives the right-click that opened it, so its callbacks must never retain a row's
/// state storage: menu -> closure -> row state -> view node -> menu responder is the back
/// edge that pinned ContextMenuResponder/NSMenu pairs after dismiss. Menu actions post a
/// value-type request here; the list takes it up. The rows and the menu hold this box
/// strongly: it belongs to the list, so nothing of a row hangs off it, and there is no
/// cycle back to the menu. Owned by the list's @State, touched on the main thread only.
@Observable
private final class SidebarMenuRequests {
    var rename: ChatThread?
    var delete: ChatThread?
}

/// What a thread row offers from its ellipsis popover and its context menu, in both sidebar layouts.
@MainActor
private enum ThreadActions {
    static func make(
        model: AppModel,
        thread: ChatThread,
        onRename: @escaping () -> Void,
        onDelete: @escaping () -> Void,
        onSettle: (() -> Void)? = nil,
        onReopen: (() -> Void)? = nil
    ) -> [RowAction] {
        var items = [RowAction(title: "Rename", symbol: "pencil") { onRename() }]
        // A helper keeps its place under its parent, and a settled thread its place at the
        // bottom; pinning would pull either out of it.
        if !thread.isHelper, !thread.isSettled {
            items.append(RowAction(title: thread.isPinned ? "Unpin" : "Pin", symbol: thread.isPinned ? "pin.slash" : "pin") {
                withAnimation(Chrome.panelSlide) {
                    model.updateThread(thread.id) { $0.isPinned.toggle() }
                }
            })
        }
        if let path = thread.worktreePath {
            items.append(RowAction(title: "Reveal worktree in Finder", symbol: "folder") { Workspace.revealInFinder(path) })
        }
        // A helper has no place of its own to settle into: it only archives, with its parent
        // or on its own. A thread offers both, the one Settings chose first.
        let settleFirst = !thread.isHelper && model.settings.threadFinishAction == .settle
        let archive = RowAction(title: "Archive", symbol: "archivebox", startsGroup: !settleFirst) {
            withAnimation(Chrome.panelSlide) { model.archive(thread.id) }
        }
        if thread.isHelper {
            items.append(archive)
        } else {
            let settle = RowAction(
                title: thread.isSettled ? "Reopen" : "Settle",
                symbol: thread.isSettled ? "arrow.uturn.backward" : "checkmark.circle",
                startsGroup: settleFirst
            ) {
                // The row's own handlers fly the ghost; without them the thread just moves.
                if thread.isSettled {
                    if let onReopen { onReopen() } else { model.reopenAnimated(thread.id) }
                } else if let onSettle {
                    onSettle()
                } else {
                    model.settleAnimated(thread.id)
                }
            }
            items += settleFirst ? [settle, archive] : [archive, settle]
        }
        items.append(RowAction(title: "Delete…", symbol: "trash", isDestructive: true) { onDelete() })
        return items
    }

    /// The context menu's items, built from value snapshots with weak captures: the AppKit
    /// menu outlives the right-click, so its callbacks must not retain the row, its frames,
    /// or its settle handlers. Rename and delete post to `requests`; the list takes them up. Settle and reopen post a finish request, which the row takes up with
    /// its pop and glide, the way the shortcut's and palette's do. Everything else touches
    /// only the model. (The ellipsis popover keeps `make`; it is not an AppKit menu.)
    static func makeContextMenu(
        model: AppModel,
        thread: ChatThread,
        settleFirst: Bool,
        requests: SidebarMenuRequests
    ) -> [RowAction] {
        let threadID = thread.id
        var items = [RowAction(title: "Rename", symbol: "pencil") { [weak model, requests, thread] in
            guard model != nil else { return }
            requests.rename = thread
        }]
        // A helper keeps its place under its parent, and a settled thread its place at the
        // bottom; pinning would pull either out of it.
        if !thread.isHelper, !thread.isSettled {
            let isPinned = thread.isPinned
            items.append(RowAction(title: isPinned ? "Unpin" : "Pin", symbol: isPinned ? "pin.slash" : "pin") { [weak model, threadID] in
                guard let model else { return }
                withAnimation(Chrome.panelSlide) {
                    model.updateThread(threadID) { $0.isPinned.toggle() }
                }
            })
        }
        if let path = thread.worktreePath {
            items.append(RowAction(title: "Reveal worktree in Finder", symbol: "folder") { [weak model, path] in
                guard model != nil else { return }
                Workspace.revealInFinder(path)
            })
        }
        // A helper has no place of its own to settle into: it only archives, with its parent
        // or on its own. A thread offers both, the one Settings chose first.
        let archive = RowAction(title: "Archive", symbol: "archivebox", startsGroup: !settleFirst) { [weak model, threadID] in
            guard let model else { return }
            withAnimation(Chrome.panelSlide) { model.archive(threadID) }
        }
        if thread.isHelper {
            items.append(archive)
        } else {
            let isSettled = thread.isSettled
            let settle = RowAction(
                title: isSettled ? "Reopen" : "Settle",
                symbol: isSettled ? "arrow.uturn.backward" : "checkmark.circle",
                startsGroup: settleFirst
            ) { [weak model, threadID, isSettled] in
                guard let model else { return }
                model.finishRequest = AppModel.FinishRequest(threadID: threadID, reopens: isSettled)
            }
            items += settleFirst ? [settle, archive] : [archive, settle]
        }
        items.append(RowAction(title: "Delete…", symbol: "trash", isDestructive: true) { [weak model, requests, thread] in
            guard model != nil else { return }
            requests.delete = thread
        })
        return items
    }
}

/// The bell beside the search field that switches the sidebar between projects and activity.
private struct ActivityViewToggle: View {
    @Binding var isOn: Bool
    @State private var isHovering = false

    var body: some View {
        let chord = ShortcutStore.shared.chord(for: .toggleActivityView).map { " (\($0.description))" } ?? ""
        Button {
            withAnimation(Chrome.panelSlide) { isOn.toggle() }
        } label: {
            Image(systemName: "bell")
                .font(Chrome.iconFont)
                .foregroundStyle(isOn ? Chrome.accent : Chrome.secondaryText)
                .frame(width: 30, height: 30)
                .background {
                    Circle()
                        .fill(isOn ? Chrome.accent.opacity(0.16) : (isHovering ? Chrome.overlay(0.08) : Color.clear))
                }
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help((isOn ? "Turn off activity view" : "Turn on activity view") + chord)
        .accessibilityLabel(Text("Activity view"))
        .accessibilityValue(Text(isOn ? "On" : "Off"))
    }
}

private struct ActivityHeader: View {
    let title: String
    let isFirst: Bool

    var body: some View {
        Text(verbatim: title)
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(Chrome.secondaryText)
            .padding(.horizontal, Chrome.rowHorizontalPadding)
            .padding(.top, isFirst ? 2 : 14)
            .padding(.bottom, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The Settled section's header: a disclosure chevron, the title and its count. Clicking
/// it folds the settled threads away or brings them back, with the sidebar's motion. The
/// choice persists in settings, so the section stays as left across relaunch.
private struct SettledHeader: View {
    @Environment(AppModel.self) private var model
    let count: Int
    let isFirst: Bool

    @State private var shown: Int
    @State private var countTask: Task<Void, Never>?

    /// The count's own roll: short and without bounce. The rows fly on `Chrome.settleFlight`,
    /// whose spring overshoots, and digits rolling on it slid past the new number and back.
    private static var roll: Animation {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            ? .easeOut(duration: 0.15)
            : .smooth(duration: 0.28)
    }

    init(count: Int, isFirst: Bool) {
        self.count = count
        self.isFirst = isFirst
        _shown = State(initialValue: count)
    }

    var body: some View {
        let collapsed = model.settings.settledCollapsed
        Button {
            withAnimation(Chrome.panelSlide) {
                model.settings.settledCollapsed.toggle()
            }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .rotationEffect(.degrees(collapsed ? 0 : 90))
                    .animation(Chrome.panelSlide, value: collapsed)
                Text(verbatim: "Settled")
                    .font(.system(size: 12, weight: .semibold))
                Text(verbatim: "\(shown)")
                    .font(.system(size: 12))
                    .monospacedDigit()
                    .opacity(0.8)
                    .fixedSize()
                    .contentTransition(.numericText(value: Double(shown)))
                    .animation(Self.roll, value: shown)
                Spacer(minLength: 4)
            }
            .foregroundStyle(Chrome.secondaryText)
            .padding(.horizontal, Chrome.rowHorizontalPadding)
            .padding(.top, isFirst ? 2 : 14)
            .padding(.bottom, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
            .animation(Chrome.panelSlide, value: isFirst)
        }
        .buttonStyle(.plain)
        .onGeometryChange(for: CGRect.self, of: { $0.frame(in: .named(GenieAnimator.coordinateSpace)) }) { RowGlideAnimator.shared.noteSettledHeader($0) }
        .help(collapsed ? "Show settled threads" : "Hide settled threads")
        .accessibilityLabel(Text("Settled, \(shown) threads"))
        .accessibilityValue(Text(collapsed ? "Collapsed" : "Expanded"))
        .accessibilityAddTraits(.isButton)
        .onAppear {
            shown = count
        }
        .onChange(of: count) { _, newCount in
            countTask?.cancel()
            // A settle counts up when its ghost lands, which `RowGlideAnimator` puts at
            // the spring's settling duration less its 0.12s handover; a reopen counts
            // down at once, as the row leaves at once. With Reduce Motion nothing flies,
            // so nothing waits.
            guard newCount > shown, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
                withAnimation(Self.roll) { self.shown = newCount }
                return
            }
            countTask = Task { @MainActor in
                do {
                    try await Task.sleep(for: .seconds(max(RowGlideAnimator.spring.settlingDuration - 0.12, 0)))
                } catch {
                    return
                }
                guard !Task.isCancelled else { return }
                withAnimation(Self.roll) { self.shown = newCount }
            }
        }
        .onDisappear {
            countTask?.cancel()
            RowGlideAnimator.shared.noteSettledHeader(nil)
        }
    }
}

private struct ActivityStatus: View {
    @Environment(AppModel.self) private var model
    let thread: ChatThread

    var body: some View {
        let runtime = model.existingRuntime(for: thread.id)
        if !(runtime?.approvals.isEmpty ?? true) || !(runtime?.questions.isEmpty ?? true) {
            Image(systemName: "hand.raised.fill")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.warning)
        } else if runtime?.isRunning == true || runtime?.isHydraMerging == true {
            MiniSpinner(cellSize: 2.4)
                .help(SidebarThreadRow.mergeStage(of: runtime) ?? "")
        } else if thread.hasUnread {
            Circle()
                .fill(thread.lastStatus == .failed ? Chrome.danger : Chrome.accent)
                .frame(width: 7, height: 7)
                .padding(.trailing, 4)
        }
    }
}

/// A row's latest frame, kept out of SwiftUI's observation: scrolling updates it on every frame,
/// and only an archive tap ever reads it. Writes skip when unchanged, so a resize frame
/// that moved nothing in the row costs nothing.
private final class FrameHolder {
    var frame: CGRect = .zero

    func note(_ frame: CGRect) {
        guard frame != self.frame else { return }
        self.frame = frame
    }
}

/// Every row's measured height, kept out of observation for the same reason: the list writes
/// one on every layout pass, and only a drag reads them. Unchanged reports are skipped,
/// so a width-only pass never touches the table.
private final class RowHeights {
    var values: [String: CGFloat] = [:]

    func note(_ id: String, _ height: CGFloat) {
        guard values[id] != height else { return }
        values[id] = height
    }
}

/// Every open thread row's frame in the list's space, kept out of observation like the
/// heights: the rows write as they lay out, and only the grab of a drag reads them.
private final class RowFrames {
    private var frames: [UUID: CGRect] = [:]

    func note(_ id: UUID, _ frame: CGRect) {
        guard frames[id] != frame else { return }
        frames[id] = frame
    }

    func forget(_ id: UUID) {
        frames[id] = nil
    }

    /// The thread whose row is under `point`, if an open one is.
    func thread(at point: CGPoint) -> UUID? {
        frames.first { $0.value.contains(point) }?.key
    }
}

/// One project's search hits: the threads matching, and how many it has in all.
private struct SidebarSearchResult {
    let project: Project
    let threads: [ChatThread]
    let count: Int
}

/// The sidebar's `threadPlacements`, kept while a cheap fingerprint of the threads stands:
/// a selection change or hover re-renders the list without rebuilding placements for
/// hundreds of threads. The fingerprint covers every field `ThreadPlacement` reads, so a
/// placement change always rebuilds.
private final class PlacementCache {
    private struct Fingerprint: Equatable {
        let id: UUID
        let projectID: UUID
        let parentThreadID: UUID?
        let isArchived: Bool
        let isInPanel: Bool
        let isSettled: Bool
        let settledAt: Date?
        let isPinned: Bool
        let sortOrder: Double?
        let activityOrder: Double?
        let activityOrderDay: Date?
        let createdAt: Date
        let updatedAt: Date
        let foldsHelpers: Bool
        let isHydraHead: Bool
        let hydraIndex: Int?
        let hasUnread: Bool

        init(_ thread: ChatThread) {
            id = thread.id
            projectID = thread.projectID
            parentThreadID = thread.parentThreadID
            isArchived = thread.isArchived
            isInPanel = thread.isInPanel
            isSettled = thread.isSettled
            settledAt = thread.settledAt
            isPinned = thread.isPinned
            sortOrder = thread.sortOrder
            activityOrder = thread.activityOrder
            activityOrderDay = thread.activityOrderDay
            createdAt = thread.createdAt
            updatedAt = thread.updatedAt
            foldsHelpers = thread.foldsHelpers
            isHydraHead = thread.isHydraHead
            hydraIndex = thread.hydra?.index
            hasUnread = thread.hasUnread
        }
    }

    private var fingerprint: [Fingerprint]?
    private var cached: [ThreadPlacement] = []

    func placements(for threads: [ChatThread]) -> [ThreadPlacement] {
        let next = threads.map(Fingerprint.init)
        if next == fingerprint { return cached }
        fingerprint = next
        cached = threads.map(ThreadPlacement.init)
        return cached
    }
}

/// The fields that decide where a thread sits in the list and how its group is ordered:
/// which project or parent it is under, whether it is shown at all, settled, pinned or
/// folded, and the dates and orders the sorts read. A title, an unread flag or a head's
/// progress note changes none of them, so those never rebuild the list; the rows read
/// them through the model's per-thread cells instead.
private struct ThreadPlacement: Equatable {
    let id: UUID
    let projectID: UUID
    let parentThreadID: UUID?
    let isArchived: Bool
    let isInPanel: Bool
    let isSettled: Bool
    let settledAt: Date?
    let isPinned: Bool
    let sortOrder: Double?
    /// The activity layout's own order within a day (see `placed`).
    let activityOrder: Double?
    let activityOrderDay: Date?
    let createdAt: Date
    let updatedAt: Date
    let foldsHelpers: Bool
    let isHydraHead: Bool
    let hydraIndex: Int?
    let hasUnread: Bool

    init(_ thread: ChatThread) {
        id = thread.id
        projectID = thread.projectID
        parentThreadID = thread.parentThreadID
        isArchived = thread.isArchived
        isInPanel = thread.isInPanel
        isSettled = thread.isSettled
        settledAt = thread.settledAt
        isPinned = thread.isPinned
        sortOrder = thread.sortOrder
        activityOrder = thread.activityOrder
        activityOrderDay = thread.activityOrderDay
        createdAt = thread.createdAt
        updatedAt = thread.updatedAt
        foldsHelpers = thread.foldsHelpers
        isHydraHead = thread.isHydraHead
        hydraIndex = thread.hydra?.index
        hasUnread = thread.hasUnread
    }
}

/// The sidebar's built list, kept while its inputs stand, the way `TimelineBlockCache`
/// keeps a chat's blocks. The key holds each thread's placement (not the thread: a title
/// or a progress note is the row's to pick up) and the project array itself, so a render
/// that moved nothing costs one pass of comparisons and no filter or sort, and one that
/// did rebuilds exactly once. Owned by the list's @State, main thread only.
private final class SidebarItemCache {
    struct Key: Equatable {
        var placements: [ThreadPlacement]
        var projects: [Project]
        /// The threads heading the activity layout under "Needs attention".
        var attention: Set<UUID>
        var activityView: Bool
        var settledCollapsed: Bool
    }

    struct SearchKey: Equatable {
        var threads: [ChatThread]
        var projects: [Project]
        var query: String
    }

    private var key: Key?
    private var built: [SidebarItem] = []
    private var helpersFor: [ThreadPlacement]?
    private var helpers: [UUID: [ChatThread]] = [:]
    private var searchKey: SearchKey?
    private var found: [SidebarSearchResult] = []
    /// Each thread's title lowercased once, for the search to match against; a thread
    /// keeps its entry until its title changes.
    private var titlesFor: [ChatThread]?
    private var titles: [UUID: String] = [:]

    func items(for wanted: Key, build: () -> [SidebarItem]) -> [SidebarItem] {
        if wanted == key { return built }
        built = build()
        key = wanted
        return built
    }

    /// The helper grouping, kept while the placements stand; `group` runs only when they changed.
    func helpers(for placements: [ThreadPlacement], group: () -> [UUID: [ChatThread]]) -> [UUID: [ChatThread]] {
        if placements == helpersFor { return helpers }
        helpers = group()
        helpersFor = placements
        return helpers
    }

    func search(for wanted: SearchKey, find: ([UUID: String]) -> [SidebarSearchResult]) -> [SidebarSearchResult] {
        if wanted == searchKey { return found }
        found = find(lowercasedTitles(for: wanted.threads))
        searchKey = wanted
        return found
    }

    private func lowercasedTitles(for threads: [ChatThread]) -> [UUID: String] {
        if threads == titlesFor { return titles }
        var next: [UUID: String] = [:]
        next.reserveCapacity(threads.count)
        // Only a title that changed is lowercased again; the rest carry over.
        let previous = titlesFor.map { Dictionary($0.map { ($0.id, $0.title) }, uniquingKeysWith: { first, _ in first }) } ?? [:]
        for thread in threads {
            if previous[thread.id] == thread.title, let kept = titles[thread.id] {
                next[thread.id] = kept
            } else {
                next[thread.id] = thread.title.lowercased()
            }
        }
        titles = next
        titlesFor = threads
        return titles
    }
}

/// The minute hand behind the sidebar's relative times. One tick for the whole list, read by
/// the time labels and nothing else, so an idle app keeps "3m" truthful without re-rendering
/// the rows around it.
@MainActor
@Observable
final class MinuteClock {
    static let shared = MinuteClock()

    private(set) var now = Date.now

    @ObservationIgnored private var tick: Task<Void, Never>?

    private init() {
        tick = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                guard let self else { return }
                self.now = .now
            }
        }
    }
}

/// How long ago a thread was last active, at the trailing end of its row. A view of its own,
/// so the minute's tick re-renders this label alone.
private struct RelativeTimeLabel: View {
    let date: Date

    var body: some View {
        Text(verbatim: RelativeTime.short(date, now: MinuteClock.shared.now))
            .font(.system(size: 11).monospacedDigit())
            .foregroundStyle(Chrome.secondaryText.opacity(0.8))
            .padding(.trailing, 4)
    }
}
