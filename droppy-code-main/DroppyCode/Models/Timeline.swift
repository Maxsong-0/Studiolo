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
    /// Set when the message is Hydra's rather than the user's own words: the roster
    /// places of the heads reporting back, or empty for a note from Hydra itself.
    var hydraHeads: [Int]?
    /// Set when the message is the lead's brief to a head, shown as a pill rather
    /// than a plain bubble. Optional, so threads stored before it still decode.
    var hydraBrief: Bool?

    var isFromHydra: Bool { hydraHeads != nil }
    var isHydraReport: Bool { !(hydraHeads ?? []).isEmpty }
    var isHydraBrief: Bool { hydraBrief == true }
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
    /// repository during the turn are never attributed to the thread. Nil on turns recorded before.
    var touchedPaths: [String]?
    /// This turn's work has landed on the branch: the auto-merge took it, and the next one
    /// starts from what came after it.
    var hydraMerged = false
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
        hydraMerged = container.value(.hydraMerged, default: false)
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

    init(threadID: UUID) {
        self.threadID = threadID
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        threadID = try container.decode(UUID.self, forKey: .threadID)
        items = container.value(.items, default: [Lenient<TimelineItem>]()).compactMap(\.value)
        turns = container.value(.turns, default: [Lenient<TurnRecord>]()).compactMap(\.value)
        usage = container.value(.usage, default: nil)
        followUps = container.value(.followUps, default: [Lenient<FollowUpPrompt>]()).compactMap(\.value)
    }
}
