// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI
import UserNotifications

struct AppAlert: Identifiable {
    let id = UUID()
    var title: String
    var message: String
}

/// The app's library of projects and threads, plus everything that spans threads.
@MainActor
@Observable
final class AppModel {
    let settings: AppSettings
    let providers: ProviderRegistry
    /// The user's MCP connections, handed to every provider session at launch.
    let mcp: MCPStore
    let terminals: TerminalStore
    let sidebar: SidebarLayout
    /// The column on the window's trailing edge: the Crew deck, the terminal, the
    /// thread's changes (see `RightSidebarLayout`).
    let rightSidebar: RightSidebarLayout
    /// The Studiolo vault's archives beside the projects (see `AppModel+Studiolo`).
    let studiolo: StudioloStore
    /// Picks chats back up once a spent usage limit resets, with the setting on.
    @ObservationIgnored private(set) var autoContinue: AutoContinue!
    /// How often each hosted hand has been started again after failing at the door
    /// (see `crewHandTurnFinished`); a hand gets two more goes.
    @ObservationIgnored var crewHandRetries: [UUID: Int] = [:]

    /// The lists are the source of truth for anything that shows several items at once. Single
    /// lookups go through the cells below, so the timeline rows, composer and chat header of one
    /// thread are left alone when another thread's title, status or unread flag changes.
    private(set) var projects: [Project] = [] {
        didSet { Self.write(Self.sync(&projectCells, with: projects)) }
    }
    private(set) var threads: [ChatThread] = [] {
        didSet {
            // The positions stand while every thread kept its place and its project; a
            // metadata change to a thread rebuilds nothing.
            let changedMembership = threads.count != threadIndex.positions.count || threads.enumerated().contains { index, thread in
                guard threadIndex.positions[thread.id] == index, let previous = threadCells[thread.id]?.value else { return true }
                return previous.projectID != thread.projectID
            }
            Self.write(Self.sync(&threadCells, with: threads))
            syncChildCells()
            if changedMembership { threadIndex = ThreadIndex(threads) }
        }
    }
    private var threadIndex = ThreadIndex([])

    /// Where each thread sits in `threads`, and each project's threads by position, so a
    /// lookup or an update never scans the list.
    private struct ThreadIndex {
        var positions: [UUID: Int] = [:]
        var projects: [UUID: [Int]] = [:]

        init(_ threads: [ChatThread]) {
            positions.reserveCapacity(threads.count)
            for (index, thread) in threads.enumerated() {
                if positions[thread.id] == nil { positions[thread.id] = index }
                projects[thread.projectID, default: []].append(index)
            }
        }
    }
    @ObservationIgnored private var projectCells: [UUID: ObservedValue<Project?>] = [:]
    @ObservationIgnored private var threadCells: [UUID: ObservedValue<ChatThread?>] = [:]
    /// The children of one thread, by that thread's id: the ids alone, so a view showing a
    /// chat's panels depends on which children it has and on each of those children, never
    /// on the rest of the library. A hand's status write leaves every other chat alone.
    @ObservationIgnored private var childCells: [UUID: ObservedValue<[UUID]>] = [:]
    /// Whether one thread is the selected one, observed on its own: a sidebar row reads its
    /// flag here rather than `selectedThreadID`, so a selection change re-renders the row
    /// that lost it and the row that gained it, not every row in the list.
    @ObservationIgnored private var selectionCells: [UUID: ObservedValue<Bool>] = [:]
    /// Each archive's page model, by prefix, made on first sight and kept (see
    /// `archivePage(for:)`): a draft typed there outlives leaving the page.
    @ObservationIgnored var archivePages: [String: ArchivePageModel] = [:]
    /// The archive whose page fills the main area (see `ArchivePage`), or nil for a thread
    /// or nothing. One or the other: picking a thread closes the page, opening the page
    /// unselects the thread. A page with an unsaved draft is not closed under the user:
    /// it stays up and asks (`archiveLeave`) while the thread waits selected behind it.
    var selectedArchivePrefix: String? {
        didSet {
            if selectedArchivePrefix != nil, selectedThreadID != nil { selectedThreadID = nil }
            if selectedArchivePrefix != oldValue { archiveLeave = nil }
        }
    }
    /// A thread picked while the archive page has an unsaved draft: the page asks whether
    /// to save, discard or stay before it goes (see `ArchivePage`, `leaveArchive`).
    var archiveLeave: ArchiveLeaveRequest?

    var selectedThreadID: UUID? {
        didSet {
            if let selectedThreadID, let prefix = selectedArchivePrefix {
                if archivePages[prefix]?.isDirty == true {
                    archiveLeave = ArchiveLeaveRequest(prefix: prefix, threadID: selectedThreadID)
                } else {
                    selectedArchivePrefix = nil
                }
            }
            if oldValue != selectedThreadID {
                if let oldValue { selectionCells[oldValue]?.value = false }
                if let selectedThreadID { selectionCell(selectedThreadID).value = true }
                SwitchLatency.noteSwitch(to: selectedThreadID.flatMap(thread)?.title ?? "none")
            }
            if let selectedThreadID {
                // A helper or hand under the selected thread is on screen with it, so
                // reading the thread reads them too; otherwise their unread mark would
                // stick on the dock after the thread that set it is back in view.
                // The write waits for the selection to paint, so the click itself
                // never rewrites the library.
                let ids = [selectedThreadID] + children(of: selectedThreadID).map(\.id)
                Task { @MainActor [weak self] in self?.markRead(ids) }
            }
            if oldValue != selectedThreadID {
                scheduleIdleSessionStop(leaving: oldValue)
                if let id = selectedThreadID {
                    Task { @MainActor [weak self] in self?.warmNeighbors(of: id) }
                }
            }
            if let selectedThreadID, let thread = thread(selectedThreadID) {
                rememberLastProject(thread.projectID)
            }
        }
    }

    /// The thread the sidebar should bring into view on its next pass, set when a thread
    /// lands somewhere new (moved to another project from the new-thread prompt, say): the
    /// list scrolls to that row and clears this. Never persisted, and nil at rest.
    var sidebarRevealThreadID: UUID?

    var isCommandPalettePresented = false
    var alert: AppAlert?
    /// Alerts waiting for the one showing to be dismissed (see `showAlert`).
    @ObservationIgnored private var queuedAlerts: [AppAlert] = []
    /// The threads whose unreadable history the user has been told of this run (see
    /// `noteThreadStorageProblem`).
    @ObservationIgnored private var threadStorageProblemsShown = Set<UUID>()

    @ObservationIgnored private var runtimes: [UUID: ThreadRuntime] = [:]
    @ObservationIgnored private var idleSessionStops: [UUID: Task<Void, Never>] = [:]
    @ObservationIgnored private var saveTask: Task<Void, Never>?
    /// The repeating merge sweep started at bootstrap. A chat whose automatic merge never
    /// got its moment while the app stayed open (a report batch left half-finished, a
    /// dialog never answered, a turn that ended in a way the trigger skipped) would sit on
    /// its files forever, with the merge note still promising they go out. The sweep asks
    /// again every few minutes; it skips anything still running, so a busy chat is left alone.
    @ObservationIgnored private var crewMergeSweepTask: Task<Void, Never>?
    @ObservationIgnored private var persistenceEnabled = true
    /// What went wrong reading library.json at launch, if anything did (see
    /// `Storage.loadLibrary`). The app then started without the projects and threads the file
    /// held; while the file is still there unread, nothing is saved over it this run.
    @ObservationIgnored private(set) var libraryLoadProblem: StorageProblem?
    @ObservationIgnored private var didRequestNotifications = false

    init() {
        settings = AppSettings()
        providers = ProviderRegistry(settings: settings)
        mcp = MCPStore()
        terminals = TerminalStore()
        sidebar = SidebarLayout()
        rightSidebar = RightSidebarLayout()
        studiolo = StudioloStore()
        // A library that did not read is not taken for an empty one: it was kept aside, or
        // left where it is and not saved over (see `Storage.loadLibrary`); `bootstrap` says so.
        let loaded = Storage.loadLibrary()
        libraryLoadProblem = loaded.problem
        let library = loaded.value ?? Library()
        projects = library.projects
        threads = library.threads
        for index in threads.indices where threads[index].lastStatus == .running {
            threads[index].lastStatus = .interrupted
        }
        // A hand at work when the app last quit went with its session.
        for index in threads.indices where threads[index].crew?.status == .running {
            threads[index].crew?.status = .stopped
            threads[index].crew?.finishedAt = .now
        }
        // Property observers stay quiet inside an initializer, so the cells are built here once.
        Self.write(Self.sync(&projectCells, with: projects))
        Self.write(Self.sync(&threadCells, with: threads))
        syncChildCells()
        threadIndex = ThreadIndex(threads)
        autoContinue = AutoContinue(app: self)
        // A project missing from a library that did not read may well still be there.
        if libraryLoadProblem == nil, let lastID = settings.lastProjectID, project(lastID) == nil {
            settings.lastProjectID = nil
        }
        startStudiolo()
    }

    /// Mirrors a list into its cells: values that changed are written to their cell (and only
    /// those, so an untouched item's observers never fire), new items get a cell, and a removed
    /// item's cell is emptied as it goes, so whoever was showing it re-renders to nothing.
    ///
    /// Only the dictionary is settled here; the cell writes are handed back for `write` to make
    /// once the caller's exclusive access to the dictionary is over. A cell write is observed,
    /// and inside `withAnimation` SwiftUI rebuilds the menu bar's commands on the spot, from
    /// within the write: they read the selected thread back through `thread(_:)`, which traps
    /// on exclusivity if the dictionary is still borrowed then.
    private static func sync<Value: Identifiable & Equatable>(
        _ cells: inout [Value.ID: ObservedValue<Value?>],
        with values: [Value]
    ) -> [(cell: ObservedValue<Value?>, value: Value?)] {
        var writes: [(cell: ObservedValue<Value?>, value: Value?)] = []
        var seen = Set<Value.ID>(minimumCapacity: values.count)
        for value in values {
            seen.insert(value.id)
            if let cell = cells[value.id] {
                if cell.value != value { writes.append((cell, value)) }
            } else {
                cells[value.id] = ObservedValue(value)
            }
        }
        guard cells.count != seen.count else { return writes }
        for (id, cell) in cells where !seen.contains(id) {
            writes.append((cell, nil))
            cells[id] = nil
        }
        return writes
    }

    /// Makes the writes `sync` handed back, with no access to the cell dictionaries open.
    private static func write<Value>(_ writes: [(cell: ObservedValue<Value?>, value: Value?)]) {
        for (cell, value) in writes { cell.value = value }
    }

    /// Mirrors who hangs under whom into the child cells, with the same no-op-write
    /// discipline as the item cells: a parent whose children are unchanged is not written,
    /// so the chat showing them is left alone. A parent that still exists keeps its cell
    /// even with nothing under it, because that is exactly the cell a chat with no hands
    /// yet is watching for its first one; only a parent that is gone loses its cell.
    private func syncChildCells() {
        var children: [UUID: [UUID]] = [:]
        for thread in threads {
            guard let parent = thread.parentThreadID else { continue }
            children[parent, default: []].append(thread.id)
        }
        for (parent, cell) in childCells {
            let ids = children[parent] ?? []
            if cell.value != ids { cell.value = ids }
        }
        for (parent, ids) in children where childCells[parent] == nil {
            childCells[parent] = ObservedValue(ids)
        }
        guard childCells.count != children.count else { return }
        let live = Set(threads.map(\.id))
        for parent in childCells.keys where !live.contains(parent) && children[parent] == nil {
            childCells[parent] = nil
        }
    }

    /// The ids under a thread, observed on their own. A thread with nothing under it yet
    /// gets its cell here, on the first read, so the caller hears about the first child
    /// that arrives; the cell is not observed state, so making it re-renders nothing.
    private func childIDs(of parentID: UUID) -> [UUID] {
        if let cell = childCells[parentID] { return cell.value }
        let cell = ObservedValue<[UUID]>([])
        childCells[parentID] = cell
        return cell.value
    }

    /// A thread's selection cell, made on the first ask; the cell is not observed state, so
    /// making it re-renders nothing.
    private func selectionCell(_ id: UUID) -> ObservedValue<Bool> {
        if let cell = selectionCells[id] { return cell }
        let cell = ObservedValue(selectedThreadID == id)
        selectionCells[id] = cell
        return cell
    }

    /// Whether the thread is selected, depending on this thread's flag alone.
    func isSelected(_ id: UUID) -> Bool {
        selectionCell(id).value
    }

    /// The threads under a thread, each read through its own cell: the caller depends on
    /// which children this thread has and on those children alone.
    func children(of parentID: UUID) -> [ChatThread] {
        childIDs(of: parentID).compactMap(thread)
    }

    func bootstrap() async {
        // With the window up: what did not read at launch, and where it went.
        reportStorageProblems()
        // The threads most likely to be opened first decode in the background from the start,
        // so the first click into a conversation never parses its history on the main thread.
        let recent = threads.filter { !$0.isArchived && !$0.isInPanel }.sorted { $0.updatedAt > $1.updatedAt }.prefix(12).map(\.id)
        warmDocuments(recent)
        await settings.loadKeychainInBackground()
        await LoginEnvironment.load()
        // After the login environment, so git runs with the user's PATH.
        sweepCrewCopies()
        // A merge the last run did not finish (the app was quit under it) goes out now.
        await resumeCrewMerges()
        // The launch sweep alone is not enough: a merge can also be missed while the app
        // stays up, and then nothing asks again until the next launch. This task asks every
        // five minutes for as long as the app runs; the sweep itself skips chats with a turn
        // or a hand still running, so it is safe to repeat.
        crewMergeSweepTask?.cancel()
        crewMergeSweepTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(300))
                guard !Task.isCancelled, let self else { return }
                await self.resumeCrewMerges()
            }
        }
        await providers.refreshAll()
        // The catalogs a fresh install starts without, or with only a seed of: one CLI and
        // two HTTPS calls, overlapped. Claude's brings its slash commands too, so the picker
        // and the composer are complete before any turn. A provider switched off in Settings
        // was not checked above and loads nothing here either.
        await withTaskGroup(of: Void.self) { group in
            for provider in [ProviderKind.claude, .codex, .deepseek, .meta] where settings.isEnabled(provider) && providers.status(provider).isInstalled {
                group.addTask { await self.providers.loadCatalog(provider) }
            }
        }
        if settings.isEnabled(.zai), providers.status(.zai).isInstalled {
            await providers.loadCatalog(.zai)
        }
        // Last, once nothing the first click needs is waiting on the disk: the attachment
        // files no thread refers to any more go, a day after they were written.
        await Storage.sweepOrphanedAttachments()
        // And, once, the checkpoints of threads archived before archiving let go of them.
        sweepArchivedCheckpoints()
    }

    /// Tells the user about every store that did not read at launch (see `StoredFile`), in one
    /// alert: where what it held was kept, or that it is left alone and not saved over this
    /// run. The library, the settings and shortcuts kept in user defaults, the MCP servers and
    /// connections, and the token activity ledger are all read by now; the ledger is read here
    /// if nothing has asked for it yet. A thread's history is read when the thread is first
    /// wanted, and says so then (see `noteThreadStorageProblem`).
    private func reportStorageProblems() {
        var problems: [StorageProblem] = []
        if let libraryLoadProblem { problems.append(libraryLoadProblem) }
        problems += settings.loadProblems
        if let shortcuts = ShortcutStore.shared.loadProblem { problems.append(shortcuts) }
        problems += mcp.loadProblems
        if let ledger = TokenLedger.shared.loadProblem { problems.append(ledger) }
        guard let first = problems.first else { return }
        var messages = problems.map(\.message)
        // The library comes first when it is there (see above); while its kept copy is
        // beside it, `Storage.keepsUnreadableLibrary` holds the Crew copy sweep back.
        if case .keptAside = libraryLoadProblem?.outcome {
            messages[0] += " While it is kept there, copies of a checkout that Crew hands worked in are not cleared away, since its threads may still name them."
        }
        showAlert(AppAlert(
            title: problems.count == 1 ? "\(first.subject) could not be read" : "Some saved data could not be read",
            message: messages.joined(separator: "\n\n")
        ))
    }

    /// Tells the user, once per thread per run, that a thread's history did not read (see
    /// `ThreadRuntime.finishLoad`). The thread's timeline says so as well, but a thread no one
    /// opens is read all the same (a Crew hand, one a queued follow-up or a continuation
    /// wakes), and while its file is left in place nothing it does this run is kept: this
    /// alert may be all the user hears of it.
    ///
    /// Threads found unreadable while one such alert still waits its turn join it, rather
    /// than each asking for its own dismissal.
    func noteThreadStorageProblem(_ problem: StorageProblem, thread id: UUID) {
        guard threadStorageProblemsShown.insert(id).inserted else { return }
        let words = "“\(thread(id)?.title ?? ChatThread.untitled)”: \(problem.message)"
        if let waiting = queuedAlerts.lastIndex(where: { Self.threadStorageAlertTitles.contains($0.title) }) {
            queuedAlerts[waiting].title = Self.threadStorageAlertTitles[1]
            queuedAlerts[waiting].message += "\n\n" + words
        } else {
            showAlert(AppAlert(title: Self.threadStorageAlertTitles[0], message: words))
        }
    }

    /// For one thread, and for several (see `noteThreadStorageProblem`).
    private static let threadStorageAlertTitles = ["A thread's history could not be read", "Some threads' histories could not be read"]

    /// Shows `alert` once the one showing, if any, is dismissed, where assigning `alert`
    /// would put it in that one's place: for word the user must not miss.
    func showAlert(_ alert: AppAlert) {
        if self.alert == nil {
            self.alert = alert
        } else {
            queuedAlerts.append(alert)
        }
    }

    /// Closes the alert showing and brings up the next one waiting (see `showAlert`), a
    /// moment later, once the closing one is off the screen.
    func dismissAlert() {
        alert = nil
        guard !queuedAlerts.isEmpty else { return }
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard let self, self.alert == nil, !self.queuedAlerts.isEmpty else { return }
            self.alert = self.queuedAlerts.removeFirst()
        }
    }

    /// Switches a provider on or off, from Settings › Providers. Launch checks only the
    /// providers that are on (see `ProviderRegistry.refreshAll`), so one switched on is
    /// checked now and its models load, rather than it reading as missing until the next
    /// launch.
    func setProviderEnabled(_ enabled: Bool, for provider: ProviderKind) {
        guard settings.isEnabled(provider) != enabled else { return }
        settings.setEnabled(enabled, for: provider)
        guard enabled else { return }
        Task {
            await providers.refresh(provider)
            guard settings.isEnabled(provider), providers.status(provider).isInstalled else { return }
            await providers.loadCatalog(provider)
        }
    }

    // MARK: - Lookup

    /// One project, observed on its own: only a change to this project re-renders the caller.
    func project(_ id: UUID) -> Project? {
        projectCells[id]?.value ?? nil
    }

    /// One thread, observed on its own: only a change to this thread re-renders the caller.
    func thread(_ id: UUID) -> ChatThread? {
        threadCells[id]?.value ?? nil
    }

    var selectedThread: ChatThread? {
        selectedThreadID.flatMap(thread)
    }

    var currentProject: Project? {
        if let selectedThread, let current = project(selectedThread.projectID) { return current }
        if let lastID = settings.lastProjectID, let last = project(lastID) { return last }
        // No selection (for example after relaunch): stay in the folder you last worked in
        // instead of jumping back to the first project ever added.
        if let recent = threads.filter({ !$0.isArchived && !$0.isInPanel }).max(by: { $0.updatedAt < $1.updatedAt }),
           let recentProject = project(recent.projectID) { return recentProject }
        return projects.first
    }

    private func rememberLastProject(_ id: UUID) {
        guard project(id) != nil else { return }
        if settings.lastProjectID != id { settings.lastProjectID = id }
    }

    /// The project's top-level threads, in sidebar order: the settled ones last, latest
    /// settled first. A helper whose parent is among them sits under that parent instead
    /// (see `helpers(of:)`); one whose parent is gone or archived stands on its own.
    func threads(in project: Project) -> [ChatThread] {
        let shown = (threadIndex.projects[project.id] ?? []).map { threads[$0] }
            .filter { !$0.isArchived && !$0.isInPanel }
        let shownIDs = Set(shown.map(\.id))
        return shown
            .filter { $0.parentThreadID.map { !shownIDs.contains($0) } ?? true }
            .sorted { lhs, rhs in
                if lhs.isSettled != rhs.isSettled { return rhs.isSettled }
                if lhs.isSettled { return Self.settledFirst(lhs, rhs) }
                if lhs.isPinned != rhs.isPinned { return lhs.isPinned }
                switch (lhs.sortOrder, rhs.sortOrder) {
                case let (left?, right?) where left != right: return left < right
                // Threads created after a reorder have no place yet and stay on top.
                case (nil, .some): return true
                case (.some, nil): return false
                default: return lhs.createdAt > rhs.createdAt
                }
            }
    }

    /// Places a thread directly above or below another thread in the same project.
    /// Places a thread directly above or below another in its project. Returns whether the
    /// order changed, for a live drag that swaps one neighbour at a time.
    @discardableResult
    func moveThread(_ id: UUID, to targetID: UUID, placeAfter: Bool) -> Bool {
        guard id != targetID, let moving = thread(id), let target = thread(targetID),
              moving.projectID == target.projectID, let project = project(moving.projectID),
              !moving.isSettled, !target.isSettled else { return false }
        let before = threads(in: project).filter { !$0.isSettled }.map(\.id)
        var order = before
        order.removeAll { $0 == id }
        guard let index = order.firstIndex(of: targetID) else { return false }
        order.insert(id, at: placeAfter ? index + 1 : index)
        guard order != before else { return false }
        updateThreads(order) { position, thread in
            if thread.id == id { thread.isPinned = target.isPinned }
            thread.sortOrder = Double(position)
        }
        return true
    }

    /// Places a thread directly above or below another within the same group of the activity
    /// layout. Returns whether the order changed.
    @discardableResult
    func moveInActivity(_ id: UUID, to targetID: UUID, placeAfter: Bool, among peers: [UUID]) -> Bool {
        guard id != targetID, peers.contains(id), peers.contains(targetID) else { return false }
        var order = peers
        order.removeAll { $0 == id }
        guard let index = order.firstIndex(of: targetID) else { return false }
        order.insert(id, at: placeAfter ? index + 1 : index)
        guard order != peers else { return false }
        let calendar = Calendar.current
        updateThreads(order) { position, thread in
            thread.activityOrder = Double(position)
            thread.activityOrderDay = calendar.startOfDay(for: thread.updatedAt)
        }
        return true
    }

    var archivedThreads: [ChatThread] {
        threads.filter(\.isArchived).sorted { $0.updatedAt > $1.updatedAt }
    }

    /// The order among settled threads: the one settled last comes first.
    nonisolated static func settledFirst(_ lhs: ChatThread, _ rhs: ChatThread) -> Bool {
        (lhs.settledAt ?? lhs.updatedAt) > (rhs.settledAt ?? rhs.updatedAt)
    }

    /// Threads in the order the sidebar shows them, for keyboard navigation: each thread
    /// followed by the helpers under it, unless they are folded away.
    var sidebarThreads: [ChatThread] {
        projects.filter(\.isExpanded).flatMap { project in
            threads(in: project).flatMap { thread in
                thread.foldsHelpers || thread.isSettled ? [thread] : [thread] + helpers(of: thread.id)
            }
        }
    }

    /// The helpers that sit under a thread in the sidebar: spawned from it and not archived.
    /// Hands are there from the moment they go out, still in the lead's panel or not, so a
    /// chat with a team at work shows it; a helper popped out into its own panel is the
    /// one kind that leaves the list. Newest first, like the threads around them.
    func helpers(of parentID: UUID) -> [ChatThread] {
        children(of: parentID)
            .filter { !$0.isArchived && (!$0.isInPanel || $0.isCrewHand) }
            .sorted { $0.createdAt > $1.createdAt }
    }

    /// The hands a lead still shows in its floating panel, in the order they were sent out.
    /// The same list as `crewHands(of:)`, read through this thread's own cells: a write to
    /// a hand of some other chat never re-renders this one.
    func panelHands(of parentID: UUID) -> [ChatThread] {
        children(of: parentID)
            .filter { $0.isInPanel && !$0.isArchived && $0.isCrewHand }
            // A hand sits in its roster place, in the order they were sent out.
            .sorted {
                let places = ($0.crew?.index ?? Int.max, $1.crew?.index ?? Int.max)
                return places.0 == places.1 ? $0.createdAt < $1.createdAt : places.0 < places.1
            }
    }

    /// The helper a thread spawned and still shows in its floating panel, if any, read
    /// through this thread's own cells. Hands have a panel of their own (`panelHands(of:)`).
    func panelSubagent(of parentID: UUID) -> ChatThread? {
        children(of: parentID).first { $0.isInPanel && !$0.isArchived && !$0.isCrewHand }
    }

    func toggleHelpersFold(_ parentID: UUID) {
        updateThread(parentID) { $0.foldsHelpers.toggle() }
    }

    func runtime(for id: UUID) -> ThreadRuntime {
        if let runtime = runtimes[id] { return runtime }
        let runtime = ThreadRuntime(threadID: id, app: self)
        runtimes[id] = runtime
        return runtime
    }

    func existingRuntime(for id: UUID) -> ThreadRuntime? {
        runtimes[id]
    }

    /// Every runtime alive right now, for checks that span chats (a merge still in flight at quit).
    var liveRuntimes: [ThreadRuntime] { Array(runtimes.values) }

    /// Decodes threads' histories ahead of time, off the main thread, so opening one costs no
    /// parse on the click. Threads already open have their history in memory and are skipped.
    func warmDocuments(_ ids: [UUID]) {
        let cold = ids.filter { runtimes[$0] == nil }
        guard !cold.isEmpty else { return }
        DocumentPrefetch.shared.warm(cold)
    }

    /// The rows either side of the selection within the thread's project, for the
    /// arrow keys and the next click.
    private func warmNeighbors(of id: UUID?) {
        warmDocuments(neighbors(of: id))
    }

    /// The rows either side of a thread within its project: what the arrow keys and the
    /// next click are most likely to open.
    func neighbors(of id: UUID?) -> [UUID] {
        guard let id, let thread = thread(id), let project = project(thread.projectID) else { return [] }
        let order = threads(in: project)
        guard let index = order.firstIndex(where: { $0.id == id }) else { return [] }
        var neighbors: [UUID] = []
        if index > 0 { neighbors.append(order[index - 1].id) }
        if index + 1 < order.count { neighbors.append(order[index + 1].id) }
        return neighbors
    }

    /// A thread left alone for ten minutes gives its agent process back, and its history
    /// with it when nothing but the saved document would be lost: the runtime goes, and
    /// opening the thread again reads it back from disk, with the provider session id on
    /// the thread resuming the conversation. A thread with a draft, an edit under way, work
    /// still owed to it or a terminal keeps its runtime, session stopped.
    private func scheduleIdleSessionStop(leaving id: UUID?) {
        if let selectedThreadID {
            idleSessionStops.removeValue(forKey: selectedThreadID)?.cancel()
        }
        guard let id, runtimes[id] != nil else { return }
        idleSessionStops[id]?.cancel()
        idleSessionStops[id] = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(600))
                guard !Task.isCancelled, let self, self.selectedThreadID != id, let runtime = self.runtimes[id] else { return }
                // A turn still working keeps its session; check again after the next interval.
                guard !runtime.isRunning else { continue }
                runtime.stopSession()
                self.idleSessionStops[id] = nil
                self.releaseIdleRuntime(id)
                return
            }
        }
    }

    /// Drops an idle thread's runtime once its document is on its way to disk. Returns
    /// whether it did; a runtime holding anything the document does not stays.
    @discardableResult
    func releaseIdleRuntime(_ id: UUID) -> Bool {
        guard let runtime = runtimes[id], selectedThreadID != id, let thread = thread(id),
              // A thread shown beside the selection, in its panel, is on screen.
              thread.parentThreadID == nil || thread.parentThreadID != selectedThreadID,
              runtime.holdsOnlyPersistedState, !sessionsToRelease.contains(id),
              autoContinue.resumesAt[id] == nil, terminals.sessions(for: id).isEmpty,
              runningHostedHands(of: id) == 0 else { return false }
        // The save is queued before the runtime goes; a read that follows waits for it
        // (see `Storage.readDocument`), so the thread can never come back older than it left.
        runtime.saveNow()
        runtime.cancelPersistence()
        runtimes[id] = nil
        return true
    }

    var workingDirectory: String? {
        guard let thread = selectedThread, let project = project(thread.projectID) else { return nil }
        return thread.worktreePath ?? project.path
    }

    // MARK: - Projects

    @discardableResult
    func addProject(at url: URL) -> Project {
        let path = url.standardizedFileURL.path
        if let existing = projects.first(where: { $0.path == path }) { return existing }
        var project = Project(name: url.lastPathComponent, path: path)
        project.scripts = ProjectFile.scripts(at: url)
        project.studioloLink = studioloAutoLink(forPath: path)
        projects.append(project)
        scheduleSave()
        return project
    }

    func chooseProjectFolder() {
        guard let added = addProjectsFromPanel() else { return }
        newThread(in: added)
    }

    /// The folder panel: every chosen folder joins the sidebar as a project and the last
    /// one added comes back, or nil when the panel was cancelled. Nothing else is made
    /// here, so a caller that already has a thread on screen (the new-thread prompt) can
    /// move that thread into the folder instead of leaving a second empty thread behind.
    @discardableResult
    func addProjectsFromPanel() -> Project? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.prompt = "Add project"
        panel.message = "Choose a folder for your agents to work in."
        guard panel.runModal() == .OK else { return nil }
        let known = Set(projects.map(\.id))
        var added: Project?
        for url in panel.urls { added = addProject(at: url) }
        if let added, !known.contains(added.id) { offerStudioloArchive(for: added.id) }
        return added
    }

    func updateProject(_ id: UUID, _ change: (inout Project) -> Void) {
        guard let index = projects.firstIndex(where: { $0.id == id }) else { return }
        var project = projects[index]
        change(&project)
        guard project != projects[index] else { return }
        projects[index] = project
        scheduleSave()
    }

    func removeProject(_ project: Project) {
        for thread in threads where thread.projectID == project.id {
            discardThreadState(thread)
        }
        threads.removeAll { $0.projectID == project.id }
        projects.removeAll { $0.id == project.id }
        if settings.lastProjectID == project.id { settings.lastProjectID = nil }
        if selectedThread == nil { selectedThreadID = nil }
        scheduleSave()
    }

    // MARK: - Threads

    /// Launch lands in a chat, never on the empty page: the newest thread nothing has been
    /// sent in yet, in the project you last worked in, or a fresh one there. Reusing an
    /// unused thread keeps a launch from leaving a "New thread" row behind every time.
    func openThreadOnLaunch() {
        guard let project = currentProject else { return }
        let unused = threads
            .filter {
                $0.projectID == project.id && !$0.isArchived && !$0.isSettled && !$0.isHelper
                    && $0.lastStatus == nil && !$0.hasCustomTitle && $0.title == ChatThread.untitled
            }
            .max { $0.createdAt < $1.createdAt }
        if let unused {
            // The row may be older than the pair or model chosen since: nothing was sent in
            // it, so it starts over on what a fresh chat would open on today.
            let fresh = makeThread(in: project, carrying: nil)
            updateThread(unused.id) { thread in
                if thread.provider != fresh.provider {
                    thread.providerSessionID = nil
                    thread.providerResumeAt = nil
                }
                thread.provider = fresh.provider
                thread.model = fresh.model
                thread.effort = fresh.effort
                thread.fastMode = fresh.fastMode
                thread.crewPairID = fresh.crewPairID
                thread.crewEnabled = fresh.crewEnabled
            }
            selectedThreadID = unused.id
        } else {
            newThread(in: project)
        }
    }

    @discardableResult
    func newThread(in project: Project? = nil, workspace: WorkspaceMode? = nil) -> ChatThread? {
        guard let project = project ?? currentProject else {
            chooseProjectFolder()
            return nil
        }
        let thread = makeThread(in: project, carrying: selectedThread)
        threads.append(thread)
        updateProject(project.id) { $0.isExpanded = true }
        rememberLastProject(project.id)
        selectedThreadID = thread.id
        scheduleSave()
        if (workspace ?? settings.defaultWorkspaceMode) == .worktree {
            Task { await createWorktree(for: thread.id) }
        }
        return thread
    }

    /// Copies conversation context into an unsent thread with independent provider state.
    @discardableResult
    func forwardThread(_ sourceID: UUID) async -> ChatThread? {
        guard let source = thread(sourceID), project(source.projectID) != nil else { return nil }
        let sourceRuntime = runtime(for: sourceID)
        await sourceRuntime.ensureLoaded()
        // The source can be renamed or removed while its history loads.
        guard let resolved = thread(sourceID), let project = project(resolved.projectID) else { return nil }
        let continuation = ThreadContinuation.capture(
            sourceID: sourceID,
            title: resolved.title,
            document: sourceRuntime.snapshotForPersistence()
        )
        var thread = makeThread(in: project, carrying: resolved)
        thread.title = "Continued: " + resolved.title
        thread.hasCustomTitle = true
        threads.append(thread)
        updateProject(project.id) { $0.isExpanded = true }
        rememberLastProject(project.id)
        let newRuntime = runtime(for: thread.id)
        guard await newRuntime.installContinuation(continuation), self.thread(thread.id) != nil else { return nil }
        selectedThreadID = thread.id
        scheduleSave()
        return thread
    }

    /// A chat as it would start in `project`: on `current`'s provider, model and pair when
    /// made from an open chat, otherwise on what was last used. Not yet in `threads`.
    private func makeThread(in project: Project, carrying current: ChatThread?) -> ChatThread {
        let installed = ProviderKind.allCases.filter { providers.status($0).isInstalled && settings.isEnabled($0) }
        // The project's own pair, when it has one, sets the provider before anything else.
        let projectPair = CrewPair.projectPair(project.crewPairID, crewOn: settings.crewEnabled, in: crewPickerPairs)
        // With no chat to carry from (a fresh launch), the pair last entered from the
        // picker is where the new chat starts. Before any pair has been entered since
        // this was remembered, the chat last worked in stands in: its pair if it led one,
        // and none if it had left its pair for a model of its own.
        let lastPairID = settings.lastCrewPairID
            ?? threads.filter { !$0.isHelper }.max { $0.updatedAt < $1.updatedAt }?.crewPairID
        let lastPair = current == nil && settings.crewEnabled
            ? lastPairID.flatMap { id in crewPickerPairs.first { $0.id == id } }
            : nil
        var provider = projectPair?.provider ?? current?.provider ?? lastPair?.provider ?? settings.defaultProvider
        if !installed.isEmpty, !installed.contains(provider) { provider = installed[0] }
        // A chat that starts on the project's pair takes that pair's lead model. A pair with
        // no lead model keeps the current chat's model, as the picker does.
        let carriesModel = projectPair?.orchestratorModel == nil && current?.provider == provider
        let model = carriesModel ? current?.model : providers.defaultModel(for: provider)?.id
        let preference = settings.preference(for: provider, model: model)
        let effort = carriesModel ? current?.effort : (preference.effort ?? settings.lastEffort(for: provider))
        let fastMode = carriesModel ? (current?.fastMode ?? false) : preference.fastMode
        var thread = ChatThread(
            projectID: project.id,
            provider: provider,
            model: model,
            effort: effort,
            runtimeMode: settings.defaultRuntimeMode,
            fastMode: fastMode
        )
        // A chat made from one that leads a pair carries the pair along with the model.
        // With no chat to carry from, the last pair is where the new chat starts, on its
        // lead model and effort, provided its provider is still installed. The project's
        // pair wins over all of that: the user set it as the rule for this project, so
        // every new chat here starts on it, leading. Everywhere else whether the chat leads
        // is "New chats lead a team" in Settings › Crew, which `ChatThread.init` read.
        if let pair = projectPair {
            let lead = crewLead(of: pair, for: thread)
            thread.model = lead.model
            thread.effort = lead.effort
            thread.fastMode = lead.fastMode
            thread.crewPairID = pair.id
            thread.crewEnabled = true
        } else if carriesModel {
            thread.crewPairID = current?.crewPairID
        } else if let pair = lastPair, pair.provider == provider {
            let lead = crewLead(of: pair, for: thread)
            thread.model = lead.model
            thread.effort = lead.effort
            thread.fastMode = lead.fastMode
            thread.crewPairID = pair.id
        }
        return thread
    }

    func updateThread(_ id: UUID, _ change: (inout ChatThread) -> Void) {
        guard let index = threadIndex.positions[id] else { return }
        var thread = threads[index]
        change(&thread)
        guard thread != threads[index] else { return }
        threads[index] = thread
        scheduleSave()
    }

    /// Moves a thread to another project's folder: what the old folder's session and
    /// worktree left on it goes, the destination project opens so the row has a place to
    /// land, the last-worked-in project follows, and the sidebar is asked to bring the
    /// moved row into view. The runtime stops first, so no turn carries on against a
    /// folder the chat has left. Wrap the call in `withAnimation` to make the move glide
    /// in the sidebar; the model itself never animates.
    func moveThread(_ id: UUID, toProject project: Project) {
        existingRuntime(for: id)?.stopSession()
        updateThread(id) { thread in
            thread.projectID = project.id
            thread.worktreePath = nil
            thread.branch = nil
            thread.providerSessionID = nil
            thread.providerResumeAt = nil
        }
        updateProject(project.id) { $0.isExpanded = true }
        rememberLastProject(project.id)
        sidebarRevealThreadID = id
    }

    private func updateThreads(_ ids: [UUID], _ change: (Int, inout ChatThread) -> Void) {
        var updated = threads
        var changed = false
        for (position, id) in ids.enumerated() {
            guard let index = threadIndex.positions[id] else { continue }
            let previous = updated[index]
            change(position, &updated[index])
            changed = changed || previous != updated[index]
        }
        guard changed else { return }
        threads = updated
        scheduleSave()
    }

    func rename(_ id: UUID, to title: String) {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        updateThread(id) {
            $0.title = trimmed
            $0.hasCustomTitle = true
        }
    }

    func archive(_ id: UUID) {
        // The helpers under a thread go with it, and come back with it.
        for helper in helpers(of: id) { archive(helper.id) }
        if selectedThreadID == id { selectNeighbor(of: id) }
        existingRuntime(for: id)?.stopSession()
        releaseCrewCopy(of: id)
        updateThread(id) {
            $0.isArchived = true
            $0.isPinned = false
            // Restored, it is a thread again, not a settled one.
            $0.isSettled = false
            $0.settledAt = nil
        }
        // Its file checkpoints go now rather than when it is deleted: an archive kept for
        // good would otherwise hold every turn's hidden refs in the repository forever.
        // Restored, it reads its turns as having none (see `ThreadRuntime.pruneCheckpoints`,
        // which also takes whatever a turn the archive stopped writes after this).
        if let projectID = thread(id)?.projectID, let project = project(projectID) {
            let git = Git(project.path)
            Task.detached(priority: .utility) { await git.deleteCheckpoints(thread: id) }
        }
        // An archived thread is out of the list, so its unread mark leaves the dock with it.
        updateDockBadge()
    }

    func unarchive(_ id: UUID) {
        updateThread(id) { $0.isArchived = false }
        for helper in threads where helper.parentThreadID == id && helper.isArchived {
            updateThread(helper.id) { $0.isArchived = false }
        }
        // Back in the list, an unread thread counts again.
        updateDockBadge()
    }

    /// Marks a thread finished: it drops to the bottom of its list as a small grey row,
    /// its helpers folded away with it, and stays a click away until it is reopened. Its
    /// session keeps running if it is; only the sidebar's view of it changes. The settle
    /// note sounds here unless the caller already played it on the click.
    func settle(_ id: UUID, sounds: Bool = true) {
        guard let thread = thread(id), !thread.isSettled, !thread.isHelper else { return }
        let now = Date.now
        updateThread(id) {
            $0.isSettled = true
            $0.settledAt = now
            $0.isPinned = false
            // Done with it means read.
            $0.hasUnread = false
        }
        for helper in helpers(of: id) {
            updateThread(helper.id) {
                $0.isSettled = true
                $0.settledAt = now
            }
        }
        updateDockBadge()
        if sounds, settings.settleSound { SettleChime.play() }
        // A settled thread stays settled across relaunch: write now, not on the debounce.
        saveLibrary()
    }

    /// Brings a settled thread back among the others, on top: reopened counts as activity.
    func reopen(_ id: UUID) {
        guard let thread = thread(id), thread.isSettled else { return }
        let now = Date.now
        updateThread(id) {
            $0.isSettled = false
            $0.settledAt = nil
            $0.updatedAt = now
            // A place dragged into before it settled no longer applies.
            $0.activityOrder = nil
            $0.activityOrderDay = nil
        }
        updateThreads(children(of: id).filter(\.isSettled).map(\.id)) { _, helper in
            helper.isSettled = false
            helper.settledAt = nil
        }
        saveLibrary()
    }

    /// Settles the thread with the sidebar's motion: the rows around it make room as it goes.
    func settleAnimated(_ id: UUID, sounds: Bool = true) {
        withAnimation(Chrome.settleFlight) { settle(id, sounds: sounds) }
    }

    /// Reopens the thread with the sidebar's motion.
    func reopenAnimated(_ id: UUID) {
        withAnimation(Chrome.settleFlight) { reopen(id) }
    }

    /// A settled thread that starts a turn is at work again, so it comes back up the list.
    func reopenIfSettled(_ id: UUID) {
        guard thread(id)?.isSettled == true else { return }
        reopenAnimated(id)
    }

    /// What the menu's, the shortcut's and the palette's finish command does to the selected
    /// thread: settles or archives it as Settings chose, and reopens a settled one.
    var finishActionTitle: String {
        if settings.threadFinishAction == .settle, selectedThread?.isSettled == true { return "Reopen thread" }
        return "\(settings.threadFinishAction.title) thread"
    }

    var finishActionSymbol: String {
        if settings.threadFinishAction == .settle, selectedThread?.isSettled == true { return "arrow.uturn.backward" }
        return settings.threadFinishAction == .settle ? "checkmark.circle" : "archivebox"
    }

    func finishSelectedThread() {
        guard let id = selectedThreadID, let thread = thread(id) else { return }
        switch settings.threadFinishAction {
        // A helper has no place of its own to settle into; it goes the archive's way.
        case .settle where thread.isHelper, .archive:
            withAnimation(Chrome.panelSlide) { archive(id) }
        case .settle:
            let request = FinishRequest(threadID: id, reopens: thread.isSettled)
            finishRequest = request
            Task {
                // No row took it up (the sidebar is hidden, say): the thread just moves.
                try? await Task.sleep(for: .milliseconds(80))
                guard finishRequest == request else { return }
                finishRequest = nil
                if request.reopens { reopenAnimated(id) } else { settleAnimated(id) }
            }
        }
    }

    /// Settles the selected thread, or reopens it when it is settled already: the shortcut's
    /// toggle, whatever General chose for Finish. A helper has no place of its own to settle
    /// into, so it stays as it is.
    func settleSelectedThread() {
        guard let id = selectedThreadID, let thread = thread(id), !thread.isHelper else { return }
        let request = FinishRequest(threadID: id, reopens: thread.isSettled)
        finishRequest = request
        Task {
            // No row took it up (the sidebar is hidden, say): the thread just moves.
            try? await Task.sleep(for: .milliseconds(80))
            guard finishRequest == request else { return }
            finishRequest = nil
            if request.reopens { reopenAnimated(id) } else { settleAnimated(id) }
        }
    }

    /// Asks before archiving the selected thread: its row shows the question, with
    /// Archive the answer Return gives and Delete beside it.
    func askToArchiveSelectedThread() {
        guard let id = selectedThreadID, thread(id) != nil else { return }
        // Nothing in it, nothing typed for it: there is nothing to keep, so it goes the way
        // an empty tab does, with no question. A history still being read is not "nothing",
        // and nor is one that did not read: it is kept aside, or still in its file unread.
        if let runtime = existingRuntime(for: id), !runtime.isLoadingHistory, runtime.loadProblem == nil, runtime.turns.isEmpty,
           runtime.continuation == nil, runtime.draft.isEmpty, runtime.followUps.isEmpty, !runtime.isRunning {
            withAnimation(Chrome.panelSlide) { delete(id) }
            return
        }
        let request = ArchiveRequest(threadID: id)
        archiveRequest = request
        Task {
            // No row to ask on (the sidebar is hidden, say): the thread is archived, which
            // is what Return would have done.
            try? await Task.sleep(for: .milliseconds(80))
            guard archiveRequest == request else { return }
            archiveRequest = nil
            withAnimation(Chrome.panelSlide) { archive(id) }
        }
    }

    /// A settle or reopen asked for away from the row: the menu, the shortcut, the palette.
    /// The thread's row takes it up and settles the way a click on its check does, pop, note
    /// and glide included.
    struct FinishRequest: Equatable {
        let id = UUID()
        let threadID: UUID
        let reopens: Bool
    }

    var finishRequest: FinishRequest?

    /// An archive asked for with the shortcut, which the thread's row takes up as a
    /// question on itself.
    struct ArchiveRequest: Equatable {
        let id = UUID()
        let threadID: UUID
    }

    var archiveRequest: ArchiveRequest?

    func delete(_ id: UUID, removeWorktree: Bool = false) {
        guard let thread = thread(id) else { return }
        // A helper still in its panel goes with the thread it belongs to.
        for helper in children(of: id) { delete(helper.id) }
        if selectedThreadID == id { selectNeighbor(of: id) }
        // A hand deleted mid-job reports to its lead as stopped, so the batch it was in
        // settles and the lead's waiting hands and merge go on rather than waiting for it.
        if thread.isCrewHand, thread.crew?.isFinished == false {
            finishCrewHand(id, status: .interrupted, summary: nil)
        }
        discardThreadState(thread)
        threads.removeAll { $0.id == id }
        if let project = project(thread.projectID) {
            let git = Git(project.path)
            // A hand's copy of the checkout was Bottega's to make, so it always goes.
            let worktree = removeWorktree || thread.crew?.hasOwnCopy == true ? thread.worktreePath : nil
            Task {
                await git.deleteCheckpoints(thread: id)
                if let worktree { try? await git.removeWorktree(at: worktree) }
            }
        }
        updateDockBadge()
        saveLibrary()
    }

    /// Deletes every archived thread at once: histories, terminals and checkpoints go with them.
    /// Worktrees stay on disk, exactly as they do after a single delete.
    func deleteArchivedThreads() {
        let archived = threads.filter(\.isArchived)
        guard !archived.isEmpty else { return }
        // Helpers still in an archived thread's panel go with it.
        let archivedIDs = Set(archived.map(\.id))
        let removed = archived + threads.filter { helper in
            guard let parent = helper.parentThreadID else { return false }
            return archivedIDs.contains(parent) && !archivedIDs.contains(helper.id)
        }
        if let selectedThreadID, removed.contains(where: { $0.id == selectedThreadID }) {
            self.selectedThreadID = nil
        }
        var cleanup: [(directory: String, threadID: UUID, copy: String?)] = []
        cleanup.reserveCapacity(removed.count)
        for thread in removed {
            discardThreadState(thread, deletingDocument: false)
            if let project = project(thread.projectID) {
                cleanup.append((project.path, thread.id, thread.crew?.hasOwnCopy == true ? thread.worktreePath : nil))
            }
        }
        Storage.deleteDocuments(removed.map(\.id))
        Task {
            for item in cleanup {
                let git = Git(item.directory)
                await git.deleteCheckpoints(thread: item.threadID)
                if let copy = item.copy { try? await git.removeWorktree(at: copy) }
            }
        }
        let removedIDs = Set(removed.map(\.id))
        threads.removeAll { removedIDs.contains($0.id) }
        updateDockBadge()
        scheduleSave()
    }

    // MARK: - Subagents

    /// Helpers whose panel closed while they worked: their session goes once the
    /// interrupted turn has finished (see turnFinished).
    @ObservationIgnored private var sessionsToRelease: Set<UUID> = []

    /// The helper a thread spawned and still shows in its floating panel, if any. Crew
    /// hands have a panel of their own (see `crewHands(of:)`).
    func subagent(of parentID: UUID) -> ChatThread? {
        panelSubagent(of: parentID)
    }

    /// Adds a thread the app made for another thread: a hand, say. The caller selects it
    /// or leaves it in a panel as it sees fit.
    func insertThread(_ thread: ChatThread) {
        threads.append(thread)
        scheduleSave()
    }

    /// Spawns a helper thread beside `parentID` and sends it `prompt` at once. The helper
    /// runs on its parent's provider, model, effort and permissions, so it works exactly as
    /// the chat that spawned it would, in the project folder itself: a merge lands on the
    /// main checkout, never in a thread's worktree. A parent already showing a helper hands
    /// the prompt to that one instead: sent now when it is idle, queued behind its running
    /// turn otherwise.
    @discardableResult
    func spawnSubagent(from parentID: UUID, title: String, prompt: String) -> ChatThread? {
        guard let parent = thread(parentID) else { return nil }
        if let existing = subagent(of: parentID) {
            let runtime = runtime(for: existing.id)
            if runtime.isRunning {
                runtime.enqueueFollowUp(text: prompt, attachments: [])
            } else {
                runtime.draft = ComposerDraft(text: prompt)
                runtime.send()
            }
            return existing
        }
        var thread = ChatThread(
            projectID: parent.projectID,
            provider: parent.provider,
            model: parent.model,
            effort: parent.effort,
            runtimeMode: parent.runtimeMode,
            fastMode: parent.fastMode
        )
        thread.parentThreadID = parentID
        thread.isInPanel = true
        thread.title = title
        thread.hasCustomTitle = true
        threads.append(thread)
        scheduleSave()
        let runtime = runtime(for: thread.id)
        runtime.draft = ComposerDraft(text: prompt)
        runtime.send()
        return thread
    }

    /// Closes a helper's panel: its turn stops, and it moves to the sidebar under the thread
    /// that spawned it, as a smaller row, so what it did stays a click away.
    func closeSubagent(_ id: UUID) {
        guard thread(id) != nil else { return }
        if let runtime = existingRuntime(for: id) {
            // A running turn ends as interrupted and releases its session once it has
            // (see turnFinished); an idle one has nothing to wait for.
            if runtime.isRunning {
                sessionsToRelease.insert(id)
                runtime.interrupt()
            } else {
                runtime.stopSession()
            }
        }
        updateThread(id) { $0.isInPanel = false }
    }

    private func discardThreadState(_ thread: ChatThread, deletingDocument: Bool = true) {
        idleSessionStops.removeValue(forKey: thread.id)?.cancel()
        runtimes[thread.id]?.cancelPersistence()
        runtimes[thread.id]?.stopSession()
        runtimes[thread.id] = nil
        terminals.closeAll(for: thread.id)
        if deletingDocument { Storage.deleteDocument(thread.id) }
    }

    private func selectNeighbor(of id: UUID) {
        let order = sidebarThreads
        guard let index = order.firstIndex(where: { $0.id == id }) else {
            selectedThreadID = nil
            return
        }
        let neighbor = order.indices.contains(index + 1) ? order[index + 1] : (index > 0 ? order[index - 1] : nil)
        selectedThreadID = neighbor?.id
    }

    // MARK: - Scripts

    /// The scripts every thread offers whatever its project, read from `bottega.json`
    /// in the app's Application Support folder. Written by `refreshGlobalScripts` and
    /// `saveGlobalScripts` alone (see `AppModel+Scripts.swift`).
    var globalScripts: [ProjectScript] = []
    /// Each scripts file as it was when last read, so a look at one nobody has touched
    /// since costs a stat and nothing else: the projects' by project, and the global one.
    @ObservationIgnored var projectScriptStamps: [UUID: ScriptFileStamp] = [:]
    @ObservationIgnored var globalScriptStamp: ScriptFileStamp?
    /// The scripts editor asked for from the Workflow menu, which the chat on screen takes
    /// up and shows over itself (see `requestScriptsEditor`).
    var scriptsEditorRequest: ScriptsEditorRequest?

    var topScript: ProjectScript? {
        guard let selectedThreadID, let thread = thread(selectedThreadID) else { return nil }
        return project(thread.projectID)?.scripts.first
    }

    /// Runs a script in the thread's terminal. The files are read again first where they
    /// changed since the menu that offered the script was built, so it runs as its file
    /// has it now, and not at all once the file has dropped it.
    func runScript(_ script: ProjectScript, threadID: UUID) {
        refreshScripts(for: threadID)
        guard let current = scripts(for: threadID).first(where: { $0.id == script.id }),
              let thread = thread(threadID), let project = project(thread.projectID) else { return }
        terminals.run(current, threadID: threadID, directory: thread.worktreePath ?? project.path)
        runtime(for: threadID).isTerminalVisible = true
    }

    func selectThread(offset: Int) {
        let order = sidebarThreads
        guard !order.isEmpty else { return }
        guard let current = selectedThreadID, let index = order.firstIndex(where: { $0.id == current }) else {
            selectedThreadID = order.first?.id
            return
        }
        selectedThreadID = order[(index + offset + order.count) % order.count].id
    }

    func selectThread(number: Int) {
        let order = sidebarThreads
        guard order.indices.contains(number - 1) else { return }
        selectedThreadID = order[number - 1].id
    }

    func createWorktree(for threadID: UUID) async {
        guard let thread = thread(threadID), thread.worktreePath == nil, let project = project(thread.projectID) else { return }
        let git = Git(project.path)
        guard await git.isRepository(), await git.hasCommits() else {
            alert = AppAlert(title: "Worktree unavailable", message: "New worktrees need a git repository with at least one commit.")
            return
        }
        let suffix = String(threadID.uuidString.lowercased().prefix(8))
        let folder = project.name.replacingOccurrences(of: " ", with: "-").lowercased()
        let path = Storage.worktreesDirectory.appendingPathComponent("\(folder)-\(suffix)").path
        let branch = "bottega/\(suffix)"
        let base = await git.status()?.branch
        do {
            try await git.addWorktree(at: path, branch: branch, base: base)
            updateThread(threadID) {
                $0.worktreePath = path
                $0.branch = branch
            }
            // Global scripts count as much as the project's own; the files are read again
            // first, so a script just added to either runs too.
            refreshScripts(for: threadID)
            for script in scripts(for: threadID) where script.runOnWorktreeCreate {
                terminals.run(script, threadID: threadID, directory: path)
                runtime(for: threadID).isTerminalVisible = true
            }
        } catch {
            alert = AppAlert(title: "Could not create a worktree", message: error.localizedDescription)
        }
    }

    // MARK: - Attention

    /// Whether a thread is on screen right now: the selected thread itself, a helper in
    /// its parent's floating panel, or a sidebar helper shown inline beneath its selected
    /// parent. Sidebar helpers count only while the parent is unfolded (`foldsHelpers == false`)
    /// and not settled, which is the same condition `SidebarView` uses to show them inline.
    private func isOnScreen(_ thread: ChatThread) -> Bool {
        if selectedThreadID == thread.id { return true }
        guard let parent = thread.parentThreadID, parent == selectedThreadID else { return false }
        if thread.isInPanel { return !thread.isArchived }
        guard !thread.isArchived, let parentThread = self.thread(parent) else { return false }
        return !parentThread.isArchived && !parentThread.isInPanel
            && !parentThread.foldsHelpers && !parentThread.isSettled
    }

    func threadNeedsAttention(_ id: UUID) {
        guard let thread = thread(id) else { return }
        // Already in view with the app frontmost: the approval or question is on
        // screen, so no banner and no dock bounce. Seen too, so no unread mark is
        // left behind to stick on the dock.
        if NSApp.isActive && isOnScreen(thread) {
            markRead(id)
            return
        }
        notify(threadID: id, title: thread.title, body: "Waiting for your decision.")
        NSApp.requestUserAttention(.informationalRequest)
    }

    /// A turn ended. `continues` means a queued message picks up right away, so the agent is
    /// still at work and neither the chime nor a notification claims otherwise.
    func turnFinished(_ id: UUID, status: TurnStatus, continues: Bool) {
        // The thread itself selected, or its panel helper or Crew hand on screen with
        // the selected parent, while the app is frontmost: in view.
        let onScreen = thread(id).map(isOnScreen) ?? false
        let isVisible = NSApp.isActive && onScreen
        let isHand = thread(id)?.isCrewHand == true
        updateThread(id) {
            $0.lastStatus = status
            $0.updatedAt = .now
            // Out of view the thread goes unread, for the dock; in view it is read,
            // which also clears an unread mark from an earlier finish while away so
            // the badge cannot stick around after the chat is back on screen. Hands
            // report to their lead and never mark themselves.
            if !isHand { $0.hasUnread = !isVisible }
        }
        updateDockBadge()
        // A helper panel closed mid-turn kept its session alive to finish stopping cleanly;
        // it has now.
        if sessionsToRelease.remove(id) != nil { existingRuntime(for: id)?.stopSession() }
        // A turn stopped by a spent usage limit waits for the reset and goes on by itself:
        // no chime and no "stopped with an error", the wait says what is happening.
        if autoContinue.turnFinished(id, status: status, continues: continues) { return }
        // A hand reports to its lead, which is the chat that chimes and notifies when
        // the whole job is done. A hand with another turn coming (its report, after its
        // budget ran out) reports at the end of that one.
        if isHand, let hand = thread(id) {
            if !continues { crewHandTurnFinished(hand, status: status) }
            return
        }
        guard !continues else { return }
        // A stop the user asked for needs no chime; the agent finishing on its own gets one,
        // whether or not the thread is in view.
        let chimed = settings.chimeWhenFinished && status != .interrupted
        if chimed { FinishChime.play() }
        guard !isVisible, settings.notifyWhenFinished, let thread = thread(id) else { return }
        let body = switch status {
        case .completed: "Finished."
        case .interrupted: "Stopped."
        case .failed: "Stopped with an error."
        case .running: ""
        }
        // The chime has already sounded, so the banner stays quiet rather than doubling it.
        notify(threadID: id, title: thread.title, body: body, sound: chimed ? nil : .default)
    }

    func markRead(_ id: UUID) {
        guard thread(id)?.hasUnread == true else { return }
        updateThread(id) { $0.hasUnread = false }
        updateDockBadge()
    }

    /// Several threads read at once cost one write to the library, so the cells and
    /// the sidebar see one change, not one per thread.
    func markRead(_ ids: [UUID]) {
        let wanted = Set(ids)
        var copy = threads
        var changed = false
        for index in copy.indices where wanted.contains(copy[index].id) && copy[index].hasUnread {
            copy[index].hasUnread = false
            changed = true
        }
        guard changed else { return }
        threads = copy
        scheduleSave()
        updateDockBadge()
    }

    func requestNotificationPermission() {
        guard !didRequestNotifications else { return }
        didRequestNotifications = true
        Task {
            _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
        }
    }

    func notify(threadID: UUID, title: String, body: String, sound: UNNotificationSound? = .default) {
        if NSApp.isActive, let thread = thread(threadID), isOnScreen(thread) { return }
        requestNotificationPermission()
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = sound
        content.userInfo = ["threadID": threadID.uuidString]
        let request = UNNotificationRequest(identifier: threadID.uuidString, content: content, trigger: nil)
        Task { try? await UNUserNotificationCenter.current().add(request) }
    }

    func updateDockBadge() {
        let unread = threads.count { $0.hasUnread && !$0.isArchived && !$0.isInPanel }
        NSApp.dockTile.badgeLabel = unread > 0 ? String(unread) : nil
    }

    // MARK: - Text generation

    /// What writes thread titles and commit messages: the provider Settings › Source control
    /// names, or with Automatic the thread's own provider ahead of the other. Only a provider
    /// switched on in Settings counts: one switched off is never checked at launch, so its
    /// sign-in reads as unknown rather than signed out, and going by sign-in alone would try
    /// it, and fail, where a signed-out one is passed over.
    func textEngine(preferring provider: ProviderKind?) -> TextGeneration.Engine? {
        func claude() -> TextGeneration.Engine? {
            guard settings.isEnabled(.claude), providers.status(.claude).auth != .signedOut,
                  let executable = providers.executable(for: .claude) else { return nil }
            return .claude(executable: executable, environment: providers.environment(for: .claude))
        }
        func codex() -> TextGeneration.Engine? {
            guard settings.isEnabled(.codex), providers.status(.codex).auth != .signedOut,
                  let executable = providers.executable(for: .codex),
                  let model = providers.defaultModel(for: .codex)?.id else { return nil }
            return .codex(executable: executable, environment: providers.environment(for: .codex), model: model)
        }
        switch settings.textGeneration {
        case .off: return nil
        case .claude: return claude()
        case .codex: return codex()
        case .automatic: return provider == .codex ? (codex() ?? claude()) : (claude() ?? codex())
        }
    }

    // MARK: - Persistence

    /// Whether library.json may be written. Not while the file is still there unread (see
    /// `libraryLoadProblem`): this run started without what it holds, and a save would put
    /// this run's library in its place.
    private var savesLibrary: Bool {
        libraryLoadProblem?.blocksSaving != true
    }

    func scheduleSave() {
        guard persistenceEnabled, savesLibrary else { return }
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            self?.saveLibrary()
        }
    }

    private func saveLibrary() {
        guard persistenceEnabled, savesLibrary else { return }
        var library = Library()
        library.projects = projects
        library.threads = threads
        let url = Storage.libraryURL
        DiskWriter.shared.encodeAndWrite(library, to: url)
    }

    /// Writes everything synchronously, for use while the app terminates.
    func saveBeforeQuit() {
        guard persistenceEnabled else { return }
        persistenceEnabled = false
        saveTask?.cancel()
        saveTask = nil
        if savesLibrary {
            var library = Library()
            library.projects = projects
            library.threads = threads
            DiskWriter.shared.writeSynchronously(library, to: Storage.libraryURL)
        }
        for runtime in runtimes.values {
            runtime.cancelPersistence()
            runtime.stopSession()
            // A thread only read since its last save is already on disk as it is.
            guard let snapshot = runtime.unsavedSnapshot() else { continue }
            DiskWriter.shared.writeSynchronously(snapshot, to: Storage.threadURL(runtime.threadID))
        }
        DiskWriter.shared.waitForQueuedWrites()
        terminals.terminateAll()
    }
}

/// Reads project scripts from `bottega.json` (or the upstream app's `droppy-code.json`) at the project root.
/// The global scripts every thread offers live in a file of the same shape in the app's Application Support folder.
enum ProjectFile {
    /// The names a scripts file goes by, in the order they are looked for. `bottega.json`
    /// first; a project the upstream app was used in may still carry its `droppy-code.json`,
    /// which reads the same, so that is the fallback.
    static let names = ["bottega.json", "droppy-code.json"]

    /// The global scripts' file: `bottega.json` in the app's own Application Support
    /// folder, which follows `AppInfo.storageName`, so Debug and Release keep a list each.
    static let globalFile = Storage.root.appendingPathComponent(ProjectFile.names[0])

    /// The scripts file at a folder's root, or nil when it has none.
    static func file(in directory: URL) -> URL? {
        names.map { directory.appendingPathComponent($0) }
            .first { FileManager.default.fileExists(atPath: $0.path) }
    }

    /// The global scripts' file, or nil while there is none.
    static var globalFileIfPresent: URL? {
        FileManager.default.fileExists(atPath: globalFile.path) ? globalFile : nil
    }

    static func scripts(at url: URL) -> [ProjectScript] {
        guard let scriptsFile = file(in: url) else { return [] }
        return scripts(inFile: scriptsFile) ?? []
    }

    /// The scripts a file lists, each marked as the file's. Nil when the file cannot be read
    /// as a JSON object with a list of scripts, as when it is caught half-way through a save,
    /// so the caller can keep the list it has rather than lose it; a file with no `scripts`
    /// key lists none.
    static func scripts(inFile file: URL) -> [ProjectScript]? {
        guard let data = try? Data(contentsOf: file), let json = JSONValue.parse(data), json.object != nil else { return nil }
        guard let entries = json["scripts"] else { return [] }
        guard let list = entries.array else { return nil }
        return list.compactMap { script -> ProjectScript? in
            guard let name = script["name"]?.string, let command = script["command"]?.string else { return nil }
            return ProjectScript(
                name: name,
                command: command,
                symbol: symbol(for: script["icon"]?.string),
                runOnWorktreeCreate: script["runOnWorktreeCreate"]?.bool ?? false,
                isFromFile: true
            )
        }
    }

    /// A list with a file's scripts as the file has them now. Each script the file lists
    /// keeps the id the list already gave it, matched by name among the file's own, so the
    /// menus keep their rows and a script's terminal tab stays its own; one that came from
    /// the file and is no longer in it goes. The scripts added in the app stay, after the
    /// file's, even one that shares a name with a script of the file: it is the user's,
    /// and the file's never stands in for it. The one exception is a script imported from
    /// the file before scripts were marked as the file's, left as it came (every field the
    /// file's), which the file takes back.
    static func merge(_ listed: [ProjectScript], into scripts: [ProjectScript]) -> [ProjectScript] {
        var unclaimed = scripts
        var merged: [ProjectScript] = []
        merged.reserveCapacity(listed.count + scripts.count)
        for var script in listed {
            let own = unclaimed.firstIndex { $0.isFromFile && $0.name == script.name }
            let imported = own == nil ? unclaimed.firstIndex { !$0.isFromFile && $0.matchesFileEntry(script) } : nil
            if let index = own ?? imported {
                script.id = unclaimed.remove(at: index).id
            }
            merged.append(script)
        }
        return merged + unclaimed.filter { !$0.isFromFile }
    }

    /// Writes a list into a scripts file in the shape `scripts(inFile:)` reads, keeping any
    /// other keys the file holds, at its top level and in each script's entry (matched by
    /// the name, or by the name the script had before, `formerNames`). A file that is there
    /// but does not read as a JSON object is left alone and the write fails, and so is one
    /// whose list holds entries `scripts(inFile:)` skipped: the editor never showed them,
    /// so hand edits that do not read yet are never written over.
    static func write(_ scripts: [ProjectScript], to file: URL, formerNames: [UUID: String] = [:]) throws {
        var root: [String: JSONValue] = [:]
        var existing: [String: [String: JSONValue]] = [:]
        if FileManager.default.fileExists(atPath: file.path) {
            guard let data = try? Data(contentsOf: file), let object = JSONValue.parse(data)?.object else {
                throw ScriptFileError.unreadable(file)
            }
            root = object
            if let listed = object["scripts"] {
                guard let entries = listed.array else { throw ScriptFileError.unreadableScripts(file, count: nil) }
                var unreadable = 0
                for entry in entries {
                    guard let fields = entry.object, let name = fields["name"]?.string, fields["command"]?.string != nil else {
                        unreadable += 1
                        continue
                    }
                    if existing[name] == nil { existing[name] = fields }
                }
                guard unreadable == 0 else { throw ScriptFileError.unreadableScripts(file, count: unreadable) }
            }
        }
        let entries = scripts.map { script -> JSONValue in
            var entry = existing[script.name] ?? formerNames[script.id].flatMap { existing[$0] } ?? [:]
            entry["name"] = .string(script.name)
            entry["command"] = .string(script.command)
            entry["icon"] = script.symbol != ProjectScript.defaultSymbol ? .string(script.symbol) : nil
            entry["runOnWorktreeCreate"] = script.runOnWorktreeCreate ? .bool(true) : nil
            return .object(entry)
        }
        root["scripts"] = JSONValue.array(entries)
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONValue.object(root).data(pretty: true).write(to: file, options: .atomic)
    }

    /// A file's `icon` as the script's SF Symbol. The five words the upstream app knew keep
    /// their marks; anything else is taken as a symbol's own name, such as `hammer.fill`,
    /// and a name this Mac has no symbol for falls back to the default mark.
    static func symbol(for icon: String?) -> String {
        switch icon {
        case "test": "checkmark.seal"
        case "lint": "wand.and.stars"
        case "configure": "gearshape"
        case "build": "hammer"
        case "debug": "ant"
        default: ProjectScript.resolvedSymbol(icon ?? "")
        }
    }
}
