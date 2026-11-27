// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import BottegaKit
import BottegaLink
import Observation
import SwiftUI

/// What the window shows, and the one place anything is sent from.
@MainActor @Observable
final class WorkshopModel {
    /// Where events come from.
    enum Source: Equatable {
        /// No run directory chosen yet.
        case none
        /// Recorded events: nothing to connect to, nothing can be sent.
        case recording(URL)
        /// LMAS's run directory, `ui.sock` inside it.
        case lmas(URL)
    }

    struct OpenAsk: Identifiable { let id: String }

    private(set) var source: Source
    private(set) var state = WorkshopState()
    private(set) var status: LinkClient.Status?
    private(set) var overview: Overview?
    private(set) var welcome: Welcome?
    private(set) var outbox: [Outbox.Item] = []
    /// Something went wrong that the user should know and can wave away.
    var notice: Why?
    private(set) var registrationFailure: Why?
    /// What this Mac was registered as, or may have been.
    private(set) var registeredName: String?

    var page: Page = .dispatcher {
        didSet {
            if page != oldValue {
                detail = nil
                arranging = nil
                markRead(leaving: oldValue, opening: page)
            }
            if let section = page.section { self.section = section }
            if page.section == .bots { botsPage = page }
        }
    }
    /// Which list the sidebar shows; follows the page, except for pages that belong to no section.
    private(set) var section: SidebarSection = .bots
    /// The part of the log the Activity page shows.
    var activityFilter: ActivityFilter = .everything
    /// Where Bots goes back to: the line last open there.
    private var botsPage: Page = .dispatcher

    func show(_ section: SidebarSection) {
        page = switch section {
        case .rooms: .rooms
        case .bots: botsPage
        case .waiting: .waiting
        case .cards: .board
        case .activity: .activity
        }
    }
    var detail: Detail? { didSet { if detail != nil { showsInspector = true } } }
    /// The ask whose sheet is up.
    var askBeingRead: OpenAsk?
    var showsInspector = false
    /// ⌘K (`WorkshopPalette`).
    var showsPalette = false
    var showsLanes = true

    /// The cards page as a board or as a list (Bottega-5 §7). Kept on this Mac, so it opens the way it was left.
    enum CardLayout: String { case board, list }
    var cardLayout = WorkshopModel.savedCardLayout {
        didSet { UserDefaults.standard.set(cardLayout.rawValue, forKey: Self.cardLayoutKey) }
    }
    private static let cardLayoutKey = "cardLayout"
    private static var savedCardLayout: CardLayout {
        UserDefaults.standard.string(forKey: cardLayoutKey).flatMap(CardLayout.init(rawValue:)) ?? .board
    }

    /// The inside of turns, as far as pushed or fetched.
    private(set) var turns: [String: TurnModel] = [:]

    private(set) var drafts = Drafts()
    /// Which composer an outbox item's words came from.
    private var origins: [String: DraftKey] = [:]
    /// What the box on each bot's home holds, by vendor: a line not opened yet.
    var lineDrafts: [String: LineDraft] = [:]
    /// Which bot's home an `open_line` was sent from, by content id.
    var openings: [String: String] = [:]
    private(set) var putAway: Set<TripID> = []

    private(set) var client: LinkClient?
    private var pumps: [Task<Void, Never>] = []
    /// The events stopped fitting what is shown (`apply`), and the link was let go of. Only starting over helps.
    private var isOutOfStep = false

    /// Letters to the user that have been on screen while the app was in front.
    private var shown: Set<String> = []
    private var onScreen: Set<String> = []
    private var reportedSeen: UInt64 = 0
    private var seenReport: Task<Void, Never>?

    init(source: Source) {
        self.source = source
        loadPutAway()
    }

    /// The run directory chosen in Settings; a Debug build can be pointed elsewhere for one launch.
    static func forThisLaunch() -> WorkshopModel {
        #if DEBUG
        let environment = ProcessInfo.processInfo.environment
        let source: Source =
            if let path = environment["BOTTEGA_RECORDING"] {
                .recording(URL(fileURLWithPath: path))
            } else if let path = environment["BOTTEGA_RUN_DIRECTORY"] {
                .lmas(URL(fileURLWithPath: path))
            } else {
                chosenSource
            }
        let model = WorkshopModel(source: source)
        if let page = environment["BOTTEGA_PAGE"].flatMap(Page.init(word:)) { model.page = page }
        if let detail = environment["BOTTEGA_DETAIL"].flatMap(Detail.init(word:)) { model.detail = detail }
        if environment["BOTTEGA_LANES"] == "0" { model.showsLanes = false }
        // `BOTTEGA_PALETTE=<what is typed>`: ⌘K is up, for a screenshot.
        if environment["BOTTEGA_PALETTE"] != nil { model.showsPalette = true }
        // `BOTTEGA_ARRANGE=header` or `sidebar`: the line's arrange panel is up, for a screenshot.
        if let place = environment["BOTTEGA_ARRANGE"], let line = model.page.line {
            model.arranging = Arranging(line: line, inSidebar: place == "sidebar")
        }
        return model
        #else
        return WorkshopModel(source: chosenSource)
        #endif
    }

    private static let runDirectoryKey = "lmasRunDirectory"

    private static var chosenSource: Source {
        guard let path = UserDefaults.standard.string(forKey: runDirectoryKey), !path.isEmpty else { return .none }
        return .lmas(URL(fileURLWithPath: path))
    }

    // MARK: Running

    private var isRunning = false

    /// Once per launch; the window's task.
    func run() async {
        guard !isRunning else { return }
        isRunning = true
        begin()
    }

    private func begin() {
        for pump in pumps { pump.cancel() }
        pumps = []
        client = nil
        isOutOfStep = false
        state = WorkshopState()
        status = nil
        overview = nil
        welcome = nil
        turns = [:]
        registeredName = nil
        registrationFailure = nil
        outbox = []
        shown = []
        reportedSeen = 0
        loadPutAway()
        switch source {
        case .none: break
        case .recording(let url): replay(url)
        case .lmas(let directory): connect(directory)
        }
    }

    private func connect(_ directory: URL) {
        let client = LinkClient(
            connector: UnixSocketConnector(path: directory.appendingPathComponent("ui.sock").path),
            chest: Self.chest(directory), store: FileKeepsakeStore(url: Self.keepsakeURL(directory)))
        self.client = client
        pumps = [
            Task { await client.run() },
            Task { for await update in client.updates { take(update) } },
        ]
    }

    /// Each LMAS gives this Mac a number of its own, so each gets keys of its own too.
    private static func tag(_ directory: URL) -> String {
        String(Digest.sha256(directory.standardizedFileURL.path).prefix(16))
    }

    private static func chest(_ directory: URL) -> any KeyChest {
        SecureChest(
            isDevelopment: AppInfo.isDevelopment, folder: supportFolder.appendingPathComponent("keys-\(tag(directory))"),
            service: "\(AppInfo.keychainService).lmas.\(tag(directory))")
    }

    private static func keepsakeURL(_ directory: URL) -> URL {
        supportFolder.appendingPathComponent("link-\(tag(directory)).json")
    }

    private static var supportFolder: URL {
        URL.applicationSupportDirectory.appendingPathComponent(AppInfo.storageName).appendingPathComponent("lmas")
    }

    private func take(_ update: LinkClient.Update) {
        // Let go of (`apply`): what the old link still had on its way is not shown over the stop.
        guard !isOutOfStep else { return }
        switch update {
        case .status(let status):
            self.status = status
            if status == .live { reportSeen() }
        case .reset(let start):
            state = WorkshopState(startingAt: start)
            turns = [:]
            shown = []
            reportedSeen = 0
        case .watching(let running):
            for turn in turns.values where running?.contains(turn.id) != true { turn.details.fellBehind() }
        case .live(let lines): for line in lines { take(line) }
        case .records(let records): apply(records)
        case .overview(let overview): self.overview = overview
        case .welcome(let welcome): self.welcome = welcome
        case .registeredName(let name): registeredName = name
        case .outbox(let items): outbox = items
        case .settled(let settled): take(settled)
        case .notice(let why): notice = why
        case .registrationFailed(let why): registrationFailure = why
        }
    }

    private func apply(_ records: [Record]) {
        do {
            for record in records { try state.apply(record) }
        } catch {
            // The link checks the numbers before it passes records on; if they still do not fit, stop all the same.
            // Reconnect and Reload All Events then start over with a new link (`startOver`).
            client = nil
            isOutOfStep = true
            for pump in pumps { pump.cancel() }
            status = .stopped(Why(code: "sequence", text: "The events do not follow on: expected \(error.expected), got \(error.got)."))
        }
    }

    // MARK: Turns

    private func take(_ live: Live) {
        guard !live.turn.isEmpty else { return }
        turnModel(live.turn).details.take(live.what)
        if case .ended = live.what { letGoOfOldTurns() }
    }

    private func turnModel(_ id: String) -> TurnModel {
        if let turn = turns[id] { return turn }
        let turn = TurnModel(id: id)
        turns[id] = turn
        return turn
    }

    /// LMAS has them; what is not being looked at is fetched again when it is.
    private func letGoOfOldTurns() {
        let over = turns.values.filter { $0.details.completeness != .growing && detail != .turn($0.id) && !$0.isFetching }
        guard over.count > 12 else { return }
        for turn in over.sorted(by: { ($0.details.head?.startedAtMs ?? 0) < ($1.details.head?.startedAtMs ?? 0) }).prefix(over.count - 12) {
            turns[turn.id] = nil
        }
    }

    /// What LMAS has of a turn, if it is not all here and nothing more comes by itself.
    func fetch(turn id: String) {
        guard let client, isLive else { return }
        let turn = turnModel(id)
        guard !turn.isFetching, turn.details.completeness == .behind else { return }
        turn.isFetching = true
        Task {
            do {
                let pages = try await client.turn(id)
                turn.details.take(pages)
                turn.problem = nil
            } catch let error as WireError {
                turn.problem = Why(code: error.code, text: error.text)
            } catch is LinkClient.ConnectionLost {
                // Asked again once there is a connection.
            } catch {
                turn.problem = Why(code: "malformed", text: error.localizedDescription)
            }
            turn.isFetching = false
        }
    }

    // MARK: Recordings

    func play(_ url: URL) {
        source = .recording(url)
        page = .dispatcher
        detail = nil
        begin()
    }

    private func replay(_ url: URL) {
        do {
            for line in try Data(contentsOf: url).split(separator: UInt8(ascii: "\n")) {
                switch try ServerLine.read(Data(line)) {
                case .record(let record): apply([record])
                case .live(let live): take(live)
                default: break
                }
            }
        } catch {
            notice = Why(code: "recording", text: "Cannot read that recording: \(error.localizedDescription)")
        }
    }

    // MARK: The link

    var hasLink: Bool { client != nil || isOutOfStep }
    /// Stopped for a reason registering again would cure.
    var canRegisterAgain: Bool { LinkClient.Status.wait(of: status) == .registeringAgain }
    var needsRegistration: Bool { status == .needsRegistration || status == .registering }
    var isLive: Bool { status == .live }
    /// This Mac's device number with the LMAS it is connected to.
    var thisDevice: String? { welcome?.device }

    var runDirectory: URL? {
        if case .lmas(let directory) = source { directory } else { nil }
    }

    func choose(runDirectory: URL) {
        UserDefaults.standard.set(runDirectory.path, forKey: Self.runDirectoryKey)
        source = .lmas(runDirectory)
        begin()
    }

    func reconnect() {
        guard let client else { return startOver() }
        Task { await client.reconnect() }
    }

    /// With the link let go of because the events stopped fitting: a new link, from the first event.
    private func startOver() {
        if isOutOfStep { begin() }
    }

    func refreshOverview() {
        guard let client else { return }
        Task { await client.refreshOverview() }
    }

    func reload() {
        guard let client else { return startOver() }
        Task { await client.reload() }
    }

    func register(name: String) {
        guard let client, isFromTheUser() else { return }
        registrationFailure = nil
        Task { await client.register(name: name) }
    }

    /// Back to registering, with the same keys: LMAS does not know this Mac any more. Signs nothing and sends nothing,
    /// so it passes no gate (the confirmation's own buttons could not); registering does.
    func forgetDevice() {
        guard let client else { return }
        Task {
            do { try await client.forgetDevice() } catch { notice = Why(code: "disk", text: error.localizedDescription) }
        }
    }

    // MARK: Drafts

    subscript(draft key: DraftKey) -> UnsentDraft {
        get { drafts[key] }
        set { drafts[key] = newValue }
    }

    func target(of key: DraftKey) -> DraftTarget? { drafts.target(of: key, in: state) }

    /// The box with the cursor in it.
    var focusedDraft: DraftKey?

    /// Text files only; what does not fit stays out and says why.
    func attach(_ urls: [URL], to key: DraftKey) {
        for url in urls {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            let name = UntrustedText.line(url.lastPathComponent, limit: 60)
            do {
                let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                guard size <= IntentDraft.Limit.file else {
                    self[draft: key].problem = Wording.why(.fileTooLong(name: name))
                    continue
                }
                guard let text = String(data: try Data(contentsOf: url), encoding: .utf8) else {
                    self[draft: key].problem = Why(code: "draft", text: "\(name) is not a text file.")
                    continue
                }
                self[draft: key].files.append(TextFile(name: url.lastPathComponent, content: text))
            } catch {
                self[draft: key].problem = Why(code: "draft", text: "Cannot read \(name): \(error.localizedDescription)")
            }
        }
    }

    // MARK: Sending

    /// The one gate in front of everything that gets signed.
    func isFromTheUser() -> Bool {
        do throws(InputGate.Refusal) {
            try HumanInput.take()
            return true
        } catch {
            notice = Wording.why(error)
            return false
        }
    }

    /// What is decided about how things are now (`IntentDraft.keeps` false: approving, declining, stopping a turn,
    /// changing a model, a project) is sent only while what is shown is how things are. The link refuses it too.
    func canDecideNow() -> Bool {
        if isLive { return true }
        notice = Wording.notNow(status)
        return false
    }

    func send(_ key: DraftKey) {
        guard !self[draft: key].isBlank else { return readFirstAsk() }
        guard let client, let intent = drafts.intent(for: key, in: state), isFromTheUser() else { return }
        do throws(IntentDraft.Problem) {
            try intent.check()
        } catch {
            drafts[key].problem = Wording.why(error)
            return
        }
        let sent = drafts[key]
        drafts.cleared(key)
        Task {
            do {
                origins[try await client.submit(intent)] = key
            } catch {
                drafts.putBack(sent.text, sent.files, Wording.whyNotTaken(error, while: status), in: key)
            }
        }
    }

    private var firstAsk: String? {
        waiting.lazy.compactMap { if case .ask(let id) = $0.what { id } else { nil } }.first
    }

    var hasAskToRead: Bool { firstAsk != nil }

    /// ⌘Return in an empty composer: up comes the ask that has waited longest, to be read; ⌘Return there approves it.
    private func readFirstAsk() {
        if let firstAsk { askBeingRead = OpenAsk(id: firstAsk) }
    }

    /// `read` is the digest of the text as the user read it. That is what gets signed, and only while the ask still
    /// says it: a number can come to stand for another ask.
    func approve(_ id: String, read: String) {
        guard let client, let ask = state.asks[id], ask.answer == nil, canDecideNow() else { return }
        guard ask.textDigest == read else {
            notice = Why(code: "changed", text: "\(UntrustedText.line(id, limit: 40)) has changed. Read it again.")
            return
        }
        guard isFromTheUser() else { return }
        let reason = "approve: " + UntrustedText.line(UntrustedText.revealing(ask.text).text, limit: 160)
        submit(.approve(ask: id, textDigest: read), to: client, touchIDReason: reason)
    }

    func decline(_ id: String) {
        guard let client, let ask = state.asks[id], ask.answer == nil, canDecideNow(), isFromTheUser() else { return }
        submit(.decline(ask: id), to: client)
    }

    func submit(_ intent: IntentDraft, to client: LinkClient, touchIDReason: String = "") {
        Task {
            do {
                try await client.submit(intent, touchIDReason: touchIDReason)
            } catch {
                notice = Wording.whyNotTaken(error, while: status)
            }
        }
    }

    /// An answer is on its way, so its buttons are not offered again.
    func isDeciding(_ ask: String) -> Bool {
        outbox.contains { item in
            switch item.draft {
            case .approve(let id, _), .decline(let id): id == ask
            case .say, .answer, .arrange, .markRead, .addProject, .removeProject, .openLine, .setModel, .stopTurn: false
            }
        }
    }

    func withdraw(_ id: String) {
        guard let client else { return }
        Task { await client.withdraw(id) }
    }

    private func take(_ settled: Outbox.Settled) {
        if let vendor = openings.removeValue(forKey: settled.item.id) { return opened(settled, from: vendor) }
        let key = origins.removeValue(forKey: settled.item.id)
        switch settled.result {
        case .done(let message, _):
            // A line the user just started: go and look at it, unless they have moved on.
            if key == .dispatcher, page == .dispatcher, let message, case .say(let to, nil, _, _, _) = settled.item.draft,
                state.role(of: to) != "dispatcher"
            {
                page = .line(message)
            }
        case .already: break
        case .refused(let why), .notSent(let why):
            if let typed = settled.item.draft.typed {
                drafts.putBack(typed.text, typed.files, why, in: state.reachable(key ?? settled.item.draft.home))
            } else {
                notice = why
            }
        }
    }

    /// Words on their way from a box.
    func sending(in key: DraftKey) -> [Outbox.Item] {
        outbox.filter { state.belongs($0.draft, above: key) }
    }

    // MARK: Waiting

    /// Longest wait first, without what the user has put away.
    var waiting: [Waiting] {
        state.waiting.filter { item in
            if case .trip(let id) = item.what { !putAway.contains(id) } else { true }
        }
    }

    func isWaiting(_ trip: TripID) -> Bool { state.trips[trip] != nil && !putAway.contains(trip) }

    var waitingLines: Set<String> { state.linesWaiting(waiting) }

    func putAway(_ trip: TripID) {
        putAway.insert(trip)
        UserDefaults.standard.set(putAway.map { "\($0.seq).\($0.index)" }.sorted(), forKey: putAwayKey)
    }

    private var putAwayKey: String {
        switch source {
        case .none: "putAwayTrips"
        case .recording(let url), .lmas(let url): "putAwayTrips-" + Digest.sha256(url.path).prefix(16)
        }
    }

    private func loadPutAway() {
        putAway = Set((UserDefaults.standard.stringArray(forKey: putAwayKey) ?? []).compactMap { word in
            let parts = word.split(separator: ".")
            guard parts.count == 2, let seq = UInt64(parts[0]), let index = Int(parts[1]) else { return nil }
            return TripID(seq: seq, index: index)
        })
    }

    // MARK: Arranging

    /// The line whose arrange panel is up, and what it hangs from: the page's header or the line's row in the sidebar.
    struct Arranging: Equatable {
        let line: String
        let inSidebar: Bool
    }
    /// Menus only open the panel: a pick in a menu with the mouse is not taken as the user's (`HumanInput`), so what
    /// gets signed is done with the panel's own buttons, or with the keyboard.
    var arranging: Arranging?

    /// Return in the panel's name box. Nothing is signed when the name stays as it is. false when it was not taken:
    /// the panel stays up, with the notice saying why.
    @discardableResult func rename(_ line: String, to text: String) -> Bool {
        guard let thread = state.threads[line], let intent = state.renaming(thread, to: text) else { return true }
        return arrange(intent)
    }

    func togglePin(_ line: String) {
        arrange(.arrange(line: line, title: nil, pinned: !state.arrangement(of: line).isPinned, archived: nil))
    }

    func toggleArchive(_ line: String) {
        arrange(.arrange(line: line, title: nil, pinned: nil, archived: !state.arrangement(of: line).isArchived))
    }

    /// Kept and sent later while LMAS is out of reach, as a letter is (`IntentDraft.keeps`).
    @discardableResult private func arrange(_ intent: IntentDraft) -> Bool {
        guard let client, isFromTheUser() else { return false }
        do throws(IntentDraft.Problem) {
            try intent.check()
        } catch {
            notice = Wording.why(error)
            return false
        }
        submit(intent, to: client)
        return true
    }

    /// Something about the line is on its way to LMAS.
    func isArranging(_ line: String) -> Bool {
        outbox.contains { if case .arrange(line, _, _, _) = $0.draft { true } else { false } }
    }

    // MARK: Read

    /// Going from one line to another with a key or a click says a line has been read: the one left, if something
    /// came in on it while it was open, otherwise the one opened. One press signs one thing, so the other waits for
    /// the next move. A page that changes by itself (a line opening after its first letter went out) marks nothing.
    private func markRead(leaving old: Page, opening new: Page) {
        guard let client, isLive, let (line, through) = state.lineToMarkRead(leaving: old.line, opening: new.line) else { return }
        // Quietly: moving about is not something to be told off for.
        do throws(InputGate.Refusal) { try HumanInput.take() } catch { return }
        submit(.markRead(line: line, through: through), to: client)
    }

    // MARK: Seen

    /// A letter to the user came into view, or left it. Shown means shown: with the app in front.
    func letter(_ id: String, isOnScreen: Bool) {
        if isOnScreen { onScreen.insert(id) } else { onScreen.remove(id) }
        reportSeen()
    }

    func reportSeen() {
        guard NSApp.isActive else { return }
        shown.formUnion(onScreen)
        guard let client, isLive, seenReport == nil else { return }
        seenReport = Task {
            try? await Task.sleep(for: .seconds(1))
            seenReport = nil
            guard let through = state.seenThrough(shown: shown), through > reportedSeen, isLive else { return }
            // Not taken note of, it is said again with the next record or the next connection.
            if await client.seen(through: through) { reportedSeen = max(reportedSeen, through) }
        }
    }
}
