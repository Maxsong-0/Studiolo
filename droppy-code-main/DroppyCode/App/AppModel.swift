import AppKit
import SwiftUI
import UserNotifications

struct AppAlert: Identifiable {
    let id = UUID()
    var title: String
    var message: String
}

/// One value under its own observation. A view that reads a thread or project through
/// its cell depends on that item alone, so a change to any other item never re-renders it.
@MainActor
@Observable
final class ObservedValue<Value> {
    var value: Value

    init(_ value: Value) {
        self.value = value
    }
}

/// The app's library of projects and threads, plus everything that spans threads.
@MainActor
@Observable
final class AppModel {
    let settings: AppSettings
    let providers: ProviderRegistry
    let terminals: TerminalStore
    let sidebar: SidebarLayout
    /// Picks chats back up once a spent usage limit resets, with the setting on.
    @ObservationIgnored private(set) var autoContinue: AutoContinue!
    /// How often each Droppy-run head has been started again after failing at the door
    /// (see `hydraHeadTurnFinished`); a head gets two more goes.
    @ObservationIgnored var hydraHeadRetries: [UUID: Int] = [:]

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
    /// on the rest of the library. A head's status write leaves every other chat alone.
    @ObservationIgnored private var childCells: [UUID: ObservedValue<[UUID]>] = [:]
    /// Whether one thread is the selected one, observed on its own: a sidebar row reads its
    /// flag here rather than `selectedThreadID`, so a selection change re-renders the row
    /// that lost it and the row that gained it, not every row in the list.
    @ObservationIgnored private var selectionCells: [UUID: ObservedValue<Bool>] = [:]
    var selectedThreadID: UUID? {
        didSet {
            if oldValue != selectedThreadID {
                if let oldValue { selectionCells[oldValue]?.value = false }
                if let selectedThreadID { selectionCell(selectedThreadID).value = true }
            }
            if let selectedThreadID {
                // A helper or head under the selected thread is on screen with it, so
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
    var isCommandPalettePresented = false
    var alert: AppAlert?

    @ObservationIgnored private var runtimes: [UUID: ThreadRuntime] = [:]
    @ObservationIgnored private var idleSessionStops: [UUID: Task<Void, Never>] = [:]
    @ObservationIgnored private var saveTask: Task<Void, Never>?
    @ObservationIgnored private var persistenceEnabled = true
    @ObservationIgnored private var didRequestNotifications = false

    init() {
        settings = AppSettings()
        providers = ProviderRegistry(settings: settings)
        terminals = TerminalStore()
        sidebar = SidebarLayout()
        let library = Storage.loadLibrary()
        projects = library.projects
        threads = library.threads
        for index in threads.indices where threads[index].lastStatus == .running {
            threads[index].lastStatus = .interrupted
        }
        // A head at work when the app last quit went with its session.
        for index in threads.indices where threads[index].hydra?.status == .running {
            threads[index].hydra?.status = .stopped
            threads[index].hydra?.finishedAt = .now
        }
        // Property observers stay quiet inside an initializer, so the cells are built here once.
        Self.write(Self.sync(&projectCells, with: projects))
        Self.write(Self.sync(&threadCells, with: threads))
        syncChildCells()
        threadIndex = ThreadIndex(threads)
        autoContinue = AutoContinue(app: self)
        if let lastID = settings.lastProjectID, project(lastID) == nil {
            settings.lastProjectID = nil
        }
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
    /// even with nothing under it, because that is exactly the cell a chat with no heads
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
        // The threads most likely to be opened first decode in the background from the start,
        // so the first click into a conversation never parses its history on the main thread.
        let recent = threads.filter { !$0.isArchived && !$0.isInPanel }.sorted { $0.updatedAt > $1.updatedAt }.prefix(12).map(\.id)
        warmDocuments(recent)
        await settings.loadKeychainInBackground()
        await LoginEnvironment.load()
        // After the login environment, so git runs with the user's PATH.
        sweepHydraCopies()
        await providers.refreshAll()
        // The catalogs a fresh install starts without, or with only a seed of: one CLI and
        // two HTTPS calls, overlapped. Claude's brings its slash commands too, so the picker
        // and the composer are complete before any turn.
        await withTaskGroup(of: Void.self) { group in
            for provider in [ProviderKind.claude, .codex, .deepseek, .meta] where providers.status(provider).isInstalled {
                group.addTask { await self.providers.loadCatalog(provider) }
            }
        }
        if providers.status(.zai).isInstalled {
            await providers.loadCatalog(.zai)
        }
        // Last, once nothing the first click needs is waiting on the disk: the attachment
        // files no thread refers to any more go, a day after they were written.
        guard !WebsiteCaptures.isEnabled else { return }
        await Storage.sweepOrphanedAttachments()
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
    /// Heads are there from the moment they go out, still in the lead's panel or not, so a
    /// chat with a team at work shows it; a helper popped out into its own panel is the
    /// one kind that leaves the list. Newest first, like the threads around them.
    func helpers(of parentID: UUID) -> [ChatThread] {
        children(of: parentID)
            .filter { !$0.isArchived && (!$0.isInPanel || $0.isHydraHead) }
            .sorted { $0.createdAt > $1.createdAt }
    }

    /// The heads a lead still shows in its floating panel, in the order they were sent out.
    /// The same list as `hydraHeads(of:)`, read through this thread's own cells: a write to
    /// a head of some other chat never re-renders this one.
    func panelHeads(of parentID: UUID) -> [ChatThread] {
        children(of: parentID)
            .filter { $0.isInPanel && !$0.isArchived && $0.isHydraHead }
            .sorted { ($0.hydra?.index ?? 0) < ($1.hydra?.index ?? 0) }
    }

    /// The helper a thread spawned and still shows in its floating panel, if any, read
    /// through this thread's own cells. Heads have a panel of their own (`panelHeads(of:)`).
    func panelSubagent(of parentID: UUID) -> ChatThread? {
        children(of: parentID).first { $0.isInPanel && !$0.isArchived && !$0.isHydraHead }
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
        guard let id, let thread = thread(id), let project = project(thread.projectID) else { return }
        let order = threads(in: project)
        guard let index = order.firstIndex(where: { $0.id == id }) else { return }
        var neighbors: [UUID] = []
        if index > 0 { neighbors.append(order[index - 1].id) }
        if index + 1 < order.count { neighbors.append(order[index + 1].id) }
        warmDocuments(neighbors)
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
              runningDroppyHeads(of: id) == 0 else { return false }
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
        projects.append(project)
        scheduleSave()
        return project
    }

    func chooseProjectFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.prompt = "Add project"
        panel.message = "Choose a folder for your agents to work in."
        guard panel.runModal() == .OK else { return }
        var added: Project?
        for url in panel.urls { added = addProject(at: url) }
        if let added { newThread(in: added) }
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
        let current = selectedThread
        let installed = ProviderKind.allCases.filter { providers.status($0).isInstalled && settings.isEnabled($0) }
        var provider = current?.provider ?? settings.defaultProvider
        if !installed.isEmpty, !installed.contains(provider) { provider = installed[0] }
        let carriesModel = current?.provider == provider
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
        // With no chat to carry from (a fresh launch), the pair last entered from the
        // picker is where the new chat starts, on its lead model and effort, provided
        // Hydra is on and the pair is still there for this provider. Before any pair
        // has been entered since this was remembered, the pair of the chat last worked
        // in stands in, so an existing pair chat carries over the first relaunch too.
        let lastPairID = settings.lastHydraPairID
            ?? threads.filter { $0.hydraPairID != nil && !$0.isHelper }.max { $0.updatedAt < $1.updatedAt }?.hydraPairID
        if carriesModel {
            thread.hydraPairID = current?.hydraPairID
        } else if settings.hydraEnabled, let pairID = lastPairID,
                  let pair = hydraPickerPairs.first(where: { $0.id == pairID }), pair.provider == provider {
            let lead = hydraLead(of: pair, for: thread)
            thread.model = lead.model
            thread.effort = lead.effort
            thread.fastMode = lead.fastMode
            thread.hydraPairID = pair.id
            thread.hydraEnabled = true
        }
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

    func updateThread(_ id: UUID, _ change: (inout ChatThread) -> Void) {
        guard let index = threadIndex.positions[id] else { return }
        var thread = threads[index]
        change(&thread)
        guard thread != threads[index] else { return }
        threads[index] = thread
        scheduleSave()
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
        releaseHydraCopy(of: id)
        updateThread(id) {
            $0.isArchived = true
            $0.isPinned = false
            // Restored, it is a thread again, not a settled one.
            $0.isSettled = false
            $0.settledAt = nil
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
        // A head deleted mid-job reports to its lead as stopped, so the batch it was in
        // settles and the lead's waiting heads and merge go on rather than waiting for it.
        if thread.isHydraHead, thread.hydra?.isFinished == false {
            finishHydraHead(id, status: .interrupted, summary: nil)
        }
        discardThreadState(thread)
        threads.removeAll { $0.id == id }
        if let project = project(thread.projectID) {
            let git = Git(project.path)
            // A head's copy of the checkout was Droppy Code's to make, so it always goes.
            let worktree = removeWorktree || thread.hydra?.hasOwnCopy == true ? thread.worktreePath : nil
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
                cleanup.append((project.path, thread.id, thread.hydra?.hasOwnCopy == true ? thread.worktreePath : nil))
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

    /// The helper a thread spawned and still shows in its floating panel, if any. Hydra
    /// heads have a panel of their own (see `hydraHeads(of:)`).
    func subagent(of parentID: UUID) -> ChatThread? {
        panelSubagent(of: parentID)
    }

    /// Adds a thread the app made for another thread: a head, say. The caller selects it
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
        guard let helper = thread(id) else { return }
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

    var topScript: ProjectScript? {
        guard let selectedThreadID, let thread = thread(selectedThreadID) else { return nil }
        return project(thread.projectID)?.scripts.first
    }

    func runScript(_ script: ProjectScript, threadID: UUID) {
        guard let thread = thread(threadID), let project = project(thread.projectID) else { return }
        terminals.run(script, threadID: threadID, directory: thread.worktreePath ?? project.path)
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
        let branch = "droppy-code/\(suffix)"
        let base = await git.status()?.branch
        do {
            try await git.addWorktree(at: path, branch: branch, base: base)
            updateThread(threadID) {
                $0.worktreePath = path
                $0.branch = branch
            }
            for script in project.scripts where script.runOnWorktreeCreate {
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
        guard !WebsiteCaptures.isEnabled, let thread = thread(id) else { return }
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
        // The thread itself selected, or its panel helper or Hydra head on screen with
        // the selected parent, while the app is frontmost: in view.
        let onScreen = thread(id).map(isOnScreen) ?? false
        let isVisible = NSApp.isActive && onScreen
        let isHead = thread(id)?.isHydraHead == true
        updateThread(id) {
            $0.lastStatus = status
            $0.updatedAt = .now
            // Out of view the thread goes unread, for the dock; in view it is read,
            // which also clears an unread mark from an earlier finish while away so
            // the badge cannot stick around after the chat is back on screen. Heads
            // report to their lead and never mark themselves.
            if !isHead { $0.hasUnread = !isVisible }
        }
        updateDockBadge()
        // A helper panel closed mid-turn kept its session alive to finish stopping cleanly;
        // it has now.
        if sessionsToRelease.remove(id) != nil { existingRuntime(for: id)?.stopSession() }
        // A turn stopped by a spent usage limit waits for the reset and goes on by itself:
        // no chime and no "stopped with an error", the wait says what is happening.
        if autoContinue.turnFinished(id, status: status, continues: continues) { return }
        // A head reports to its lead, which is the chat that chimes and notifies when
        // the whole job is done. A head with another turn coming (its report, after its
        // budget ran out) reports at the end of that one.
        if isHead, let head = thread(id) {
            if !continues { hydraHeadTurnFinished(head, status: status) }
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
        guard !didRequestNotifications, !WebsiteCaptures.isEnabled else { return }
        didRequestNotifications = true
        Task {
            _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
        }
    }

    func notify(threadID: UUID, title: String, body: String, sound: UNNotificationSound? = .default) {
        guard !WebsiteCaptures.isEnabled else { return }
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

    func textEngine(preferring provider: ProviderKind?) -> TextGeneration.Engine? {
        func claude() -> TextGeneration.Engine? {
            guard providers.status(.claude).auth != .signedOut, let executable = providers.executable(for: .claude) else { return nil }
            return .claude(executable: executable, environment: providers.environment(for: .claude))
        }
        func codex() -> TextGeneration.Engine? {
            guard providers.status(.codex).auth != .signedOut,
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

    func scheduleSave() {
        guard persistenceEnabled else { return }
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            self?.saveLibrary()
        }
    }

    private func saveLibrary() {
        guard persistenceEnabled else { return }
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
        var library = Library()
        library.projects = projects
        library.threads = threads
        DiskWriter.shared.writeSynchronously(library, to: Storage.libraryURL)
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

/// Reads project scripts from `droppy-code.json` at the project root.
enum ProjectFile {
    static func scripts(at url: URL) -> [ProjectScript] {
        guard let data = try? Data(contentsOf: url.appendingPathComponent("droppy-code.json")),
              let json = JSONValue.parse(data) else { return [] }
        return (json["scripts"]?.array ?? []).compactMap { script in
            guard let name = script["name"]?.string, let command = script["command"]?.string else { return nil }
            return ProjectScript(
                name: name,
                command: command,
                symbol: symbol(for: script["icon"]?.string),
                runOnWorktreeCreate: script["runOnWorktreeCreate"]?.bool ?? false
            )
        }
    }

    static func symbol(for icon: String?) -> String {
        switch icon {
        case "test": "checkmark.seal"
        case "lint": "wand.and.stars"
        case "configure": "gearshape"
        case "build": "hammer"
        case "debug": "ant"
        default: "play"
        }
    }
}
