// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The mark a project wears in the activity list's icon style: an emoji, or the name of an
/// SF Symbol. Stored as one string, `emoji:\U0001F528` or `symbol:hammer`, so the library needs no
/// shape of its own and a library written before the field reads straight through.
enum ProjectIcon: Hashable, Sendable, Codable {
    case emoji(String)
    case symbol(String)

    /// The emoji, when the mark is one.
    var emoji: String? {
        if case .emoji(let value) = self { return value }
        return nil
    }

    /// The symbol's name, when the mark is one.
    var symbolName: String? {
        if case .symbol(let name) = self { return name }
        return nil
    }

    /// The stored form: the kind, a colon, the value.
    var storage: String {
        switch self {
        case .emoji(let value): "emoji:\(value)"
        case .symbol(let name): "symbol:\(name)"
        }
    }

    /// Reads the stored form. A value with no kind in front of it is a symbol's name, so a
    /// library written by hand still reads.
    init?(storage: String) {
        guard !storage.isEmpty else { return nil }
        if storage.hasPrefix("emoji:") {
            self = .emoji(String(storage.dropFirst("emoji:".count)))
        } else if storage.hasPrefix("symbol:") {
            self = .symbol(String(storage.dropFirst("symbol:".count)))
        } else {
            self = .symbol(storage)
        }
    }

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        guard let icon = ProjectIcon(storage: raw) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "An empty project icon"))
        }
        self = icon
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(storage)
    }
}

struct Project: Codable, Identifiable, Hashable, Sendable {
    var id: UUID
    var name: String
    var path: String
    var addedAt: Date
    var isExpanded: Bool
    var scripts: [ProjectScript]
    /// The pair that each new chat in this project starts on. Nil means no rule: the chat
    /// starts on the pair the app remembers (see `AppModel.newThread`). The pair must be
    /// in Settings and its provider must be on, or the rule does not apply.
    var crewPairID: UUID?
    /// The project's mark in the activity list's icon style: its emoji or SF Symbol. Nil
    /// draws the folder mark, so a project reads at a glance before one has been picked.
    var icon: ProjectIcon?
    /// How the project stands with the user's Studiolo vault. Nil until something decides:
    /// an archive whose dashboard names this folder then links itself (see
    /// `StudioloIndex.pairing`), while a link or an unlinking the user chose stays put.
    var studioloLink: StudioloLink?

    init(name: String, path: String) {
        id = UUID()
        self.name = name
        self.path = path
        addedAt = .now
        isExpanded = true
        scripts = []
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        path = try container.decode(String.self, forKey: .path)
        name = container.value(.name, default: URL(fileURLWithPath: path).lastPathComponent)
        addedAt = container.value(.addedAt, default: .now)
        isExpanded = container.value(.isExpanded, default: true)
        scripts = container.values(.scripts)
        crewPairID = container.value(.crewPairID, default: nil)
        icon = container.value(.icon, default: nil)
        studioloLink = container.value(.studioloLink, default: nil)
    }

    var url: URL { URL(fileURLWithPath: path) }

    /// The linked archive's `项目/<name>`, whether or not the vault still has it.
    var studioloPrefix: String? {
        if case let .archive(prefix) = studioloLink { prefix } else { nil }
    }
}

/// A project's standing with the Studiolo vault, once something has decided it.
enum StudioloLink: Codable, Hashable, Sendable {
    /// Filed under the archive `项目/<name>`.
    case archive(String)
    /// The user took the link away: no archive, and none links itself again.
    case unlinked
}

struct ProjectScript: Codable, Identifiable, Hashable, Sendable {
    /// The mark a script wears when it names none, or names an SF Symbol this Mac lacks.
    static let defaultSymbol = "play"

    var id: UUID
    var name: String
    var command: String
    var symbol: String
    var runOnWorktreeCreate: Bool
    /// Read from a scripts file (a project's `bottega.json`, or the global one) rather
    /// than added in the app. The file stays the script's source: a change there replaces
    /// it, and a script gone from the file goes from the list. A script saved before the
    /// flag existed reads as added in the app; the next read of its project's file claims it
    /// back only if it is still exactly as the file has it (see `ProjectFile.merge`), and a
    /// script the user changed since stays theirs.
    var isFromFile: Bool

    /// Whether this is, field for field, the script a file's entry reads as: an import from
    /// the file nobody has changed.
    func matchesFileEntry(_ entry: ProjectScript) -> Bool {
        name == entry.name && command == entry.command && symbol == entry.symbol && runOnWorktreeCreate == entry.runOnWorktreeCreate
    }

    init(name: String, command: String, symbol: String = ProjectScript.defaultSymbol, runOnWorktreeCreate: Bool = false, isFromFile: Bool = false) {
        id = UUID()
        self.name = name
        self.command = command
        self.symbol = symbol
        self.runOnWorktreeCreate = runOnWorktreeCreate
        self.isFromFile = isFromFile
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = container.value(.id, default: UUID())
        name = try container.decode(String.self, forKey: .name)
        command = try container.decode(String.self, forKey: .command)
        symbol = container.value(.symbol, default: Self.defaultSymbol)
        runOnWorktreeCreate = container.value(.runOnWorktreeCreate, default: false)
        isFromFile = container.value(.isFromFile, default: false)
    }
}

struct ChatThread: Codable, Identifiable, Hashable, Sendable {
    static let untitled = "New thread"

    var id: UUID
    var projectID: UUID
    var title: String
    var createdAt: Date
    var updatedAt: Date
    var provider: ProviderKind
    var model: String?
    var effort: String?
    var fastMode: Bool
    var runtimeMode: RuntimeMode
    var interactionMode: InteractionMode
    var worktreePath: String?
    var branch: String?
    var providerSessionID: String?
    var providerResumeAt: String?
    var isPinned: Bool
    var isArchived: Bool
    /// Finished, as far as the sidebar is concerned: the thread sits at the bottom of its
    /// list as a small grey row until it is reopened, by hand or by a new turn.
    var isSettled = false
    /// When it settled; the latest settled thread sits first among them.
    var settledAt: Date?
    var hasUnread: Bool
    var hasCustomTitle: Bool
    /// The thread's place once its project has been reordered by dragging; nil keeps newest first.
    var sortOrder: Double?
    /// The thread's place within its group in the activity layout once dragged there.
    var activityOrder: Double?
    /// The day that place was given; a thread active on a later day is newest first again.
    var activityOrderDay: Date?
    var lastStatus: TurnStatus?
    /// The thread this one was spawned from, for a helper (a merge, for now). A helper opens
    /// in its parent's floating panel and, once that closes, sits under its parent in the
    /// sidebar as a smaller row for as long as the parent is there.
    var parentThreadID: UUID?
    /// Whether the helper is showing in its parent's floating panel. While it is, it is kept
    /// out of the sidebar, the palette, the recent picks and the unread count.
    var isInPanel = false
    /// Whether this thread's helpers are shown one by one under it in the sidebar. Off
    /// until the user opens them: folded, one line under the chat names the team, and a
    /// click on it unfolds them. Saved under its own key, so chats from before the fold
    /// was the default fold too.
    var expandsHelpers = false
    var foldsHelpers: Bool {
        get { !expandsHelpers }
        set { expandsHelpers = !newValue }
    }
    /// Per-chat Crew switch: off kills hands for this chat only, on restores them
    /// (with the app-wide switch on). New threads start on the default Settings › Crew
    /// sets (see `crewDefaultEnabled()`); flipping a chat's switch changes that chat alone.
    var crewEnabled = true
    /// Whether `crewEnabled` above was saved by a build that has the per-chat switch.
    /// Saves from before it carried an ignored false under the same key, so a thread
    /// without this flag decodes as on whatever its stored value says; every save from
    /// here on writes the flag, and its stored value is then the user's own choice.
    var crewEnabledIsExplicit = true
    /// The pair an old thread picked while Crew had per-chat switches. Kept for
    /// decoding; new threads use the best fit for their provider and model.
    var crewPairID: UUID?
    /// How many hands this chat has sent out so far: the next one's place in the roster.
    var crewSpawnCount = 0
    /// Set on a hand: the thread is one of its parent's team.
    var crew: CrewHandInfo?

    /// Spawned from another thread, whichever side of the panel it is on.
    var isHelper: Bool { parentThreadID != nil }

    /// A Crew hand, whichever side of the panel it is on.
    var isCrewHand: Bool { crew != nil }

    /// Defaults key holding the per-chat Crew default; `AppSettings` reads it from here.
    static let crewDefaultKey = "crewDefaultEnabled"

    /// The per-chat Crew default new threads start with, as "New chats lead a team" in
    /// Settings › Crew has it: on until the user turns it off there.
    static func crewDefaultEnabled() -> Bool {
        let defaults = UserDefaults.standard
        return defaults.object(forKey: crewDefaultKey) as? Bool ?? true
    }

    init(projectID: UUID, provider: ProviderKind, model: String?, effort: String?, runtimeMode: RuntimeMode, fastMode: Bool = false) {
        id = UUID()
        self.projectID = projectID
        title = Self.untitled
        createdAt = .now
        updatedAt = .now
        self.provider = provider
        self.model = model
        self.effort = effort
        self.fastMode = fastMode
        self.runtimeMode = runtimeMode
        interactionMode = .build
        isPinned = false
        isArchived = false
        hasUnread = false
        hasCustomTitle = false
        crewEnabled = Self.crewDefaultEnabled()
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        projectID = try container.decode(UUID.self, forKey: .projectID)
        title = container.value(.title, default: Self.untitled)
        createdAt = container.value(.createdAt, default: .now)
        updatedAt = container.value(.updatedAt, default: createdAt)
        provider = container.value(.provider, default: .codex)
        model = container.value(.model, default: nil)
        effort = container.value(.effort, default: nil)
        fastMode = container.value(.fastMode, default: false)
        // `value` swallows a type mismatch as well as a missing key, so a mode that
        // cannot be read falls back to the one that asks before it acts: a thread must
        // never quietly come back from disk with full access to the machine.
        runtimeMode = container.value(.runtimeMode, default: .supervised)
        interactionMode = container.value(.interactionMode, default: .build)
        worktreePath = container.value(.worktreePath, default: nil)
        branch = container.value(.branch, default: nil)
        providerSessionID = container.value(.providerSessionID, default: nil)
        providerResumeAt = container.value(.providerResumeAt, default: nil)
        isPinned = container.value(.isPinned, default: false)
        isArchived = container.value(.isArchived, default: false)
        isSettled = container.value(.isSettled, default: false)
        settledAt = container.value(.settledAt, default: nil)
        hasUnread = container.value(.hasUnread, default: false)
        hasCustomTitle = container.value(.hasCustomTitle, default: false)
        sortOrder = container.value(.sortOrder, default: nil)
        activityOrder = container.value(.activityOrder, default: nil)
        activityOrderDay = container.value(.activityOrderDay, default: nil)
        lastStatus = container.value(.lastStatus, default: nil)
        parentThreadID = container.value(.parentThreadID, default: nil)
        isInPanel = container.value(.isInPanel, default: false)
        expandsHelpers = container.value(.expandsHelpers, default: false)
        // Threads saved while Crew was app-wide only carry an ignored false and no
        // explicit flag: they come back on. A save that wrote the flag holds a real choice.
        let explicit = container.value(.crewEnabledIsExplicit, default: false)
        crewEnabled = explicit ? container.value(.crewEnabled, default: true) : true
        crewEnabledIsExplicit = true
        crewPairID = container.value(.crewPairID, default: nil)
        crewSpawnCount = container.value(.crewSpawnCount, default: 0)
        crew = container.value(.crew, default: nil)
    }
}

struct Library: Codable, Sendable {
    var projects: [Project] = []
    var threads: [ChatThread] = []

    init() {}

    /// A project or thread this build cannot read goes alone; a list that is not a list makes
    /// the whole file unreadable, so it is kept aside rather than saved over (see `StoredFile`).
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        projects = try container.documentValues(.projects)
        threads = try container.documentValues(.threads)
    }
}
