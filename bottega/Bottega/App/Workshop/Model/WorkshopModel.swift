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

    var page: Page = .dispatcher { didSet { if page != oldValue { detail = nil } } }
    var detail: Detail? { didSet { if detail != nil { showsInspector = true } } }
    /// The ask whose sheet is up.
    var askBeingRead: OpenAsk?
    var showsInspector = false
    var showsLanes = true

    private(set) var drafts = Drafts()
    /// Which composer an outbox item's words came from.
    private var origins: [String: DraftKey] = [:]
    private(set) var putAway: Set<TripID> = []

    private var client: LinkClient?
    private var pumps: [Task<Void, Never>] = []

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
        state = WorkshopState()
        status = nil
        overview = nil
        welcome = nil
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

    /// Each LMAS gives this Mac a number of its own, so each gets keys of its own too: registering again with one
    /// leaves the others alone.
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
        switch update {
        case .status(let status):
            self.status = status
            if status == .live { reportSeen() }
        case .reset(let start):
            state = WorkshopState(startingAt: start)
            shown = []
            reportedSeen = 0
        case .records(let records): apply(records)
        case .overview(let overview): self.overview = overview
        case .welcome(let welcome): self.welcome = welcome
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
            notice = Why(code: "sequence", text: "The events do not follow on: expected \(error.expected), got \(error.got).")
            client = nil
            for pump in pumps { pump.cancel() }
            status = .stopped(notice ?? .notSigned)
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
                if case .record(let record) = try ServerLine.read(Data(line)) { apply([record]) }
            }
        } catch {
            notice = Why(code: "recording", text: "Cannot read that recording: \(error.localizedDescription)")
        }
    }

    // MARK: The link

    var hasLink: Bool { client != nil }
    /// Stopped for a reason registering again would cure.
    var canRegisterAgain: Bool {
        if case .stopped(let why) = status { ["keys", "handshake"].contains(why.code) } else { false }
    }
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
        guard let client else { return }
        Task { await client.reconnect() }
    }

    func reload() {
        guard let client else { return }
        Task { await client.reload() }
    }

    func register(name: String) {
        guard let client, isFromTheUser() else { return }
        registrationFailure = nil
        Task { await client.register(name: name) }
    }

    /// Back to registering, with new keys: LMAS does not know this Mac any more, or its keys no longer sign. Signs
    /// nothing and sends nothing, so it passes no gate (the confirmation's own buttons could not); registering does.
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
    private func isFromTheUser() -> Bool {
        if HumanInput.take() { return true }
        notice = Why(code: "synthetic", text: "That did not come from the keyboard or the mouse, so nothing was sent.")
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
                drafts.putBack(sent.text, sent.files, Why(code: "send", text: error.localizedDescription), in: key)
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

    /// `shown` is the ask as it was when it came on screen: that is what gets signed, not what it has become since.
    func approve(_ shown: Ask) {
        guard let client, isLive, let ask = state.asks[shown.id], ask.answer == nil else { return }
        guard ask.text == shown.text else {
            notice = Why(code: "changed", text: "\(shown.id) has changed. Read it again.")
            return
        }
        guard isFromTheUser() else { return }
        let reason = "approve: " + UntrustedText.line(UntrustedText.revealing(shown.text).text, limit: 160)
        submit(.approve(ask: shown.id, textDigest: Digest.sha256(shown.text)), to: client, touchIDReason: reason)
    }

    func decline(_ id: String) {
        guard let client, let ask = state.asks[id], ask.answer == nil, isFromTheUser() else { return }
        submit(.decline(ask: id), to: client)
    }

    private func submit(_ intent: IntentDraft, to client: LinkClient, touchIDReason: String = "") {
        Task {
            do {
                try await client.submit(intent, touchIDReason: touchIDReason)
            } catch {
                notice = Why(code: "send", text: error.localizedDescription)
            }
        }
    }

    /// An answer is on its way, so its buttons are not offered again.
    func isDeciding(_ ask: String) -> Bool {
        outbox.contains { item in
            switch item.draft {
            case .approve(let id, _), .decline(let id): id == ask
            case .say, .answer: false
            }
        }
    }

    func withdraw(_ id: String) {
        guard let client else { return }
        Task { await client.withdraw(id) }
    }

    private func take(_ settled: Outbox.Settled) {
        let key = origins.removeValue(forKey: settled.item.id)
        switch settled.result {
        case .done(let message):
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
            let through = state.seenThrough(shown: shown)
            guard through > reportedSeen, isLive else { return }
            reportedSeen = through
            await client.seen(through: through)
        }
    }
}
