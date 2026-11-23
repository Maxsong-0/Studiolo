// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

struct TimelineItem: Codable, Identifiable, Hashable, Sendable {
    var id: String
    var turnID: UUID?
    var date: Date
    var content: Content

    init(id: String = UUID().uuidString, turnID: UUID?, date: Date = .now, content: Content) {
        self.id = id
        self.turnID = turnID
        self.date = date
        self.content = content
    }

    enum Content: Codable, Hashable, Sendable {
        case user(UserMessage)
        case assistant(AssistantMessage)
        case reasoning(ReasoningBlock)
        case tool(ToolCall)
        case plan(ProposedPlan)
        case todos([TodoStep])
        case notice(Notice)
        case turnEnd(TurnSummary)
    }

    /// A thinking block with nothing to read, as a redacted stream leaves behind.
    var isEmptyReasoning: Bool {
        guard case .reasoning(let block) = content else { return false }
        return block.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

/// The content is coded by hand in the shape the synthesized conformance gave it,
/// `{"<case>": {"_0": <value>}}`, so every stored thread reads as before. It is not left to
/// synthesis because a checklist's steps are read one by one: a step this build cannot read
/// goes alone rather than taking the whole checklist, and its row, out of the timeline.
/// Both directions switch over every case, so a case added without its key does not compile.
extension TimelineItem.Content {
    private enum CaseKeys: String, CodingKey {
        case user, assistant, reasoning, tool, plan, todos, notice, turnEnd
    }

    /// The key of the one unlabeled value each case carries.
    private enum ValueKeys: String, CodingKey {
        case value = "_0"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CaseKeys.self)
        guard container.allKeys.count == 1, let key = container.allKeys.first else {
            throw DecodingError.typeMismatch(Self.self, DecodingError.Context(codingPath: container.codingPath, debugDescription: "Invalid number of keys found, expected one."))
        }
        let payload = try container.nestedContainer(keyedBy: ValueKeys.self, forKey: key)
        switch key {
        case .user: self = .user(try payload.decode(UserMessage.self, forKey: .value))
        case .assistant: self = .assistant(try payload.decode(AssistantMessage.self, forKey: .value))
        case .reasoning: self = .reasoning(try payload.decode(ReasoningBlock.self, forKey: .value))
        case .tool: self = .tool(try payload.decode(ToolCall.self, forKey: .value))
        case .plan: self = .plan(try payload.decode(ProposedPlan.self, forKey: .value))
        case .todos: self = .todos(payload.values(.value))
        case .notice: self = .notice(try payload.decode(Notice.self, forKey: .value))
        case .turnEnd: self = .turnEnd(try payload.decode(TurnSummary.self, forKey: .value))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CaseKeys.self)
        func put<Value: Encodable>(_ value: Value, as key: CaseKeys) throws {
            var payload = container.nestedContainer(keyedBy: ValueKeys.self, forKey: key)
            try payload.encode(value, forKey: .value)
        }
        switch self {
        case .user(let message): try put(message, as: .user)
        case .assistant(let message): try put(message, as: .assistant)
        case .reasoning(let block): try put(block, as: .reasoning)
        case .tool(let call): try put(call, as: .tool)
        case .plan(let plan): try put(plan, as: .plan)
        case .todos(let steps): try put(steps, as: .todos)
        case .notice(let notice): try put(notice, as: .notice)
        case .turnEnd(let summary): try put(summary, as: .turnEnd)
        }
    }
}

extension TimelineItem {
    /// Model-written prose with its long dashes flattened, for threads stored by a
    /// build that cleaned text only as it arrived. The user's own words, tool output
    /// and notices are not the model's writing and are left exactly as they were.
    var cleanedOfEmDashes: TimelineItem {
        var item = self
        switch item.content {
        case .assistant(var message):
            message.text = TextCleanup.withoutEmDashes(message.text)
            item.content = .assistant(message)
        case .reasoning(var block):
            block.text = TextCleanup.withoutEmDashes(block.text)
            item.content = .reasoning(block)
        case .plan(var plan):
            plan.markdown = TextCleanup.withoutEmDashes(plan.markdown)
            item.content = .plan(plan)
        default:
            break
        }
        return item
    }
}

struct UserMessage: Codable, Hashable, Sendable {
    var text: String
    var attachments: [Attachment] = []
    /// Set when the message is Crew's rather than the user's own words: the roster
    /// places of the hands reporting back, or empty for a note from Crew itself.
    var crewHands: [Int]?
    /// Set when the message is the lead's brief to a hand, shown as a pill rather
    /// than a plain bubble. Optional, so threads stored before it still decode.
    var crewBrief: Bool?

    var isFromCrew: Bool { crewHands != nil }
    var isCrewBrief: Bool { crewBrief == true }
}

extension UserMessage {
    /// Read as the synthesized conformance reads it, except that the attachments are read
    /// one by one: a file this build cannot read goes alone, and the message keeps its
    /// words and its other files rather than leaving the timeline with it.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        text = try container.decode(String.self, forKey: .text)
        attachments = container.values(.attachments)
        crewHands = try container.decodeIfPresent([Int].self, forKey: .crewHands)
        crewBrief = try container.decodeIfPresent(Bool.self, forKey: .crewBrief)
    }
}

/// One queued follow-up prompt. Sent as a direct user chat message once the
/// running turn finishes, so the user can steer the chat while it works and
/// stack several follow-ups that run one after another.
struct FollowUpPrompt: Codable, Hashable, Identifiable, Sendable {
    var id = UUID()
    var text: String
    var attachments: [Attachment] = []
    var createdAt = Date.now
    /// Prompts sharing a bundle sit together, show one number and go out as one
    /// message (see `ThreadRuntime.bundleFollowUp`).
    var bundleID: UUID? = nil

    var isEmpty: Bool {
        attachments.isEmpty && !text.contains { !$0.isWhitespace }
    }
}

extension FollowUpPrompt {
    /// Read as the synthesized conformance reads it, except that the attachments are read
    /// one by one: a file this build cannot read goes alone, not the queued prompt with it.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        text = try container.decode(String.self, forKey: .text)
        attachments = container.values(.attachments)
        createdAt = try container.decode(Date.self, forKey: .createdAt)
        bundleID = try container.decodeIfPresent(UUID.self, forKey: .bundleID)
    }
}

struct Attachment: Codable, Hashable, Identifiable, Sendable {
    var id = UUID()
    var name: String
    var path: String
    var mimeType: String

    var url: URL { URL(fileURLWithPath: path) }
    var isImage: Bool { mimeType.hasPrefix("image/") }

    /// The images among the attachments with their bytes as base64, read and encoded off
    /// the main actor: a photo is megabytes, and its string a third larger again.
    @concurrent
    static func base64Images(_ attachments: [Attachment]) async -> [(image: Attachment, base64: String)] {
        attachments.compactMap { attachment in
            guard attachment.isImage, let data = try? Data(contentsOf: attachment.url) else { return nil }
            return (attachment, data.base64EncodedString())
        }
    }
}

struct AssistantMessage: Codable, Hashable, Sendable {
    var text: String
    var isStreaming = false
}

struct ReasoningBlock: Codable, Hashable, Sendable {
    var text: String
    var isStreaming = false
}

struct ToolCall: Codable, Hashable, Sendable {
    enum Kind: String, Codable, Sendable {
        case command
        case read
        case edit
        case search
        case web
        case mcp
        case agent
        case other
    }

    enum Status: String, Codable, Sendable {
        case running
        case completed
        case failed
        case declined
    }

    static let outputLimit = 60_000

    var kind: Kind
    var title: String
    var detail: String?
    var output = ""
    var status: Status = .running
    var exitCode: Int?
    var edits: [FileEdit] = []
    var startedAt = Date.now
    var finishedAt: Date?

    mutating func appendOutput(_ text: String) {
        output += text
        trimOutput()
    }

    mutating func setOutput(_ text: String) {
        output = text
        trimOutput()
    }

    mutating func finish(_ status: Status) {
        self.status = status
        if finishedAt == nil { finishedAt = .now }
    }

    private mutating func trimOutput() {
        guard output.utf8.count > Self.outputLimit else { return }
        output = "…" + String(output.suffix(Self.outputLimit / 2))
    }
}

extension ToolCall {
    /// Read as the synthesized conformance reads it, except that the edits are read one by
    /// one: an edit this build cannot read goes alone, not the tool's row with it.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decode(Kind.self, forKey: .kind)
        title = try container.decode(String.self, forKey: .title)
        detail = try container.decodeIfPresent(String.self, forKey: .detail)
        output = try container.decode(String.self, forKey: .output)
        status = try container.decode(Status.self, forKey: .status)
        exitCode = try container.decodeIfPresent(Int.self, forKey: .exitCode)
        edits = container.values(.edits)
        startedAt = try container.decode(Date.self, forKey: .startedAt)
        finishedAt = try container.decodeIfPresent(Date.self, forKey: .finishedAt)
    }

    /// Whether the files this call lists as edited are on disk. A tool's `edits` are what
    /// it was asked to write, and they are on its row from the moment the row appears:
    /// they landed only once the tool completed, so a write still waiting on an approval,
    /// a declined one and a failed one have changed nothing. A command's edits are the
    /// other way round, read back from the working tree after it ran (see
    /// `ThreadRuntime.settleCommand`), so they are on disk whenever there are any,
    /// whatever its exit code said.
    var hasLandedEdits: Bool {
        guard !edits.isEmpty else { return false }
        switch status {
        case .completed: return true
        case .failed: return kind == .command
        case .running, .declined: return false
        }
    }

    /// The edits that are on disk: `edits` once `hasLandedEdits` says they landed, none
    /// before. The readers that turn a row's edits into "this file was changed" (a turn's
    /// files, a hand's landing, a merge's file list, a command's already-reported paths)
    /// read this rather than `edits`, so what counts as landed is decided in one place.
    /// Showing what a tool was asked to write, as the row's own diff does, still reads `edits`,
    /// and so does `AppModel.crewChatContext`, which is meant to over-cover: a write still
    /// waiting on its approval may land, and a hand should keep off that file meanwhile.
    var landedEdits: [FileEdit] {
        hasLandedEdits ? edits : []
    }
}

struct FileEdit: Codable, Hashable, Sendable {
    var path: String
    var diff: String?
    var additions = 0
    var deletions = 0
}

struct ProposedPlan: Codable, Hashable, Sendable {
    enum State: String, Codable, Sendable {
        case drafting
        case proposed
        case accepted
        case dismissed
    }

    var markdown: String
    var state: State
}

struct TodoStep: Codable, Hashable, Sendable {
    enum Status: String, Codable, Sendable {
        case pending
        case active
        case done
    }

    var text: String
    var status: Status
}

struct Notice: Codable, Hashable, Sendable {
    enum Level: String, Codable, Sendable {
        case info
        case warning
        case error
    }

    var level: Level
    var message: String
    /// Set on the usage-limit notice alone: the chat whose auto-continue it belongs to.
    /// Its row draws the badge with the two controls while that chat waits (see `AutoContinue`).
    var limit: LimitNotice? = nil
}

/// The chat a usage-limit notice belongs to. `AutoContinue` marks its notice with this,
/// and the notice's row reads that chat's wait and outcome from it.
struct LimitNotice: Codable, Hashable, Sendable {
    var threadID: UUID
}

enum TurnStatus: String, Codable, Sendable {
    case running
    case completed
    case interrupted
    case failed
}

struct TurnSummary: Codable, Hashable, Sendable {
    var turnID: UUID
    var status: TurnStatus
    var duration: TimeInterval
    var filesChanged: Int
    var additions: Int
    var deletions: Int
    var changes: FileChangeSummary?
}

/// One outcome of the Crew auto-merge for this thread, kept so the lead can be told the true merge state at the front of its next message.
struct CrewMergeRecord: Codable, Hashable, Sendable {
    /// `held`: some job files are claimed by another chat's unfinished work, so this
    /// merge took nothing and nothing was spent; it goes out once that claim clears.
    enum Outcome: String, Codable, Sendable { case merged, failed, stray, held }
    var at: Date
    var outcome: Outcome
    var project: String?
    /// For a merge: the request's "!12" or "#12", the pushed branch, or the local commit
    /// with the branch it went on, by `mode`.
    var label: String?
    var url: URL?
    var files: Int
    var detail: String?
    /// What a `merged` outcome amounts to (see `CrewMergeMode`). Nil on the other outcomes
    /// and on records from before the modes, which were all merge requests.
    var mode: CrewMergeMode? = nil
}

struct TurnRecord: Codable, Identifiable, Hashable, Sendable {
    var id = UUID()
    var index: Int
    var providerTurnID: String?
    var startedAt = Date.now
    var completedAt: Date?
    var status: TurnStatus = .running
    var baseCheckpoint: String?
    var endCheckpoint: String?
    var userItemID: String?
    /// The provider's own diff, used when the workspace is not a git repository.
    var providerDiff: String?
    /// The provider's last message id in this turn, used to rewind a conversation.
    var providerAnchor: String?
    /// Repository-relative paths this turn's agent edited, so changes made elsewhere in the
    /// repository during the turn are never attributed to the thread. Only edits that landed
    /// count (see `ToolCall.landedEdits`); turns recorded before T-068 may also list paths of
    /// writes that were declined, failed or never approved. Nil on turns recorded before.
    var touchedPaths: [String]?
    /// What the providers' ledgers counted on this turn, one report per provider, model and
    /// hand, so a run's spend is attributable after the app has moved on. Nil on turns
    /// recorded before, and on those no ledger reported for.
    var tokenSpends: [TokenSpend]?
    /// This turn's work has landed on the branch: the auto-merge took it, and the next one
    /// starts from what came after it.
    var crewMerged = false
    /// Repository-relative paths whose changes from this turn were undone from the timeline
    /// or the changes page (see `ThreadRuntime.revertFiles`). Nil until one is.
    var revertedPaths: [String]?
}

extension TurnRecord {
    /// Read field by field, so a turn stored by an earlier build, which knew nothing of
    /// the newest of them, still reads as the turn it was rather than being dropped.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = container.value(.id, default: UUID())
        index = container.value(.index, default: 0)
        providerTurnID = container.value(.providerTurnID, default: nil)
        startedAt = container.value(.startedAt, default: Date.now)
        completedAt = container.value(.completedAt, default: nil)
        status = container.value(.status, default: .running)
        baseCheckpoint = container.value(.baseCheckpoint, default: nil)
        endCheckpoint = container.value(.endCheckpoint, default: nil)
        userItemID = container.value(.userItemID, default: nil)
        providerDiff = container.value(.providerDiff, default: nil)
        providerAnchor = container.value(.providerAnchor, default: nil)
        touchedPaths = container.value(.touchedPaths, default: nil)
        tokenSpends = container.valuesIfPresent(.tokenSpends)
        crewMerged = container.value(.crewMerged, default: false)
        revertedPaths = container.valuesIfPresent(.revertedPaths)
    }
}

struct ContextUsage: Codable, Hashable, Sendable {
    var usedTokens: Int
    var windowTokens: Int?

    var fraction: Double? {
        guard let windowTokens, windowTokens > 0 else { return nil }
        return min(1, Double(usedTokens) / Double(windowTokens))
    }
}

struct ThreadDocument: Codable, Equatable, Sendable {
    var threadID: UUID
    var items: [TimelineItem] = []
    var turns: [TurnRecord] = []
    var usage: ContextUsage?
    var followUps: [FollowUpPrompt] = []
    var crewMerges: [CrewMergeRecord] = []
    /// The forwarded chat this thread continues, when the user opened it with one. Nil for
    /// a thread that started here. Optional, so threads stored before it still decode.
    var continuation: ThreadContinuation? = nil
    /// Session log lines the agent has not been told yet (see `SessionLog`): they go out with
    /// its next turn, and stay here until that turn has reached the provider.
    var sessionLogPending: [String] = []

    init(threadID: UUID) {
        self.threadID = threadID
    }

    /// A row, turn or queued prompt this build cannot read goes alone; a list that is not a list
    /// makes the whole file unreadable, so it is kept aside rather than saved over with nothing
    /// (see `StoredFile`).
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        threadID = try container.decode(UUID.self, forKey: .threadID)
        items = try container.documentValues(.items)
        turns = try container.documentValues(.turns)
        usage = container.value(.usage, default: nil)
        followUps = try container.documentValues(.followUps)
        crewMerges = try container.documentValues(.crewMerges)
        continuation = container.value(.continuation, default: nil)
        sessionLogPending = container.values(.sessionLogPending)
    }
}
