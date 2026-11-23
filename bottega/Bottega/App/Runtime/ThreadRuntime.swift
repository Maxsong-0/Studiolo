// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Foundation
import SwiftUI

/// One timeline row. A class, so a streaming row redraws without re-diffing the whole thread.
@MainActor
@Observable
final class TimelineEntry: Identifiable {
    enum Kind {
        case user
        case assistant
        case reasoning
        case tool
        case plan
        case todos
        case notice
        case turnEnd
    }

    let id: String
    /// Fixed for the row's lifetime, so grouping never observes streaming content.
    let kind: Kind
    let turnID: UUID?
    var item: TimelineItem

    init(_ item: TimelineItem) {
        id = item.id
        turnID = item.turnID
        kind = switch item.content {
        case .user: .user
        case .assistant: .assistant
        case .reasoning: .reasoning
        case .tool: .tool
        case .plan: .plan
        case .todos: .todos
        case .notice: .notice
        case .turnEnd: .turnEnd
        }
        self.item = item
    }
}

/// Entries compare by identity: a row observes its entry's content itself, so the same
/// object always means the same row, and a rebuilt timeline can skip the rows whose
/// entries it already shows.
extension TimelineEntry: Equatable {
    nonisolated static func == (lhs: TimelineEntry, rhs: TimelineEntry) -> Bool {
        lhs === rhs
    }
}

/// A message from earlier in the chat that the next message talks about: shown as a
/// chip in the chat box, sent ahead of the typed words as a quote.
struct ReplyQuote: Identifiable, Equatable, Sendable {
    var id = UUID()
    var text: String

    var excerpt: String {
        guard let line = text.split(separator: "\n", omittingEmptySubsequences: false).map({ $0.trimmingCharacters(in: .whitespaces) }).first(where: { !$0.isEmpty }) else { return "" }
        let collapsed = line.split(omittingEmptySubsequences: true, whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        guard collapsed.count > 72 else { return collapsed }
        return String(collapsed.prefix(72)) + "…"
    }
}

extension ReplyQuote {
    /// `text` as a markdown blockquote: every line behind a `> `, a blank one behind `>`
    /// alone. What a quote in the chat box goes out as (see `ComposerDraft.outgoingText`),
    /// and what a message's "Copy as quote" puts on the pasteboard.
    nonisolated static func blockquote(_ text: String) -> String {
        text.split(separator: "\n", omittingEmptySubsequences: false).map { $0.isEmpty ? ">" : "> " + $0 }.joined(separator: "\n")
    }

    /// A sent message read back into the quotes it led with and the words after them,
    /// the reverse of `ComposerDraft.outgoingText`: each `> ` block up to a blank line
    /// that no further `>` line follows. A leading slash command stays with the words.
    /// A message with no leading quote comes back whole.
    nonisolated static func peel(_ text: String) -> (quotes: [ReplyQuote], body: String) {
        var lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        var command: String?
        if let first = lines.first, first.hasPrefix("/"), lines.count > 2, lines[1].isEmpty, lines[2].hasPrefix(">") {
            command = first
            lines.removeFirst(2)
        }
        var quotes: [ReplyQuote] = []
        var index = 0
        while index < lines.count, lines[index].hasPrefix(">") {
            var block: [String] = []
            while index < lines.count, lines[index].hasPrefix(">") {
                var line = lines[index].dropFirst()
                if line.hasPrefix(" ") { line = line.dropFirst() }
                block.append(String(line))
                index += 1
            }
            quotes.append(ReplyQuote(text: block.joined(separator: "\n")))
            // One blank line separates a block from the next; a blank line before the words stays for the trim below.
            if index + 1 < lines.count, lines[index].isEmpty, lines[index + 1].hasPrefix(">") {
                index += 1
            }
        }
        guard !quotes.isEmpty else { return ([], text) }
        var body = lines[index...].joined(separator: "\n")
        while body.hasPrefix("\n") { body.removeFirst() }
        if let command { body = body.isEmpty ? command : command + " " + body }
        return (quotes, body)
    }
}

/// The slash command or skill the message leads with, picked from the suggestions:
/// a chip in the chat box, the first word of what goes out.
struct DraftCommand: Equatable, Sendable {
    var name: String
    var detail: String
    var isBuiltIn: Bool
}

struct ComposerDraft: Equatable {
    var text = ""
    var attachments: [Attachment] = []
    var quotes: [ReplyQuote] = []
    var command: DraftCommand? = nil

    var isEmpty: Bool {
        attachments.isEmpty && command == nil && !text.contains { !$0.isWhitespace }
    }

    /// What actually goes out: every quote as a markdown blockquote, a blank line, then
    /// the typed words.
    var outgoingText: String {
        let body: String = {
            guard !quotes.isEmpty else { return text }
            let blocks = quotes.map { ReplyQuote.blockquote($0.text) }
            return blocks.joined(separator: "\n\n") + "\n\n" + text
        }()
        guard let command else { return body }
        let head = "/\(command.name)"
        if body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return head }
        if quotes.isEmpty { return head + " " + body }
        return head + "\n\n" + body
    }
}

/// A prompt mid-edit (a queued follow-up, or a message already sent): which one, and its
/// text and files as edited so far. A class, so the rows watching which prompt is open
/// never re-render on keystrokes.
@MainActor
@Observable
final class PromptEdit {
    let id: UUID
    var text: String
    var attachments: [Attachment]

    init(id: UUID, text: String, attachments: [Attachment]) {
        self.id = id
        self.text = text
        self.attachments = attachments
    }

    var isEmpty: Bool {
        attachments.isEmpty && !text.contains { !$0.isWhitespace }
    }
}

enum RuntimePhase: Equatable {
    case idle
    case starting
    case running
}

/// Owns a thread's live state: its timeline, its provider session and its pending requests.
@MainActor
@Observable
final class ThreadRuntime {
    let threadID: UUID

    private(set) var entries: [TimelineEntry] = []
    private(set) var turns: [TurnRecord] = [] {
        // A turn leaving the thread (a load replacing them) takes any edit of its message,
        // and its cached diff, with it.
        didSet {
            if let edit = messageEdit, !turns.contains(where: { $0.id == edit.id }) { messageEdit = nil }
            if turns.count < oldValue.count {
                let kept = Set(turns.map(\.id))
                for turn in oldValue where !kept.contains(turn.id) { turnDiffs.removeValue(for: turn.id) }
            }
        }
    }
    private(set) var usage: ContextUsage?
    /// Forwarded context from another thread, kept with the thread's history rather than
    /// as a message: the source's prompt goes out ahead of the first ordinary message
    /// (see `startTurn`), and never becomes part of the user's own message or draft.
    private(set) var continuation: ThreadContinuation?
    private(set) var phase: RuntimePhase = .idle
    private(set) var isReverting = false
    private(set) var approvals: [ApprovalRequest] = []
    private(set) var questions: [QuestionRequest] = []
    private(set) var turnStartedAt: Date?
    /// Bumped whenever the thread's files may differ from what the last diff saw. The diff
    /// cache is keyed by it, so the entries of an old revision are
    /// unreachable and go with it rather than lingering until 64 newer ones push them out.
    private(set) var diffRevision = 0 {
        didSet {
            revisionDiffs = RecentCache(limit: 4)
        }
    }
    /// Queued steering prompts. Enqueued while a turn runs, each one is sent as a
    /// direct user chat message once the running turn finishes, in order.
    /// True from init until a history that was not prefetched has been decoded and installed.
    private(set) var isLoadingHistory = false
        private(set) var followUps: [FollowUpPrompt] = [] {
        // A follow-up leaving the queue (sent, deleted) takes any edit of it with it.
        didSet {
            if let edit = followUpEdit, !followUps.contains(where: { $0.id == edit.id }) { followUpEdit = nil }
        }
    }
    private(set) var crewMerges: [CrewMergeRecord] = []
    /// The queued follow-up being edited, with the edit so far. Lives on the thread rather
    /// than in the editor, so leaving the thread mid-edit and coming back finds the editor
    /// open where it was left.
    var followUpEdit: PromptEdit?
    /// Whether the queue tab's rows are folded away under its title. Lives on the thread
    /// like `followUpEdit` does: the tab leaves the screen whenever the queue empties, and
    /// a fold the reader set must not spring back open on the next queued prompt.
    var followUpsCollapsed = false
    /// Bumped by Fold all turns: every turn block on screen folds back to its answer (see
    /// `TurnFinishedBlock`), the running turn's steps with them.
    private(set) var foldGeneration = 0
    /// The turns Fold all turns folded, so a turn's block the timeline only builds later,
    /// scrolled to, opens folded too, even where turns open on their steps. Read as a block
    /// is made, never observed.
    @ObservationIgnored private(set) var foldedTurnIDs: Set<UUID> = []
    /// Bumped to put the caret in this chat's box (see `ComposerView`).
    private(set) var composerFocusRequest = 0

    /// Counts the turns that start on their own — a queued follow-up, a hand's report, the
    /// budget note — rather than on a message the reader just sent. The timeline compares it
    /// with the value it last saw, so an unattended turn never drags a reader who scrolled up
    /// down to the end. Never observed: the timeline reads it directly.
    @ObservationIgnored private(set) var autoStartedTurnToken = 0
    /// The sent message being edited (by its turn), the same way.
    var messageEdit: PromptEdit?

    var draft = ComposerDraft() {
        didSet {
            let isEmpty = draft.isEmpty
            if isEmpty != draftIsEmpty { draftIsEmpty = isEmpty }
        }
    }
    /// `draft.isEmpty`, flipped only when it changes: the menu bar's commands read this
    /// rather than the draft, so a keystroke never re-evaluates the command tree.
    private(set) var draftIsEmpty = true
    /// The terminal drawer at the foot of the chat. A thread's own: two chats can each
    /// have one open, and the right sidebar has nothing to do with it.
    var isTerminalVisible = false {
        // Opened, the terminal takes the keyboard (see `TerminalHost`); a thread switched to
        // with its terminal already open leaves the keyboard where it is.
        didSet { if isTerminalVisible, !oldValue { terminalWantsFocus = true } }
    }
    @ObservationIgnored var terminalWantsFocus = false
    /// True while the terminal's divider is held: the pane's height follows the pointer, and
    /// the chat's panels follow it with no spring until it is let go.
    var isTerminalResizing = false
    /// The right sidebar's changes page, as this thread sees it. The column is the
    /// window's, not a thread's, so it only ever answers for the thread on screen; the
    /// switches, the shortcuts and everything else that opens a page writes through these,
    /// so there is still one flag per page to read (see `RightSidebarLayout`).
    var isDiffVisible: Bool {
        get { page == .changes }
        set { show(.changes, newValue) }
    }
    /// The right sidebar's Crew page, as this thread sees it.
    var showsCrewDeck: Bool {
        get { page == .crew }
        set { show(.crew, newValue) }
    }
    /// The right sidebar's task page, as this thread sees it.
    var showsTasks: Bool {
        get { page == .task }
        set { show(.task, newValue) }
    }

    /// The page the right sidebar is on, for this thread: nothing while another thread is
    /// on screen, so a hand of a chat in the background never opens the column.
    private var page: RightSidebarLayout.Tab? {
        guard let app, app.selectedThreadID == threadID else { return nil }
        return app.rightSidebar.tab
    }

    /// Opens a page, or closes the column when that page is the one on show.
    private func show(_ tab: RightSidebarLayout.Tab, _ wanted: Bool) {
        guard let app, app.selectedThreadID == threadID else { return }
        if wanted {
            app.rightSidebar.open(tab)
        } else if app.rightSidebar.tab == tab {
            app.rightSidebar.close()
        }
    }
    /// Which corner of the chat pane the helper panel this thread spawned sits in.
    var subagentPanelDock: PanelDockCorner = .bottomTrailing
    /// The hand last picked, highlighted in the team popover.
    var crewSelectedHandID: UUID?
    /// A hand the deck should scroll to and open, asked for by the team popover; the
    /// deck clears it once done (see `CrewDeck`).
    var crewDeckReveal: UUID?
    /// The deck's finished hands are folded under their header. "Clear finished hands"
    /// folds them; they stay on the page either way (see `CrewDeck`).
    var crewFinishedFolded = false
    /// The order the user gave the panels inside their corners by dragging; a panel not
    /// listed comes after the listed ones, in the built-in order: helper, team, popped
    /// usage.
    var panelStackOrder: [FloatingPanelID] = []
    /// Hands by the tool row that stands for them in this timeline, so the row can show
    /// who was sent out.
    private(set) var crewToolHands: [String: UUID] = [:]
    /// Hands by the provider's own id for them, for the events that come from inside them.
    @ObservationIgnored private var crewNativeHands: [String: UUID] = [:]
    /// Reports from hosted hands waiting for the lead to be idle.
    @ObservationIgnored private var crewPendingReports: [CrewReport] = []
    /// The reports' turn is on its way (see `flushCrewReports`): a hand finishing in the
    /// meantime joins it rather than starting a turn of its own.
    @ObservationIgnored private var crewFlushScheduled = false
    /// The landing under way in this lead's checkout (see `AppModel.landCrewHand`); the
    /// next hand's waits behind it, so two never write the same files at once.
    @ObservationIgnored var crewLanding: Task<Void, Never>?
    /// Delegations still in flight: the hands of each batch report together.
    @ObservationIgnored private var crewBatches: [UUID: CrewBatch] = [:]
    /// Delegated tasks past the pair's limit, sent out as hands finish, each with when it
    /// joined the queue: that is its hand's `queuedAt`, so the time it waited shows as queue.
    @ObservationIgnored private var crewWaiting: [(delegation: CrewDelegation, batchID: UUID, queuedAt: Date)] = [] {
        didSet {
            crewWaitingCount = crewWaiting.count
            // Nothing left to hold once the queue is empty (sent, or dropped).
            if crewWaiting.isEmpty { crewHeldForGo = false }
        }
    }
    /// The queued tasks wait for the user's go-ahead (Crew set to hold hands, see
    /// `noteHeldHands`): none goes out until `/go`, not even into the room a finishing hand
    /// makes, which would otherwise send the held ones out unasked.
    @ObservationIgnored private var crewHeldForGo = false
    /// How many delegated tasks wait for a hand to finish before they go out, for the
    /// hands popover to say so: a team held to the cap is not a team that lost its tail.
    private(set) var crewWaitingCount = 0
    /// How many times hands have gone out for the user's current request; a message of
    /// the user's own starts the count over.
    @ObservationIgnored private var crewDelegationRounds = 0
    /// Which hand claimed which file this request, so a later entry naming a file an
    /// earlier one took is caught while both are still queued. Cleared with the rounds.
    @ObservationIgnored private var crewClaimedPaths: [String: String] = [:]
    /// The delegation block being read as the reply streams: which turn and row it
    /// belongs to, the batch its hands go out under, and how many entries have gone
    /// out so far. One block per turn: the first row with one claims it.
    @ObservationIgnored private var crewStream: (turnID: UUID, entryID: String, batchID: UUID, sent: Int)?
    /// How many turns of the user's current request were spent telling the lead its block
    /// could not be read or was held back. Each such message is a turn the lead answers,
    /// and a lead that answers with the same block again would be told again, without end:
    /// past `CrewLimits.maxRefusedBlocks` the block is simply dropped and the turn ends. Starts over
    /// with the rounds, on a message of the user's own.
    @ObservationIgnored private var crewRefusedBlocks = 0
    /// A report told the lead to hold the one verification until the last hand is back, and
    /// none has told it to run it yet: the report that finds no hand still out does, even
    /// when its own hands changed nothing (see `CrewPrompts.verificationStep`). Starts over
    /// with the rounds.
    @ObservationIgnored private var crewVerificationOwed = false
    /// A hosted hand's time for one turn (see `CrewLimits.maxSeconds`): past it, the hand is
    /// stopped and its next turn is its report.
    @ObservationIgnored private var handBudget: Task<Void, Never>?
    @ObservationIgnored private var handBudgetSpent = false
    @ObservationIgnored private var handReportsNext = false
    /// The team's finished work is on its way to the remote (see `AppModel.autoMergeCrewWork`).
    var isCrewMerging = false
    /// What the merge is doing right now, in words, for the timeline: gathering the
    /// team's files, writing the commit, pushing, opening the merge request, merging.
    var crewMergeStage: String?
    /// The stage under way, for the pill's track of dots (see `CrewMergeStage`); nil
    /// while no merge runs.
    var crewMergeStep: CrewMergeStage?
    /// When the merge under way began, so the timeline can show how long it has been at it.
    var crewMergeStartedAt: Date?
    /// The id the merge's outcome note will land under, chosen when the merge starts. The
    /// timeline draws its merging pill as the block with this id, so the note takes the
    /// pill's place under the same identity and the pill morphs into the merged report
    /// rather than being replaced (see `DisplayBlock.merging`).
    var crewMergeNoteID: String?
    /// A native hand's progress while it runs (its provider's note, its tool count, its
    /// spend), kept here rather than on the thread record: the provider reports it many
    /// times a second, and a write to a thread re-renders everything that lists threads.
    /// Only the panel row that shows this hand follows these; they land on the record
    /// once the hand finishes (see `AppModel.finishCrewHand`).
    var crewActivity: String?
    var crewToolCalls = 0
    /// The spend is not observed at all: it moves with every report, and the views that
    /// show it (the count on the hand's deck card, the usage card at the top of the deck)
    /// read it on a tick of their own instead.
    @ObservationIgnored var crewTokens = 0
    /// When the hand last did anything (a tool, a word of its reply or its reasoning):
    /// what the watchdog reads, with the two below, to tell a hand that is thinking from
    /// one that has stalled (see `AppModel.startCrewWatchdog`).
    /// Not observed: it moves with every flush of every hand, and the one view that
    /// reads it (the lead's working pill) is redrawn by the watchdog's label instead.
    @ObservationIgnored var crewLastEventAt: Date?
    /// How long the hand has been quiet, in seconds; zero before it has done anything.
    var crewIdleSeconds: TimeInterval { crewLastEventAt.map { Date.now.timeIntervalSince($0) } ?? 0 }

    /// How many files the hand's tools have written in the turn under way. Read off the
    /// turn's rows each time rather than counted as the rows arrive: a tool's edits are
    /// on its row from the start, before any approval, and a count taken then (as this
    /// once was) took every proposed write for a written one and never started over
    /// with the next turn. Only edits that landed count (see `ToolCall.hasLandedEdits`).
    var crewLandedEdits: Int {
        guard let turnID = currentTurnID else { return 0 }
        return entries.reduce(0) { total, entry in
            guard entry.item.turnID == turnID, case .tool(let call) = entry.item.content, call.hasLandedEdits else { return total }
            return total + call.edits.count
        }
    }

    /// Whether the hand is stopped on something only a person can give it: an approval to
    /// run or write, a plan to accept, a question to answer. Only a person answers them,
    /// so one left unanswered holds the hand there for as long as it lives.
    var crewAwaitsPerson: Bool { !approvals.isEmpty || !questions.isEmpty }

    /// What this hand has spent so far, in tokens: the running total a native hand's
    /// provider reports, or what a hosted hand's own turns recorded, whichever is more.
    /// Follows the turns but not `crewTokens`, so a view showing it reads it on a tick.
    var crewSpentTokens: Int {
        let recorded = turns.reduce(0) { total, turn in
            total + (turn.tokenSpends ?? []).reduce(0) { $0 + $1.totalTokens }
        }
        return max(crewTokens, recorded)
    }

    private struct CrewBatch {
        var pending: Set<UUID>
        var reports: [CrewReport] = []
    }
    var diffSelection: UUID?
    /// The edits a tapped tool row asked to see, patches included. The page leaves
    /// their cards expanded and scrolls to the first of them, and falls back to a patch
    /// when the turn's diff has nothing for it; empty for the tab and the Review button,
    /// which open the whole selection.
    private(set) var diffFocusEdits: [FileEdit] = []
    /// Bumped whenever `diffFocusEdits` changes, so the page reloads its file list
    /// even when a second row names the same file with a different patch.
    private(set) var diffFocusRevision = 0

    /// Opens the changes popover on `anchor` (a tool row or a Review button), hanging
    /// from its `edge`, showing `turn`'s changes, or the whole thread's for nil.
    /// `focusEdits` are the tapped row's own edits, so the file the page opens on is
    /// what the row reports even when the turn's diff cannot show it yet.
    func showDiff(turn: UUID?, focusEdits: [FileEdit] = []) {
        if diffSelection != turn { diffSelection = turn }
        setDiffFocus(focusEdits)
        if !isDiffVisible { isDiffVisible = true }
    }

    /// Drops the edits a tapped tool row focused, so the openers that show a whole
    /// selection (the changes page, ⌘D) never open on some earlier row's file.
    func clearDiffFocus() {
        setDiffFocus([])
    }

    private func setDiffFocus(_ edits: [FileEdit]) {
        let focus = edits.filter { !$0.path.isEmpty }
        guard focus != diffFocusEdits else { return }
        diffFocusEdits = focus
        diffFocusRevision += 1
    }

    /// The toolbar button, ⌘D and the palette: they show the whole
    /// selection, so any file a tapped tool row focused is dropped first and the page
    /// opens at the top of the list rather than on some earlier row's file.
    func toggleDiff() {
        clearDiffFocus()
        isDiffVisible.toggle()
    }

    @ObservationIgnored private weak var app: AppModel?
    @ObservationIgnored private var session: (any ProviderSession)?
    /// A session being started ahead of the first message (`warmSession`).
    @ObservationIgnored private var warmup: Task<Void, Never>?
    /// The session's ID while no message has gone through it. The provider has nothing
    /// to resume under such an ID: a session warmed while the first message was typed and
    /// replaced before it went (a setting changed the launch) was resumed as "no
    /// conversation found", and the chat was told its agent had lost a conversation that
    /// never was. So the ID reaches the thread with the first message, and a session
    /// replaced before then is followed by one that starts fresh.
    @ObservationIgnored private var unsentSessionID: String?
    @ObservationIgnored private var lastFlushAt = ContinuousClock.now - .seconds(1)
    @ObservationIgnored private var sessionSignature: SessionSignature?
    /// Whether the anti-slop policy has already gone out as a fallback prefix on this
    /// session. The providers that cannot keep it in a system prompt get it once, with
    /// the first turn that carries text; a restarted, retried or explicitly compacted
    /// session takes it again (see `releaseSession`, `compact`).
    @ObservationIgnored private var antiSlopFallbackSent = false
    /// Whether the forwarded continuation has already gone out as a prefix on this
    /// session. Delivered once, with the first ordinary message that goes through the
    /// session; a restarted, retried or explicitly compacted session takes it again
    /// (see `releaseSession`, `compact`).
    @ObservationIgnored private var continuationSent = false
    /// Session log lines the agent has not been told (see `SessionLog`): put in front of its
    /// next turn's prompt, and let go of only once that turn has reached the provider.
    @ObservationIgnored private var sessionLogPending: [String] = []
    /// Which session the runtime is listening to. Every session made carries the count it
    /// was made under, and its events are dropped once the count has moved on: a CLI that
    /// closes long after it was let go used to end the turn that replaced it, take down
    /// the live session with it and leave its process running with nothing holding it.
    @ObservationIgnored private var sessionEpoch = 0
    /// Whether `ensureSession` is waiting on a session's `start()`. A session that exits
    /// then is reported by that throwing, and `ensureSession` starts over where it can,
    /// so its exit event neither lets the session go nor fails the turn.
    @ObservationIgnored private var isStartingSession = false
    /// The turn being finalized, claimed before the first suspension: a completion and an
    /// exit can both arrive for one turn, and the second must not finalize it again.
    @ObservationIgnored private var finishingTurnID: UUID?
    @ObservationIgnored private var entryIndex: [String: TimelineEntry] = [:]
    @ObservationIgnored private var pendingDeltas: [String: PendingDelta] = [:]
    @ObservationIgnored private var flushTask: Task<Void, Never>?
    /// Bumped by anything that changes what `saveNow` writes, so the periodic save
    /// during a quiet stretch finds nothing new and does no work.
    @ObservationIgnored private var saveRevision = 0
    @ObservationIgnored private var lastSavedRevision = 0
    @ObservationIgnored private var saveTask: Task<Void, Never>?
    @ObservationIgnored private var historyLoad: Task<Void, Never>?
    @ObservationIgnored private var persistenceEnabled = true
    /// What went wrong reading the thread's file, if anything did (see `Storage.openDocument`):
    /// the file was kept aside and the thread went on without it, or it is still there unread
    /// and nothing is saved over it this run. The timeline says which (see `finishLoad`).
    @ObservationIgnored private(set) var loadProblem: StorageProblem?
    @ObservationIgnored private var currentTurnID: UUID?
    /// The turn being closed (see `finishTurn`): its id is taken out of `currentTurnID`
    /// first, and its diffs are then waited for. An event landing in that wait (a late
    /// tool result, a notice, a Crew note) still belongs to the turn; filed under no turn
    /// it split the turn's rows into two blocks with one id, and the second, the running
    /// tail, stopped drawing.
    @ObservationIgnored private var closingTurnID: UUID?
    /// The pair whose hands-provider fallback was already noted in this timeline.
    @ObservationIgnored private var crewFallbackNoted: UUID?
    /// The turn a new row is filed under.
    private var turnIDForNewRows: UUID? { currentTurnID ?? closingTurnID }
    /// The provider's latest diff and resume anchor for the running turn. Codex re-sends its
    /// whole turn diff on every file change and Claude names an anchor per message; both land
    /// on the turn record once, as the turn finishes, since every write to `turns` re-runs
    /// the whole timeline body.
    @ObservationIgnored private var currentProviderDiff: String?
    @ObservationIgnored private var currentProviderAnchor: String?
    @ObservationIgnored private var interruptWatchdog: Task<Void, Never>?
    /// The tree each running command tool is diffed from, by tool id, and
    /// the diffs being settled as they finish (see `watchCommand`).
    @ObservationIgnored private var commandTrees: [String: Task<String?, Never>] = [:]
    @ObservationIgnored private var commandSettles: [String: Task<Void, Never>] = [:]
    /// Snapshots run one after another: each is several git processes over the whole
    /// checkout, and a turn can have many commands in flight at once.
    @ObservationIgnored private var treeCaptures: Task<String?, Never>?
    /// The tree a command left behind, with the count below as it stood when it was taken.
    /// While that count has not moved, nothing can have changed the working tree since, so
    /// the next command takes it as its own "before" and snapshots nothing.
    @ObservationIgnored private var settledTree: (tree: String, epoch: Int)?
    /// The last tree any command of this turn finished capturing, whatever ran since: a
    /// command starting later than that is diffed from it when no settled tree stands,
    /// rather than from a snapshot started alongside it that the command could outrun.
    @ObservationIgnored private var latestTree: String?
    /// Bumped by anything that could write a file: a command starting, a tool that edits.
    @ObservationIgnored private var treeEpoch = 0
    /// The row a turn's todo list lives on, so each update finds it without scanning the
    /// timeline.
    @ObservationIgnored private var todosEntryID: String?
    /// Paths the lead's native hands changed during this turn: what their tools landed, and
    /// what their commands left in the tree. Their tools arrive as
    /// `.agentEvent` and are rehearsed onto the hands' own timelines, so without this the
    /// lead's turn would count none of their work and a merge would take a fraction of it.
    @ObservationIgnored private var crewTouched: Set<String> = []
    /// Snapshots around a native hand's shell commands, by hand and tool (see
    /// `watchCrewCommand`).
    @ObservationIgnored private var crewCommandTrees: [String: Task<String?, Never>] = [:]

    private struct SessionSignature: Equatable {
        var provider: ProviderKind
        var directory: String
        var launchRuntimeMode: RuntimeMode?
        var launchEffort: String?
        var launchFast: Bool?
        /// Antigravity pins model, effort, mode and plan state at launch
        /// (`--model`, `--effort`, `--mode`), so any of them restarts it.
        var launchModel: String?
        var launchInteraction: InteractionMode?
        /// Codex takes its context window in the thread's config at launch (see
        /// `CodexModelWindows`), while the model goes with each turn: a model with another
        /// largest window restarts the session, so the window asked for is the model's own.
        var launchContextWindow: Int?
        /// Providers that run hands natively define them at launch, so the team restarts
        /// the session when Crew is switched or its pair changes. Held without its limits
        /// (see `signatureShape`): no session launches with them.
        var crew: CrewLaunch?
        /// The limits a hosted hand on an API provider paces its own turn by (see
        /// `OpenAICompatibleSession`): a change to them starts the hand's next turn on a
        /// session that has them. Nil for every other thread, whose session never reads them.
        var crewHandLimits: CrewLimits?
        /// The anti-slop policy travels with the session, so flipping the setting
        /// restarts it and the policy in force is the one the session was made with.
        var antiSlopEnabled: Bool
    }

    enum DeltaKind {
        case message
        case reasoning
        case toolOutput
        case plan
    }

    private struct PendingDelta {
        var kind: DeltaKind
        var text: String
    }

    init(threadID: UUID, app: AppModel) {
        self.threadID = threadID
        self.app = app
        if let document = DocumentPrefetch.shared.take(threadID) {
            install(document)
        } else {
            // The file is read and decoded off the main thread so the click that
            // opens the chat paints at once; the runtime shows an empty history
            // until it lands. A file that did not read is not taken for an empty
            // history (see `Storage.openDocument`).
            isLoadingHistory = true
            historyLoad = Task.detached(priority: .userInitiated) { [threadID] in
                let opened = Storage.openDocument(threadID)
                await self.finishLoad(opened.document, problem: opened.problem)
            }
        }
    }

    private func install(_ document: ThreadDocument) {
        turns = document.turns + turns
        for turn in document.turns { if let paths = turn.revertedPaths, !paths.isEmpty { revertedPaths[turn.id] = Set(paths) } }
        usage = usage ?? document.usage
        continuation = document.continuation ?? continuation
        sessionLogPending = document.sessionLogPending + sessionLogPending
        followUps = document.followUps.filter { !$0.isEmpty } + followUps
        crewMerges = document.crewMerges + crewMerges
        // Thinking with no text is nothing to show or keep: Claude Code redacts its
        // reasoning and streams only empty deltas, which older builds stored as blank
        // entries. They are dropped here and never created below. Threads stored before
        // the app flattened model prose on the way in are cleaned as they are read, so
        // no reply from any earlier build can put an em dash back on screen.
        let loaded = document.items.filter { !$0.isEmptyReasoning }.map { TimelineEntry($0.cleanedOfEmDashes) }
        for entry in loaded {
            entryIndex[entry.id] = entry
            endStreaming(entry)
            if case .tool(var call) = entry.item.content, call.status == .running {
                call.finish(.failed)
                entry.item.content = .tool(call)
            }
        }
        entries = loaded + entries
        // A card left in its sent state by a quit mid-hold goes now: its hands went out long ago.
        for entry in loaded {
            guard case .assistant(var message) = entry.item.content, CrewPrompts.hasSentDelegationBlock(in: message.text) else { continue }
            message.text = CrewPrompts.withoutDelegationBlock(message.text)
            if message.text.isEmpty { message.text = "Sent out hands." }
            entry.item.content = .assistant(message)
        }
        for index in turns.indices where turns[index].status == .running {
            turns[index].status = .interrupted
        }
        // A turn the app quit under has no end marker, so it showed every step inline
        // for good; one is written for it here from the turn record, and the block
        // folds like any finished turn.
        var marked = Set<UUID>()
        for entry in entries where entry.kind == .turnEnd {
            if let turnID = entry.turnID {
                marked.insert(turnID)
            }
        }
        for turn in document.turns.reversed() {
            guard let index = turns.firstIndex(where: { $0.id == turn.id }),
                  !marked.contains(turn.id),
                  let lastIndex = entries.lastIndex(where: { $0.turnID == turn.id })
            else { continue }
            let endedAt = turns[index].completedAt ?? entries[lastIndex].item.date
            if turns[index].completedAt == nil {
                turns[index].completedAt = endedAt
            }
            let summary = TurnSummary(turnID: turn.id, status: turns[index].status, duration: max(0, endedAt.timeIntervalSince(turns[index].startedAt)), filesChanged: turns[index].touchedPaths?.count ?? 0, additions: 0, deletions: 0, changes: nil)
            let entry = TimelineEntry(TimelineItem(turnID: turn.id, date: endedAt, content: .turnEnd(summary)))
            entries.insert(entry, at: lastIndex + 1)
            entryIndex[entry.id] = entry
        }
        // The load above normalizes what it read (streaming flags cleared, running
        // tools failed), so memory may differ from disk. Read back as it was saved, the
        // document needs no write at quit; one just repaired is written once more.
        saveRevision += 1
        let installed = snapshotForPersistence()
        if installed.items == document.items, installed.turns == document.turns, installed.followUps == document.followUps, installed.crewMerges == document.crewMerges {
            lastSavedRevision = saveRevision
        }
    }

    private func finishLoad(_ document: ThreadDocument, problem: StorageProblem?) {
        guard isLoadingHistory else { return }
        install(document)
        isLoadingHistory = false
        historyLoad = nil
        guard let problem else { return }
        // Set before the notice goes in, so the save the notice asks for already knows
        // whether it may write. Left in place, the notice lives in memory alone. Kept aside,
        // it is written at once as the thread's new file, so the pointer to the old one is on
        // disk even if the app goes before the usual save would have run.
        loadProblem = problem
        appendNotice(problem.blocksSaving ? .error : .warning, problem.message)
        if !problem.blocksSaving { saveNow() }
        // The notice reaches only someone who opens the thread; a hand or a thread woken in
        // the background may never be, so the app says it as well.
        app?.noteThreadStorageProblem(problem, thread: threadID)
    }

    /// Anything that continues the history (a new turn) waits for it first.
    func ensureLoaded() async {
        if let historyLoad { await historyLoad.value }
    }

    /// Installs context only while the destination has no conversation of its own.
    func installContinuation(_ value: ThreadContinuation) async -> Bool {
        await ensureLoaded()
        guard let thread, turns.isEmpty, entries.isEmpty, thread.providerSessionID == nil else { return false }
        // Selecting the new row can warm a session before the snapshot is installed.
        releaseSession(stop: true)
        continuation = value
        saveRevision += 1
        saveNow()
        return true
    }

    var isRunning: Bool { phase != .idle }

    var thread: ChatThread? { app?.thread(threadID) }

    /// Whether any hand of this lead, native or hosted, is still at work. The native
    /// ones run inside the lead's own session, so a turn with hands at work is never
    /// interrupted on the user's behalf by a new message: only the Stop button, the stop
    /// chord and Stop and take back stop it, and those are the user stopping the lead on
    /// purpose.
    var hasWorkingHands: Bool {
        (app?.runningCrewHands(of: threadID) ?? 0) > 0
    }

    /// Whether any helper of this lead — a hand or a plain helper — is at work: the same
    /// question the folded-helpers line asks about the same helpers, so a lead's own row,
    /// its badge and the line under it never disagree.
    var hasWorkingHelpers: Bool {
        guard let app else { return false }
        for helper in app.children(of: threadID) {
            if let runtime = app.existingRuntime(for: helper.id), runtime.isRunning || runtime.isCrewMerging {
                return true
            }
        }
        return false
    }

    /// Whether any native hand of this lead is still at work: those are the ones stopping
    /// the lead's turn would take down. A hosted hand lives in a thread of its own and
    /// reports to whatever turn the lead is on, so it survives the lead being stopped.
    var hasWorkingNativeHands: Bool {
        guard let app else { return false }
        return app.runningCrewHands(of: threadID) > app.runningHostedHands(of: threadID)
    }

    /// Stop and take back (the Thread menu, the palette, a chord once one is recorded; no
    /// longer Escape, which cost a turn every time it was pressed by mistake): the turn is
    /// stopped, stays in the thread as stopped, and its message goes back in the composer to
    /// be changed and sent again. Nothing is rewound: the conversation, the provider's
    /// context and the files stay as the stopped turn left them (the next turn carries the
    /// aborted exchange along; a message changed and resent reads against it). With
    /// something already typed in the box, or hands out on the turn (their reports belong to
    /// it), the turn is only stopped.
    func takeBackRunningTurn() {
        guard draft.isEmpty, !hasWorkingHands, let turnID = currentTurnID, let turn = turns.first(where: { $0.id == turnID }),
              let itemID = turn.userItemID, let entry = entryIndex[itemID],
              case .user(let message) = entry.item.content, !message.isFromCrew else {
            interrupt()
            return
        }
        interrupt()
        Task {
            // The interrupt lands asynchronously; the watchdog ends a session that ignores
            // it within eight seconds. Words typed into the box while the stop landed are
            // the reader's, not the message's to overwrite.
            for _ in 0..<200 where phase != .idle { try? await Task.sleep(for: .milliseconds(50)) }
            guard phase == .idle, draft.isEmpty, turns.contains(where: { $0.id == turnID }) else { return }
            draft.text = message.text
            draft.attachments = message.attachments
        }
    }

    /// Fold all turns: every turn folds back to its answer, the running one's steps too.
    func foldAllTurns() {
        foldedTurnIDs.formUnion(turns.map(\.id))
        foldGeneration &+= 1
    }

    /// Puts the caret in this chat's box: the palette placing a command there.
    func requestComposerFocus() {
        composerFocusRequest &+= 1
    }

    var sentPrompts: [String] {
        entries.compactMap { entry in
            if case .user(let message) = entry.item.content, !message.isFromCrew, !message.isCrewBrief { return message.text }
            return nil
        }
    }

    var hasSentPrompts: Bool {
        entries.contains { entry in
            if case .user(let message) = entry.item.content { return !message.isFromCrew }
            return false
        }
    }

    var pendingPlanApproval: ApprovalRequest? {
        approvals.first { $0.kind == .plan }
    }

    // MARK: - Sending

    func send() {
        guard !isReverting, !draft.isEmpty, phase == .idle else { return }
        let attachments = draft.attachments
        let outgoing = draft.outgoingText.trimmingCharacters(in: .whitespacesAndNewlines)
        if handleLocalCommand(outgoing) {
            draft = ComposerDraft()
            return
        }
        draft = ComposerDraft()
        Task { await ensureLoaded(); await startTurn(text: outgoing, attachments: attachments) }
    }

    func sendCrewBrief(_ text: String, attachments: [Attachment]) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty,
              phase == .idle else { return }
        draft = ComposerDraft()
        Task { await ensureLoaded(); await startTurn(text: text, attachments: attachments, crewBrief: true) }
    }

    /// Sends a prompt macro to the lead, the palette's way of typing its command: at once
    /// while the chat is idle, else queued behind the running turn as the command, which no
    /// hand ever takes (`staysWithLead`) and which goes out as the prompt it stands for when
    /// its turn comes (`startTurn`). The draft is left as it is.
    func sendMacro(_ macro: PromptMacro) {
        guard !isReverting else { return }
        if phase == .idle {
            Task { await ensureLoaded(); await startTurn(text: macro.command, attachments: []) }
        } else {
            enqueueFollowUp(text: macro.command, attachments: [])
        }
    }

    // MARK: - Follow-up queue

    /// Whether the chat box can queue a message: while a turn runs, and while hands are
    /// out with the lead waiting on them. Command-Return sends to the idle lead at once;
    /// the queue is for what should wait until the hands have reported.
    var canQueue: Bool {
        isRunning || hasWorkingHands
    }

    /// Instantly queues the composer's draft as a follow-up while a turn runs or hands are
    /// at work. The prompt, pics and other attachments included, is sent as a direct user
    /// chat message once the running turn (or the hands' report) finishes. Stacks up:
    /// every queued prompt runs in order. With nothing to queue behind, the draft simply
    /// goes out: the queue chord is never a dead key.
    func queueDraftAsFollowUp() {
        guard !isReverting, !draft.isEmpty else { return }
        guard canQueue else {
            send()
            return
        }
        enqueueFollowUp(text: draft.outgoingText, attachments: draft.attachments)
        draft = ComposerDraft()
    }

    /// Draft captured by Command-Return while a turn runs: stop the turn, then send this
    /// right away instead of queueing it behind the queue.
    @ObservationIgnored private var pendingSend: PendingSend?
    /// A queued follow-up's turn is on its way to an idle lead (see
    /// `flushFollowUpsIfIdle`): a second prompt queued in the meantime waits its
    /// turn rather than starting one of its own, so two never double-send.
    @ObservationIgnored private var followUpFlushScheduled = false

    private struct PendingSend {
        var text: String
        var attachments: [Attachment]
    }

    /// Interrupts the running turn and sends the draft as soon as the stop
    /// lands, instead of queueing it as a follow-up. The composer clears at
    /// once; the message goes out when the turn fully stops (or immediately,
    /// if the turn finished on its own in the meantime). With hands at work the
    /// turn is left alone and the draft queues instead: see `hasWorkingHands`.
    func interruptAndSend() {
        guard !isReverting, !draft.isEmpty, phase != .idle else { return }
        // Hands at work are never stopped by a new message. The native ones run inside
        // the lead's own session, so interrupting the turn would kill them mid-task and
        // throw away minutes of their work. The draft goes to a hosted hand of its
        // own when the settings allow it, else it waits as a follow-up behind the
        // running turn; either way nothing is interrupted.
        if hasWorkingHands {
            enqueueFollowUp(text: draft.outgoingText, attachments: draft.attachments)
            draft = ComposerDraft()
            return
        }
        pendingSend = PendingSend(text: draft.outgoingText, attachments: draft.attachments)
        draft = ComposerDraft()
        interrupt()
    }

    /// Pairs the dragged follow-up onto another so they go out as one message: both
    /// carry the target's bundle (or a fresh one), and the dragged prompt moves to sit
    /// right after the bundle's last prompt.
    func bundleFollowUp(_ id: UUID, onto target: UUID) {
        guard id != target,
              let targetIndex = followUps.firstIndex(where: { $0.id == target }),
              followUps.firstIndex(where: { $0.id == id }) != nil else { return }
        let bundle = followUps[targetIndex].bundleID ?? UUID()
        followUps[targetIndex].bundleID = bundle
        var dragged = followUps.remove(at: followUps.firstIndex(where: { $0.id == id })!)
        dragged.bundleID = bundle
        // The target still carries the bundle, so this always finds at least it.
        let insertAt = (followUps.lastIndex(where: { $0.bundleID == bundle }) ?? (followUps.count - 1)) + 1
        followUps.insert(dragged, at: min(insertAt, followUps.count))
        saveRevision += 1
        scheduleSave()
    }

    /// Takes a prompt out of its bundle on purpose (the link tapped, or the row dragged
    /// out): it goes as its own message again, and a bundle left with one prompt dissolves.
    func unbundleFollowUp(_ id: UUID) {
        guard let index = followUps.firstIndex(where: { $0.id == id }), followUps[index].bundleID != nil else { return }
        followUps[index].bundleID = nil
        tidyBundles()
        saveRevision += 1
        scheduleSave()
    }

    /// Drops a prompt from its bundle when neither neighbour shares it, and dissolves
    /// a bundle left with a single prompt.
    private func unbundleIfSeparated(_ id: UUID) {
        guard let index = followUps.firstIndex(where: { $0.id == id }),
              let bundle = followUps[index].bundleID else { return }
        let prevShares = index > 0 && followUps[index - 1].bundleID == bundle
        let nextShares = index + 1 < followUps.count && followUps[index + 1].bundleID == bundle
        guard !prevShares && !nextShares else { return }
        followUps[index].bundleID = nil
        let remaining = followUps.filter { $0.bundleID == bundle }
        if remaining.count == 1, let lone = followUps.firstIndex(where: { $0.id == remaining[0].id }) {
            followUps[lone].bundleID = nil
        }
    }

    /// Clears any bundle held by only one prompt.
    private func tidyBundles() {
        var counts: [UUID: Int] = [:]
        for prompt in followUps {
            if let bundle = prompt.bundleID { counts[bundle, default: 0] += 1 }
        }
        for index in followUps.indices {
            if let bundle = followUps[index].bundleID, counts[bundle] == 1 {
                followUps[index].bundleID = nil
            }
        }
    }

    /// The merged prompt for the bundle starting at `index`, without removing it.
    private func mergedBundle(startingAt index: Int) -> FollowUpPrompt {
        let first = followUps[index]
        guard let bundle = first.bundleID else { return first }
        var end = index
        while end + 1 < followUps.count, followUps[end + 1].bundleID == bundle { end += 1 }
        let members = Array(followUps[index...end])
        var merged = first
        merged.text = Self.bundleText(members)
        merged.attachments = members.flatMap { $0.attachments }
        merged.bundleID = nil
        return merged
    }

    /// The text of a bundle going out as one message. With no attachment among the
    /// members, their texts one after the other; otherwise each message is numbered and
    /// names its own pictures (by their order among the images sent, which is the order
    /// the merged attachments keep) and files (by path) under it, so the model never has
    /// to guess which picture or file came with which of the messages.
    static func bundleText(_ members: [FollowUpPrompt]) -> String {
        let texts = members.map { $0.text.trimmingCharacters(in: .whitespacesAndNewlines) }
        guard members.count > 1, members.contains(where: { !$0.attachments.isEmpty }) else {
            return texts.filter { !$0.isEmpty }.joined(separator: "\n\n")
        }
        var imageNumber = 0
        var sections: [String] = []
        for (offset, member) in members.enumerated() {
            let number = offset + 1
            var lines = ["Message \(number):"]
            if !texts[offset].isEmpty { lines.append(texts[offset]) }
            let named = member.attachments.map { attachment -> String in
                guard attachment.isImage else { return "file \(attachment.path)" }
                imageNumber += 1
                return "image \(imageNumber) (\(attachment.name))"
            }
            if !named.isEmpty {
                lines.append("Attached to message \(number): " + named.joined(separator: ", ") + ".")
            }
            sections.append(lines.joined(separator: "\n"))
        }
        let header = "\(members.count) messages sent together. The pictures and files a message came with are named under it, the pictures numbered in the order they are attached."
        return ([header] + sections).joined(separator: "\n\n")
    }

    /// Removes the prompt at `index` and every following adjacent prompt with the same
    /// non-nil bundle, returning them as one merged prompt.
    private func takeBundle(startingAt index: Int) -> FollowUpPrompt {
        let merged = mergedBundle(startingAt: index)
        let bundle = followUps[index].bundleID
        if let bundle {
            var end = index
            while end + 1 < followUps.count, followUps[end + 1].bundleID == bundle { end += 1 }
            followUps.removeSubrange(index...end)
        } else {
            followUps.remove(at: index)
        }
        return merged
    }

    /// Sends a queued follow-up right away instead of waiting its turn: the running turn
    /// stops and the prompt goes out as soon as the stop lands, exactly as Command-Return
    /// does with the draft; idle, it simply sends. The rest of the queue waits for the new
    /// turn. With native hands at work nothing stops, as with Command-Return: the prompt
    /// goes to a hand of its own when the settings allow it, else it stays at the front of
    /// the queue. hosted hands live on through a stop, so with only those out the turn
    /// stops for the prompt as it would with none: "now" means now.
    func sendFollowUpNow(_ id: UUID) {
        guard !isReverting else { return }
        guard let index = followUps.firstIndex(where: { $0.id == id }) else { return }
        var start = index
        if let bundle = followUps[index].bundleID {
            while start > 0, followUps[start - 1].bundleID == bundle { start -= 1 }
        }
        let prompt = takeBundle(startingAt: start)
        saveRevision += 1
        scheduleSave()
        guard !prompt.isEmpty else { return }
        if phase == .idle {
            if handleLocalCommand(prompt.text.trimmingCharacters(in: .whitespacesAndNewlines)) { return }
            Task { await ensureLoaded(); await startTurn(text: prompt.text, attachments: prompt.attachments) }
        } else if pendingSend == nil {
            // Native hands at work are never stopped for a queued prompt: they run inside
            // the lead's session and an interrupt would kill them. The prompt goes to a
            // hosted hand when the settings allow it, else back to the front of the
            // queue to wait for the turn, and the lead keeps working.
            if hasWorkingNativeHands {
                if !dispatchQueuedHand(prompt) {
                    followUps.insert(prompt, at: 0)
                    saveRevision += 1
                    scheduleSave()
                }
                return
            }
            pendingSend = PendingSend(text: prompt.text, attachments: prompt.attachments)
            interrupt()
        } else {
            // Command-Return already has a message going out the moment the turn stops;
            // this one goes right behind it.
            followUps.insert(prompt, at: 0)
            saveRevision += 1
            scheduleSave()
        }
    }

    func enqueueFollowUp(text: String, attachments: [Attachment]) {
        let prompt = FollowUpPrompt(
            text: text.trimmingCharacters(in: .whitespacesAndNewlines),
            attachments: attachments
        )
        guard !prompt.isEmpty else { return }
        // With Crew on, a task queued behind a running turn goes to a hand right away,
        // with a note on what the lead is doing, instead of waiting its turn.
        if phase != .idle, dispatchQueuedHand(prompt) { return }
        followUps.append(prompt)
        saveRevision += 1
        scheduleSave()
        // An idle lead takes the next queued message at once, even with hands still
        // out: the queue only ever waits behind a running turn, never behind an
        // idle lead. Without this a prompt queued while the lead is idle (steered
        // behind hands at work) sits until the next turn ends, which may be never.
        flushFollowUpsIfIdle()
    }

    /// Hands a queued prompt to a hosted hand, when the app and the chat allow it and
    /// the pair has room for one more. Returns whether the hand went out.
    private func dispatchQueuedHand(_ prompt: FollowUpPrompt) -> Bool {
        guard let app, app.settings.crewQueueHands, !Self.staysWithLead(prompt.text) else { return false }
        return dispatchHand(prompt, origin: .queued)
    }

    /// Whether a message is one of the chat's own commands rather than a task: plan mode,
    /// compact, the go-ahead for held hands, a prompt macro. They are the lead's to act on,
    /// so no hand is sent out with one: a queued one waits for the lead.
    private static func staysWithLead(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return ["/plan", "/compact", "/go", "/crew-go"].contains(trimmed) || PromptMacro.Invocation(trimmed) != nil
    }

    /// Sends a prompt out as a hosted hand with a note on what the lead is doing. Only a
    /// working lead hands a queued prompt on, so the hand is always told it works beside
    /// one. The prompt was queued when it was written, so any wait for a free hand since
    /// counts as its hand's queue. Returns whether the hand went out.
    private func dispatchHand(_ prompt: FollowUpPrompt, origin: CrewHandInfo.Origin) -> Bool {
        guard let app, let thread, !thread.isArchived, let launch = app.crewLaunch(for: thread),
              launch.hasRoom(running: app.runningHostedHands(of: threadID)) else { return false }
        // The user's own words are never handed to a hand that will not start: in a plan chat
        // on a provider `CrewHandAuthority.planRefusal` turns away, the hand would fail at once
        // and the prompt, attachments included, would be gone. It stays queued for the lead.
        guard CrewHandAuthority.planRefusal(provider: launch.handsProvider, interaction: thread.interactionMode) == nil else { return false }
        let task = TextCleanup.singleLine(prompt.text, limit: 60)
        let context = app.crewChatContext(for: threadID)
        let text = prompt.text
        let stripsChecks = launch.stripsVerificationSteps
        return app.spawnHostedHand(from: threadID, task: task, origin: origin, attachments: prompt.attachments, queuedAt: prompt.createdAt) { persona, workplace in
            CrewPrompts.queuedHandPrompt(persona: persona, task: text, context: context, workplace: workplace, stripsVerificationSteps: stripsChecks)
        } != nil
    }

    /// Follow-ups queued while the hands were all busy go out as hands once one frees up,
    /// as long as the lead is still at work; an idle lead takes them itself, in order.
    private func dispatchQueuedFollowUps() {
        guard phase != .idle else { return }
        while !followUps.isEmpty {
            let merged = mergedBundle(startingAt: 0)
            guard dispatchQueuedHand(merged) else { break }
            _ = takeBundle(startingAt: 0)
            saveRevision += 1
            scheduleSave()
        }
    }

    /// Sends the next queued follow-up to an idle lead at once, even with hands still
    /// out. A turn the user stopped waits for them instead (see `drainFollowUps`).
    private func flushFollowUpsIfIdle() {
        guard phase == .idle, !followUpFlushScheduled, !followUps.isEmpty,
              thread?.lastStatus != .interrupted else { return }
        followUpFlushScheduled = true
        Task { await startQueuedFollowUpTurn() }
    }

    /// The turn `flushFollowUpsIfIdle` scheduled. Something that started first (the
    /// hands' reports, or the user's own word) owns the queue now: its end drains it.
    private func startQueuedFollowUpTurn() async {
        followUpFlushScheduled = false
        guard phase == .idle else { return }
        // The queue drains exactly as after a completed turn: in order, skipping
        // prompts that emptied while queued.
        _ = drainFollowUps(after: .completed)
    }

    func removeFollowUp(_ id: UUID) {
        followUps.removeAll { $0.id == id }
        tidyBundles()
        saveRevision += 1
        scheduleSave()
    }

    /// Moves a queued follow-up next to another one, for drag reordering. A no-op
    /// when it is already there, so hovering the same half never churns.
    /// Moves a follow-up to sit right before or after `target`; returns whether the queue changed.
    @discardableResult
    func moveFollowUp(_ id: UUID, to target: UUID, placeAfter: Bool) -> Bool {
        guard id != target,
              let from = followUps.firstIndex(where: { $0.id == id }),
              let to = followUps.firstIndex(where: { $0.id == target }) else { return false }
        var dest = to + (placeAfter ? 1 : 0)
        if from < dest { dest -= 1 }
        guard from != dest else { return false }
        let prompt = followUps.remove(at: from)
        followUps.insert(prompt, at: dest)
        unbundleIfSeparated(id)
        saveRevision += 1
        scheduleSave()
        return true
    }

    func updateFollowUp(_ id: UUID, text: String, attachments: [Attachment]) {
        guard let index = followUps.firstIndex(where: { $0.id == id }) else { return }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty, attachments.isEmpty {
            followUps.remove(at: index)
            tidyBundles()
        } else {
            followUps[index].text = text
            followUps[index].attachments = attachments
        }
        saveRevision += 1
        scheduleSave()
    }

    /// Sends the next queued follow-up as a direct user message. Only runs when idle after a
    /// completed turn: an interrupted turn means the user hit stop, so the queue waits for them.
    /// Returns whether a turn starts.
    private func drainFollowUps(after status: TurnStatus) -> Bool {
        guard status == .completed, phase == .idle, !followUps.isEmpty else { return false }
        var next = takeBundle(startingAt: 0)
        saveRevision += 1
        // Skip prompts that emptied while queued (an attachment file deleted on disk still counts,
        // so only the text+attachment check applies).
        while next.isEmpty, !followUps.isEmpty {
            next = takeBundle(startingAt: 0)
            saveRevision += 1
        }
        guard !next.isEmpty else {
            scheduleSave()
            return false
        }
        scheduleSave()
        if handleLocalCommand(next.text.trimmingCharacters(in: .whitespacesAndNewlines)) {
            return drainFollowUps(after: status)
        }
        // This turn starts on its own (see autoStartedTurnToken).
        autoStartedTurnToken &+= 1
        Task { await ensureLoaded(); await startTurn(text: next.text, attachments: next.attachments) }
        return true
    }

    func implementPlan(_ entryID: String) {
        guard phase == .idle, let entry = entryIndex[entryID], case .plan(var plan) = entry.item.content else { return }
        plan.state = .accepted
        entry.item.content = .plan(plan)
        saveRevision += 1
        app?.updateThread(threadID) { $0.interactionMode = .build }
        Task { await ensureLoaded(); await startTurn(text: "Implement the plan.", attachments: []) }
    }

    func dismissPlan(_ entryID: String) {
        guard let entry = entryIndex[entryID], case .plan(var plan) = entry.item.content else { return }
        plan.state = .dismissed
        entry.item.content = .plan(plan)
        saveRevision += 1
        scheduleSave()
    }

    private func handleLocalCommand(_ text: String) -> Bool {
        switch text {
        case "/plan":
            if let provider = thread?.provider { app?.providers.recordCommand("plan", for: provider) }
            app?.updateThread(threadID) { $0.interactionMode = $0.interactionMode == .plan ? .build : .plan }
            return true
        case "/go", "/crew-go":
            // Only meaningful while hands are actually waiting; otherwise it falls through
            // and goes to the agent as ordinary text.
            guard crewWaitingCount > 0 else { return false }
            releaseHeldHands()
            return true
        case "/compact" where thread?.provider == .codex || thread?.provider == .copilot || thread?.provider == .deepseek || thread?.provider == .meta || thread?.provider == .zai || thread?.provider == .pi:
            if let provider = thread?.provider { app?.providers.recordCommand("compact", for: provider) }
            compact()
            return true
        default:
            return false
        }
    }

    /// Starts a turn on `text`. `crewHands` marks the message as hands reporting back to
    /// their lead rather than the user's own words; `crewBrief` marks it as the lead's
    /// brief to a hand, shown as a pill rather than a plain bubble.
    private func startTurn(text sent: String, attachments: [Attachment], crewHands: [Int]? = nil, crewBrief: Bool = false) async {
        guard phase == .idle, !isReverting else {
            enqueueFollowUp(text: sent, attachments: attachments)
            return
        }
        guard let app, let initialThread = app.thread(threadID), let project = app.project(initialThread.projectID) else { return }
        guard !initialThread.isArchived else {
            enqueueFollowUp(text: sent, attachments: attachments)
            return
        }
        // A prompt macro's command goes out as the prompt it stands for, and here alone, as
        // the turn starts: queued, or queued again above, it stays the command, which keeps
        // it the lead's. Hands' reports and briefs are never the user's commands.
        let invocation = crewHands == nil && !crewBrief ? PromptMacro.Invocation(sent) : nil
        if let invocation { app.providers.recordCommand(invocation.macro.rawValue, for: initialThread.provider) }
        let titleSource = invocation?.titleSource ?? sent
        let text = invocation?.prompt ?? sent
        let turnIndex = (turns.map(\.index).max() ?? -1) + 1
        var turn = TurnRecord(index: turnIndex)
        let userItem = TimelineItem(turnID: turn.id, content: .user(UserMessage(text: text, attachments: attachments, crewHands: crewHands, crewBrief: crewBrief ? true : nil)))
        turn.userItemID = userItem.id
        turns.append(turn)
        saveRevision += 1
        currentTurnID = turn.id
        currentProviderDiff = nil
        currentProviderAnchor = nil
        append(userItem)
        phase = .starting
        turnStartedAt = .now
        // A hand's quiet is counted from the start of whichever turn it works: its brief,
        // the report it writes when its budget runs out, a word from the user. The files
        // it wrote are read per turn (see `crewLandedEdits`), so a clock left over from the
        // turn before would call a hand that has only just been started again quiet for as
        // long as it had been idle, and stop it before its first word.
        if initialThread.crew != nil { crewLastEventAt = .now }
        app.updateThread(threadID) {
            $0.updatedAt = .now
            $0.lastStatus = .running
        }
        // A settled thread put back to work is open again.
        app.reopenIfSettled(threadID)
        crewStream = nil
        if crewHands == nil {
            crewDelegationRounds = 0
            crewRefusedBlocks = 0
            crewVerificationOwed = false
            crewClaimedPaths = [:]
        }
        // A finished hand told more from the panel is at work again: its lead's team
        // counts it, and its next report goes out as a fresh one. Its run is open again
        // too, so the time it reports covers this turn (see `CrewHandInfo.executionFinishedAt`).
        if let info = initialThread.crew, info.kind == .hosted, info.isFinished {
            app.updateCrewHand(threadID) {
                $0.status = .running
                $0.finishedAt = nil
                $0.executionFinishedAt = nil
            }
        }
        // A hosted hand gets so long for a turn, on any provider (Settings › Crew ›
        // Limits). The API sessions stop themselves a little sooner from inside the turn;
        // this is the backstop, and the only stop a CLI hand has. With no time budget set
        // the hand is left to its tool budget and the watchdog; its report turn always
        // has one.
        let isFinalReport = handReportsNext
        handReportsNext = false
        if initialThread.crew?.kind == .hosted {
            let limits = app.settings.crewLimits
            let allowance: TimeInterval? = isFinalReport
                ? limits.reportSeconds
                : limits.maxSeconds.map { $0 + (initialThread.provider.isAPIKeyBased ? 90 : 0) }
            handBudget?.cancel()
            handBudget = nil
            if let allowance {
                handBudget = Task { [weak self] in
                    try? await Task.sleep(for: .seconds(allowance))
                    guard let self, !Task.isCancelled, self.currentTurnID == turn.id, self.phase != .idle else { return }
                    self.handBudgetSpent = !isFinalReport
                    self.interrupt()
                }
            }
        }
        scheduleSave()
        // The chat is named as its first message goes in, not when the session finally
        // takes it: a cold session can take seconds, and a first send that fails must
        // not leave the chat nameless for good. A macro names it after the words that came
        // with it, or after itself, never after its checklist.
        generateTitle(from: titleSource, attachments: attachments)

        await ensureModel(initialThread)

        let directory = initialThread.worktreePath ?? project.path
        let git = Git(directory)
        let checkpointRef = Git.checkpointRef(thread: threadID, turn: turnIndex, phase: "start")
        // Anything could have happened in the checkout since the last turn, so no snapshot
        // from it stands any more. The hands' files are not cleared here: a native hand
        // works on after the lead's reply ends, and what it changed then belongs to the
        // next turn, which takes it up at its end (see `finishTurn`).
        settledTree = nil
        latestTree = nil
        treeEpoch += 1
        // The checkpoint is several git processes over the whole checkout. It runs while
        // the session starts rather than in front of it, and both are waited for before a
        // word reaches the model, so nothing the agent does can slip past the snapshot.
        let checkpoint = Task.detached(priority: .userInitiated) { () -> Bool in
            guard await git.isRepository() else { return false }
            return (try? await git.captureCheckpoint(checkpointRef)) != nil
        }

        do {
            // A session started while the message was being typed is the one to use. The
            // session starts alongside the checkpoint. It is picked up from the runtime
            // afterwards rather than handed back as a value, because a session is not
            // Sendable and this task is one of its own.
            if let warmup { await warmup.value }
            let start = Task { () async throws -> Void in _ = try await self.ensureSession(directory: directory) }
            if await checkpoint.value {
                updateTurn(turn.id) { $0.baseCheckpoint = checkpointRef }
            }
            try await start.value
            if SlashCommand.name(in: text) != nil {
                await app.providers.loadCommands(initialThread.provider, directory: directory)
            }
            // The user can stop a turn while it is still starting.
            guard currentTurnID == turn.id, let session, let thread = app.thread(threadID) else { return }
            let command = SlashCommand.name(in: text).flatMap { name in
                app.providers.commands[ProviderRegistry.commandKey(thread.provider, directory: directory)]?.first { $0.name == name }
            }
            var prompt = text
            let files = attachments.filter { !$0.isImage }
            if !files.isEmpty {
                prompt += "\n\nAttached files:\n" + files.map { "- \($0.path)" }.joined(separator: "\n")
            }
            // A lead whose hands are hosted is told how to ask Bottega for them. A
            // session that keeps that policy in its system prompt (the API providers, and
            // the providers with hands of their own sending them out elsewhere) gets the
            // team note here; every other CLI also gets the policy, having nowhere else
            // to keep it. The user's enabled delegation request is added below both paths.
            if let launch = app.crewLaunch(for: thread) {
                let merge = launch.autoMerges ? CrewPrompts.mergeStatus(merges: crewMerges, unmergedFiles: app.crewUnmergedFileCount(of: threadID)) : nil
                if !launch.runsNatively {
                    let team = CrewPrompts.teamStatus(app.crewTeam(of: threadID).compactMap(\.crew))
                    let canDelegate = crewDelegationRounds < launch.limits.maxDelegationRounds
                    if Self.keepsCrewPolicyInSystemPrompt(thread.provider) {
                        prompt = (crewHands == nil ? CrewPrompts.fallbackTurnNote(team: team, merge: merge) : CrewPrompts.fallbackReportNote(team: team, merge: merge, canDelegate: canDelegate)) + prompt
                    } else if crewHands == nil {
                        prompt = CrewPrompts.fallbackPreamble(launch, team: team, merge: merge) + prompt
                    } else {
                        prompt = CrewPrompts.fallbackReportPreamble(launch, team: team, merge: merge, canDelegate: canDelegate) + prompt
                    }
                } else if merge != nil {
                    prompt = CrewPrompts.fallbackTurnNote(team: nil, merge: merge) + prompt
                }
                // Selecting Crew is a request to delegate. Carry that request at the
                // user-input boundary as well as keeping the routing policy in the session.
                // Automatic reports retain their round limit and review-only instructions.
                if crewHands == nil {
                    prompt = CrewPrompts.delegationRequest(launch) + prompt
                }
            }
            // Leave slash commands at the start of the input; deliver guidance with the next ordinary message.
            var injectedAntiSlopPolicy = false
            let antiSlopEpoch = sessionEpoch
            if !Self.keepsCrewPolicyInSystemPrompt(thread.provider),
               SlashCommand.name(in: text) == nil,
               !antiSlopFallbackSent {
                let instructions = AntiSlopPolicy.instructions(enabled: sessionSignature?.antiSlopEnabled ?? app.settings.antiSlopEnabled)
                if !instructions.isEmpty {
                    prompt = instructions + "\n\n" + prompt
                    injectedAntiSlopPolicy = true
                }
            }
            // Forwarded context from the source thread precedes the first ordinary
            // message, after the prompt the user's own words built. A slash command
            // consumes no ordinary prompt, so it neither carries the snapshot nor
            // spends the one delivery.
            var injectedContinuation = false
            let continuationEpoch = sessionEpoch
            if let continuation, !continuationSent, SlashCommand.name(in: text) == nil {
                prompt = continuation.prompt + "\n\n" + prompt
                injectedContinuation = true
            }
            // What happened to the files outside the agent's sight (a turn's changes undone,
            // see `SessionLog`) goes in front of the first ordinary message after it, and
            // is let go of only once that message has reached the provider.
            let sessionLogSent = SlashCommand.name(in: text) == nil ? sessionLogPending : []
            let sessionLogEpoch = sessionEpoch
            if !sessionLogSent.isEmpty {
                prompt = SessionLog.note(sessionLogSent, thread: threadID) + "\n\n" + prompt
            }
            phase = .running
            try await session.send(TurnInput(
                text: prompt,
                images: thread.provider.supportsImages ? attachments.filter(\.isImage) : [],
                // The id the session launched under (see `largestContextModel`): the bare id
                // here would switch a Claude session back off its 1M variant on the first turn.
                model: app.providers.largestContextModel(thread.model, for: thread.provider),
                effort: thread.effort,
                serviceTier: serviceTier(for: thread),
                fastMode: thread.fastMode,
                runtimeMode: thread.runtimeMode,
                interactionMode: thread.interactionMode,
                isFinalReport: isFinalReport,
                command: command
            ))
            // Failed sends and replaced sessions must leave delivery pending.
            if injectedAntiSlopPolicy, sessionEpoch == antiSlopEpoch {
                antiSlopFallbackSent = true
            }
            if injectedContinuation, sessionEpoch == continuationEpoch {
                continuationSent = true
            }
            // A send the session did not take (it was replaced meanwhile) delivered
            // nothing; lines added while the send was in flight stay pending either way.
            if !sessionLogSent.isEmpty, sessionEpoch == sessionLogEpoch {
                sessionLogPending.removeFirst(min(sessionLogSent.count, sessionLogPending.count))
                saveRevision += 1
            }
            // The session has a conversation the provider can resume from here on.
            if let unsentSessionID {
                self.unsentSessionID = nil
                app.updateThread(threadID) { $0.providerSessionID = unsentSessionID }
            }
            if let command { app.providers.recordCommand(command.name, for: thread.provider) }
        } catch {
            guard currentTurnID == turn.id else { return }
            appendNotice(.error, error.localizedDescription)
            await finishTurn(status: .failed, turnID: turn.id)
        }
    }

    /// Whether the provider's session carries the lead's Crew policy from launch: the API
    /// sessions put it in their system prompt, and the providers with hands of their own
    /// define the hands, or the policy for hosted ones, at launch. Such a session takes
    /// the team as part of its signature, so a change to the team restarts it.
    private static func keepsCrewPolicyInSystemPrompt(_ provider: ProviderKind) -> Bool {
        AppModel.crewIsNative(provider) || provider.isAPIKeyBased
    }

    /// A lead's launch as its session's signature holds it. The limits bound what
    /// Bottega does with the hands (rounds, queueing, budgets, the watchdog) and no
    /// provider launches with them, so a change to them must not restart the session and
    /// take the native hands in it down with it.
    private static func signatureShape(_ launch: CrewLaunch?) -> CrewLaunch? {
        guard var launch else { return nil }
        launch.limits = .default
        return launch
    }

    /// Codex takes fast mode per turn as a service tier; models without a fast tier send none.
    private func serviceTier(for thread: ChatThread) -> String? {
        guard thread.provider == .codex,
              let tier = app?.providers.model(thread.model, for: thread.provider)?.fastTier else { return nil }
        return thread.fastMode ? tier : "default"
    }

    /// A thread with no model yet takes the provider's default, so the session launches
    /// with the model the app would show.
    private func ensureModel(_ thread: ChatThread) async {
        guard thread.model == nil, let app else { return }
        await app.providers.loadCatalog(thread.provider)
        if let model = app.providers.defaultModel(for: thread.provider) {
            app.updateThread(threadID) {
                $0.model = model.id
                $0.effort = $0.effort ?? model.defaultEffort
            }
        }
    }

    /// The provider process, started while the user is still typing the first message,
    /// so sending does not wait for the CLI to boot: about half a second for `claude`.
    /// Only for CLI providers without a session; a send in the meantime waits for the
    /// warm-up instead of starting a second process.
    func warmSession() {
        guard phase == .idle, session == nil, warmup == nil, !isReverting,
              let app, let thread = app.thread(threadID), !thread.isArchived, !thread.provider.isAPIKeyBased,
              let project = app.project(thread.projectID) else { return }
        let directory = thread.worktreePath ?? project.path
        warmup = Task { [weak self] in
            guard let self else { return }
            await ensureModel(thread)
            guard phase == .idle, session == nil else { warmup = nil; return }
            // A start that fails is not retried on every keystroke: the send makes its own.
            if (try? await ensureSession(directory: directory)) != nil { warmup = nil }
        }
    }

    private func ensureSession(directory: String, allowNewSession: Bool = true) async throws -> any ProviderSession {
        guard let app, let thread = app.thread(threadID) else { throw ProviderError.notRunning }
        // Copilot switches modes live, except that Auto's assisted-approval judge is a
        // session flag: entering or leaving Auto resumes the session with it set right.
        let copilotLaunchMode: RuntimeMode? = thread.provider == .copilot && thread.runtimeMode == .auto ? .auto : nil
        // A pair that sends its hands out on another provider names their model from that
        // provider's catalogue, which this chat may never have loaded. The lead's brief
        // names the model, and the brief is part of the session's signature, so the
        // catalogue loads first rather than the session restarting once it has.
        if app.crewIsOn(thread), let pair = app.crewPair(for: thread), app.crewHandsProvider(of: pair) != thread.provider {
            await app.providers.loadCatalog(app.crewHandsProvider(of: pair))
        }
        // The user's rules for leads may come from a file kept elsewhere, and they are part
        // of the lead's policy and so of the signature: the file is read again first, so an
        // edit made since reaches the lead with this message (`crewLeadInstructionsText`).
        if app.crewIsOn(thread) { await app.settings.refreshCrewLeadInstructionsFile() }
        // Providers with hands of their own define them at launch (or, with the hands
        // sent out elsewhere, the policy for asking Bottega for them); the API
        // providers put the lead's Crew policy in their system prompt. Either way the
        // team is part of the session, and a change to it restarts one.
        let crew = Self.keepsCrewPolicyInSystemPrompt(thread.provider) ? app.crewLaunch(for: thread) : nil
        let signature = SessionSignature(
            provider: thread.provider,
            directory: directory,
            // OpenCode's questions are set when it launches (`OpenCodeLaunch`), so its mode restarts it too.
            launchRuntimeMode: thread.provider == .cursor || thread.provider == .grok || thread.provider == .devin || thread.provider == .opencode || thread.provider == .antigravity ? thread.runtimeMode : copilotLaunchMode,
            launchEffort: thread.provider == .claude || thread.provider == .antigravity ? thread.effort : nil,
            launchFast: thread.provider == .claude ? thread.fastMode : nil,
            launchModel: thread.provider == .antigravity ? thread.model : nil,
            launchInteraction: thread.provider == .antigravity ? thread.interactionMode : nil,
            launchContextWindow: thread.provider == .codex
                ? CodexModelWindows.largest(forAll: CodexModelWindows.models(thread: thread.model, crew: crew), environment: app.providers.environment(for: .codex))?.maximum
                : nil,
            crew: Self.signatureShape(crew),
            crewHandLimits: thread.provider.isAPIKeyBased && thread.crew?.kind == .hosted ? app.settings.crewLimits : nil,
            antiSlopEnabled: app.settings.antiSlopEnabled
        )
        if let session, session.isRunning, sessionSignature == signature { return session }
        // A message sent right after launch waits for the login shell, where a CLI installed
        // off the fallback PATH is found; this must not read as "not installed".
        await LoginEnvironment.load()
        releaseSession(stop: true)
        // The session the hands lived in is gone, and so are they.
        stopNativeHands()

        if thread.provider.isAPIKeyBased {
            guard !app.settings.apiKey(for: thread.provider).isEmpty else {
                throw ProviderError.notInstalled(thread.provider)
            }
            func makeAPISession(resumeID: String?) -> any ProviderSession {
                let configuration = SessionConfiguration(
                    provider: thread.provider,
                    executable: nil,
                    workingDirectory: URL(fileURLWithPath: directory),
                    environment: app.providers.environment(for: thread.provider),
                    resumeID: resumeID,
                    resumeAt: resumeID == nil ? nil : thread.providerResumeAt,
                    model: thread.model,
                    effort: thread.effort,
                    fastMode: thread.fastMode,
                    runtimeMode: thread.runtimeMode,
                    interactionMode: thread.interactionMode,
                    apiKey: app.settings.apiKey(for: thread.provider),
                    crew: crew,
                    transcript: apiTranscript(),
                    isCrewHand: thread.crew?.kind == .hosted,
                    crewLimits: signature.crewHandLimits ?? .default,
                    antiSlopEnabled: signature.antiSlopEnabled
                )
                let created: any ProviderSession = switch thread.provider {
                case .deepseek: DeepSeekSession(configuration: configuration)
                case .meta: MetaSession(configuration: configuration)
                case .zai: ZaiSession(configuration: configuration)
                default: DeepSeekSession(configuration: configuration)
                }
                observe(created)
                return created
            }
            var candidate = makeAPISession(resumeID: thread.providerSessionID)
            session = candidate
            sessionSignature = signature
            let sessionID: String
            var started = false
            defer {
                if !started {
                    candidate.onEvent = nil
                    if session === candidate {
                        releaseSession(stop: true)
                    } else {
                        candidate.stop()
                    }
                }
            }
            do {
                sessionID = try await candidate.start()
            } catch where thread.providerSessionID != nil && thread.providerResumeAt == nil && allowNewSession && session === candidate && !Task.isCancelled {
                // The session that would not resume is let go of entirely, handler and
                // all, before the one that starts over takes its place.
                releaseSession(stop: true)
                candidate = makeAPISession(resumeID: nil)
                session = candidate
                sessionSignature = signature
                sessionID = try await candidate.start()
            }
            guard session === candidate, !Task.isCancelled else { throw CancellationError() }
            started = true
            app.updateThread(threadID) { $0.providerSessionID = sessionID }
            return candidate
        }

        guard let executable = app.providers.executable(for: thread.provider) else {
            throw ProviderError.notInstalled(thread.provider)
        }
        func makeSession(resumeID: String?) -> any ProviderSession {
            let configuration = SessionConfiguration(
                provider: thread.provider,
                executable: executable,
                workingDirectory: URL(fileURLWithPath: directory),
                environment: app.providers.environment(for: thread.provider),
                resumeID: resumeID,
                resumeAt: resumeID == nil ? nil : thread.providerResumeAt,
                // Every model at the largest context window it takes (Claude's 1M variant
                // where the CLI lists one; Codex asks for its largest in the thread's config).
                model: app.providers.largestContextModel(thread.model, for: thread.provider),
                effort: thread.effort,
                fastMode: thread.fastMode,
                runtimeMode: thread.runtimeMode,
                interactionMode: thread.interactionMode,
                crew: crew,
                antiSlopEnabled: signature.antiSlopEnabled
            )
            let created: any ProviderSession = switch thread.provider {
            case .codex: CodexSession(configuration: configuration)
            case .claude: ClaudeSession(configuration: configuration)
            case .antigravity: AntigravitySession(configuration: configuration)
            case .copilot: CopilotSession(configuration: configuration)
            case .commandcode: CommandCodeSession(configuration: configuration)
            case .pi: PiSession(configuration: configuration)
            case .cursor, .opencode, .grok, .devin: ACPSession(configuration: configuration)
            case .deepseek: DeepSeekSession(configuration: configuration)
            case .meta: MetaSession(configuration: configuration)
            case .zai: ZaiSession(configuration: configuration)
            }
            observe(created)
            return created
        }

        var candidate = makeSession(resumeID: thread.providerSessionID)
        session = candidate
        sessionSignature = signature
        let sessionID: String
        var started = false
        isStartingSession = true
        defer {
            isStartingSession = false
            if !started {
                candidate.onEvent = nil
                if session === candidate {
                    releaseSession(stop: true)
                } else {
                    candidate.stop()
                }
            }
        }
        do {
            sessionID = try await candidate.start()
        } catch where allowNewSession && session === candidate && !Task.isCancelled && Self.startsOver(after: error, resuming: thread) {
            // The session that would not resume is let go of entirely, handler and all,
            // before the one that starts over takes its place. A conversation the provider
            // no longer has cannot be picked up at a rewind anchor either, so the anchor
            // goes with it, and the chat is told its agent starts afresh.
            releaseSession(stop: true)
            if Self.lostConversation(error) {
                app.updateThread(threadID) { $0.providerResumeAt = nil }
                if currentTurnID != nil {
                    appendNotice(.warning, "\(thread.provider.displayName) no longer has this conversation. It starts a new one here, without the messages above.")
                }
            }
            candidate = makeSession(resumeID: nil)
            session = candidate
            sessionSignature = signature
            sessionID = try await candidate.start()
        }
        guard session === candidate, !Task.isCancelled else { throw CancellationError() }
        started = true
        recordSessionID(sessionID)
        return candidate
    }

    /// Puts the session's ID on the thread, or holds it back while no message has gone
    /// through the session (`unsentSessionID`). A thread whose conversation the session
    /// picked up takes the ID at once: a provider may hand out a new one for it.
    private func recordSessionID(_ sessionID: String) {
        guard let app else { return }
        if app.thread(threadID)?.providerSessionID == nil {
            unsentSessionID = sessionID
        } else {
            app.updateThread(threadID) { $0.providerSessionID = sessionID }
        }
    }

    /// Whether a session that would not resume the thread's conversation starts over
    /// from nothing: always when the provider no longer has the conversation, and
    /// otherwise only when no rewind anchor is lost with it.
    private static func startsOver(after error: Error, resuming thread: ChatThread) -> Bool {
        guard thread.providerSessionID != nil else { return false }
        return lostConversation(error) || thread.providerResumeAt == nil
    }

    /// The provider's word for a conversation it no longer has: Claude Code's "No
    /// conversation found with session ID", Command Code's own line.
    private static func lostConversation(_ error: Error) -> Bool {
        let text = error.localizedDescription
        return text.localizedCaseInsensitiveContains("no conversation found")
            || text.localizedCaseInsensitiveContains("no longer has this conversation")
    }

    /// The conversation before the current turn as plain exchanges, for an API session
    /// that starts over: it keeps its history in memory only, so this is how a hand
    /// steered on after its session went, or any chat reopened later, still knows what
    /// was said and done. Tool calls stay out; the replies say what they did. Bounded, so
    /// a long thread does not send its whole past with every relaunch.
    private func apiTranscript() -> [TranscriptMessage] {
        // One pass to collect the exchanges, then the runs are joined rather than
        // appended into one another: repeated `+=` on array elements copies the whole
        // reply for every fragment, which is quadratic on fragmented history.
        let exclude = currentTurnID
        var fragments: [(assistant: Bool, text: String)] = []
        fragments.reserveCapacity(entries.count)
        for entry in entries where entry.item.turnID != exclude {
            switch entry.item.content {
            case .user(let message) where !message.text.isEmpty:
                fragments.append((assistant: false, text: message.text))
            case .assistant(let message) where !message.text.isEmpty:
                fragments.append((assistant: true, text: message.text))
            default:
                break
            }
        }
        var transcript: [TranscriptMessage] = []
        transcript.reserveCapacity(fragments.count)
        var index = 0
        while index < fragments.count {
            if !fragments[index].assistant {
                transcript.append(TranscriptMessage(role: .user, text: fragments[index].text))
                index += 1
                continue
            }
            // The status lines a turn streams between its tools read as one reply.
            let runStart = index
            while index < fragments.count, fragments[index].assistant { index += 1 }
            let joined = fragments[runStart..<index].map(\.text).joined(separator: "\n\n")
            transcript.append(TranscriptMessage(role: .assistant, text: joined))
        }
        var kept = Array(transcript.suffix(40))
        var budget = 60_000
        for index in kept.indices.reversed() {
            let length = kept[index].text.count
            if length <= budget {
                budget -= length
            } else if budget > 200 {
                kept[index].text = "…" + String(kept[index].text.suffix(budget))
                budget = 0
            } else {
                kept.removeSubrange(...index)
                break
            }
        }
        return kept
    }

    // MARK: - Controls

    func interrupt() {
        guard phase != .idle else { return }
        let turnID = currentTurnID
        interruptWatchdog?.cancel()
        interruptWatchdog = nil
        // Nothing has been sent yet: the provider has no turn of its own to stop, and
        // asking it would do nothing at all, leaving the watchdog to kill a session that
        // is only just starting. The turn ends here instead, and the message it was about
        // to send never goes.
        if phase == .starting {
            currentTurnID = nil
            Task { await finishTurn(status: .interrupted, turnID: turnID) }
            return
        }
        Task {
            guard currentTurnID == turnID, turnID != nil else { return }
            if let session {
                await session.interrupt()
            } else {
                await finishTurn(status: .interrupted, turnID: turnID)
            }
        }
        interruptWatchdog = Task { [weak self] in
            try? await Task.sleep(for: .seconds(8))
            guard let self, !Task.isCancelled, self.currentTurnID == turnID, turnID != nil else { return }
            self.releaseSession(stop: true)
            await self.finishTurn(status: .interrupted)
        }
    }

    func resolve(_ request: ApprovalRequest, option: ApprovalRequest.Option) {
        // A person answering is the hand's next event: it is back at work from here, and
        // its quiet starts over rather than carrying the wait it just ended.
        noteCrewHandEvent()
        approvals.removeAll { $0.id == request.id }
        if request.kind == .plan, let itemID = request.toolItemID,
           let entry = entryIndex[itemID], case .plan(var plan) = entry.item.content {
            plan.state = option.role == .approve ? .accepted : .dismissed
            entry.item.content = .plan(plan)
            saveRevision += 1
            scheduleSave()
        }
        session?.resolveApproval(request.id, optionID: option.id)
    }

    /// Approves the request on top, the one the agent waits on first, with its plain approve
    /// option (never "always"): Command-Return with nothing typed, the card's own button by
    /// keyboard. False when no request waits.
    @discardableResult
    func approveTopRequest() -> Bool {
        guard let request = approvals.first,
              let approve = request.options.first(where: { $0.role == .approve }) else { return false }
        resolve(request, option: approve)
        return true
    }

    func answer(_ request: QuestionRequest, answers: [String: [String]]) {
        noteCrewHandEvent()
        questions.removeAll { $0.id == request.id }
        session?.answerQuestion(request.id, answers: answers)
    }

    func compact() {
        guard phase == .idle else { return }
        guard let session, session.isRunning else {
            appendNotice(.info, "Nothing to compact yet.")
            return
        }
        appendNotice(.info, "Compacting context.")
        Task {
            do {
                try await session.compact()
                // Compaction replaces the session's history with a summary, and the
                // fallback prefix went with it: the next turn carries the policy again,
                // and the forwarded context along with it.
                antiSlopFallbackSent = false
                continuationSent = false
            } catch {
                appendNotice(.warning, error.localizedDescription)
            }
        }
    }

    /// A stretch of the thread's turns whose file changes are undone together.
    enum RevertScope: Hashable, Sendable {
        /// One turn.
        case turn(UUID)
        /// A turn and every turn after it: a message edited from there.
        case from(UUID)
        /// Every turn of the thread: the changes page over the whole thread.
        case thread
    }

    /// What undoing a stretch's file changes would do to each file, all of its files or
    /// `paths` alone (see `Git.previewFileRevert`). Files are hashed and merged in memory,
    /// hundreds of milliseconds on a big change, so the editor asks for it from the pencil's
    /// hover and again on the click, and both get one run. Kept while the thread's files
    /// stay as the last turn left them (`diffRevision`) and for ten seconds, so edits made
    /// outside the app are picked up soon after.
    func revertPreview(_ scope: RevertScope, paths: [String]? = nil) -> Task<RevertPreview, Error> {
        let key = RevertKey(scope: scope, paths: paths)
        if let entry = revertPreviews[key], entry.revision == diffRevision, entry.startedAt.duration(to: .now) < .seconds(10) {
            return entry.task
        }
        let revision = diffRevision
        let task = Task { [weak self] () throws -> RevertPreview in
            guard let self else { throw ProviderError.notRunning }
            let result: Result<RevertPreview, Error>
            do { result = .success(try await self.previewFileRevert(scope, paths: paths)) } catch { result = .failure(error) }
            if self.revertPreviews[key]?.revision == revision { self.revertPreviews[key]?.result = result }
            return try result.get()
        }
        // Previews of files as they were before the last change are stale for every stretch.
        revertPreviews = revertPreviews.filter { $0.value.revision == revision }
        revertPreviews[key] = RevertPreviewEntry(revision: revision, startedAt: .now, task: task, result: nil)
        return task
    }

    /// The preview already in hand for a stretch, if a hover fetched it: the editor opens on
    /// it at once instead of on a spinner.
    func cachedRevertPreview(_ scope: RevertScope, paths: [String]? = nil) -> Result<RevertPreview, Error>? {
        guard let entry = revertPreviews[RevertKey(scope: scope, paths: paths)], entry.revision == diffRevision,
              entry.startedAt.duration(to: .now) < .seconds(10) else { return nil }
        return entry.result
    }

    private struct RevertKey: Hashable {
        let scope: RevertScope
        let paths: [String]?
    }

    private struct RevertPreviewEntry {
        let revision: Int
        let startedAt: ContinuousClock.Instant
        let task: Task<RevertPreview, Error>
        var result: Result<RevertPreview, Error>?
    }

    /// Bounded by the thread's turns: only entries of the current revision are kept.
    @ObservationIgnored private var revertPreviews: [RevertKey: RevertPreviewEntry] = [:]

    /// The paths each turn has had undone (see `revertFiles`), as git names them from the
    /// repository's root, read by its changes card: kept apart from `turns`, so a card
    /// follows this alone rather than every turn record.
    private(set) var revertedPaths: [UUID: Set<String>] = [:]
    /// The paths of each turn an undo left alone, changed again since in the same lines
    /// (see `revertFiles`), for the card to say so. For this run only: the next undo of the
    /// file, or its later change, settles it.
    private(set) var revertConflicts: [UUID: Set<String>] = [:]

    /// A stretch as git is to undo it: the checkpoint before its first turn, the one after
    /// its last, and the files its agent changed in between, of `paths` alone when given.
    private struct RevertStretch {
        let git: Git
        let turns: [TurnRecord]
        let base: String
        let end: String
        /// The files the turns recorded changing, as they recorded them.
        let paths: [String]
        /// The files asked for alone, as git names them (a card's or the changes page's).
        let only: [String]?
    }

    private func revertStretch(_ scope: RevertScope, paths wanted: [String]?) throws -> RevertStretch {
        guard let app, let thread, let project = app.project(thread.projectID) else {
            throw ProviderError.failed("This thread is no longer here.")
        }
        let range: Range<Int>
        switch scope {
        case .turn(let id):
            guard let index = turns.firstIndex(where: { $0.id == id }) else { throw ProviderError.failed("This message is no longer in the thread.") }
            range = index..<(index + 1)
        case .from(let id):
            guard let index = turns.firstIndex(where: { $0.id == id }) else { throw ProviderError.failed("This message is no longer in the thread.") }
            range = index..<turns.count
        case .thread:
            range = 0..<turns.count
        }
        let stretch = Array(turns[range])
        guard let base = stretch.first?.baseCheckpoint else { throw ProviderError.failed(Self.noCheckpointMessage) }
        // The stretch ends where its last turn with a checkpoint left the files; a turn after
        // that one may only be one that changed nothing, or its changes would go unseen.
        guard let last = stretch.lastIndex(where: { $0.endCheckpoint != nil }), let end = stretch[last].endCheckpoint,
              stretch[(last + 1)...].allSatisfy({ ($0.touchedPaths ?? []).isEmpty }) else {
            throw ProviderError.failed(Self.noCheckpointMessage)
        }
        // The turns' paths as recorded, and the ones asked for as the cards name them: git
        // tells the two forms apart against the files the stretch changed (see
        // `RevertPreview.names`), so a file of the same name elsewhere is never taken.
        let paths = Set(stretch.flatMap { $0.touchedPaths ?? [] }).sorted()
        return RevertStretch(git: Git(thread.worktreePath ?? project.path), turns: stretch, base: base, end: end, paths: paths, only: wanted)
    }

    /// The files this thread's agent changed in a stretch, as undoing them would find them.
    /// Only the paths its tools and shell commands touched are looked at, so work done in the
    /// same checkout by other threads or by hand never shows up here, let alone gets undone.
    private func previewFileRevert(_ scope: RevertScope, paths: [String]?) async throws -> RevertPreview {
        let stretch = try revertStretch(scope, paths: paths)
        do {
            return try await stretch.git.previewFileRevert(base: stretch.base, end: stretch.end, paths: stretch.paths, only: stretch.only)
        } catch {
            throw await Self.checkpointError(error, stretch)
        }
    }

    /// A checkpoint let go of since its turn recorded it (every one of an archived thread's,
    /// see `pruneCheckpoints`) reads as none rather than as git's complaint about a bad
    /// revision.
    private static func checkpointError(_ error: Error, _ stretch: RevertStretch) async -> Error {
        guard await stretch.git.resolves(stretch.base), await stretch.git.resolves(stretch.end) else {
            return ProviderError.failed(noCheckpointMessage)
        }
        return error
    }

    private static let noCheckpointMessage = "No file checkpoint is available for this message, so its file changes cannot be undone."

    /// Undoes the file changes of a stretch of turns, all of its files or `paths` alone, and
    /// leaves the conversation exactly as it is: every message stays, and the provider keeps
    /// its whole history. Only the stretch's own changes come out; later changes to the same
    /// files stay, and a file whose later changes overlap them is left alone (see
    /// `Git.revertFiles`). The turns are marked with what they had undone, the timeline says
    /// what happened, and so does the thread's session log, whose new lines the agent is
    /// told at the start of its next turn (see `SessionLog`). Answers with what was done.
    @discardableResult
    func revertFiles(_ scope: RevertScope, paths: [String]? = nil) async throws -> RevertPreview {
        guard phase == .idle, !isReverting else { throw ProviderError.failed("Wait for the running turn to finish first.") }
        guard let app, app.runningHostedHands(of: threadID) == 0 else {
            throw ProviderError.failed("Stop the running agents before undoing file changes.")
        }
        let stretch = try revertStretch(scope, paths: paths)
        isReverting = true
        defer { isReverting = false }
        let result: RevertPreview
        do {
            result = try await stretch.git.revertFiles(base: stretch.base, end: stretch.end, paths: stretch.paths, only: stretch.only)
        } catch {
            throw await Self.checkpointError(error, stretch)
        }
        recordRevert(result, stretch: stretch, scope: scope)
        return result
    }

    /// Marks each turn of the stretch with the files it has had undone, and says what
    /// happened in the timeline and in the session log.
    private func recordRevert(_ result: RevertPreview, stretch: RevertStretch, scope: RevertScope) {
        // What changed on disk, what was found already back, and what was left alone.
        let undone = result.restorable.map(\.path)
        let settled = result.files.filter { $0.outcome != .conflict }.map(\.path)
        let conflicted = result.conflicted.map(\.path)
        for turn in stretch.turns {
            // The turn's paths as it recorded them, against git's names for the files.
            func recorded(_ path: String) -> Bool { (turn.touchedPaths ?? []).contains { result.names(path, as: $0) } }
            let mine = settled.filter(recorded)
            let stuck = conflicted.filter(recorded)
            if !mine.isEmpty {
                updateTurn(turn.id) { record in
                    record.revertedPaths = Array(Set(record.revertedPaths ?? []).union(mine)).sorted()
                }
                revertedPaths[turn.id, default: []].formUnion(mine)
                revertConflicts[turn.id]?.subtract(mine)
            }
            if !stuck.isEmpty { revertConflicts[turn.id, default: []].formUnion(stuck) }
        }
        let which = Self.describe(scope, stretch: stretch)
        var lines: [String] = []
        if !undone.isEmpty { lines.append("Undid \(which)'s changes to \(Self.list(undone)).") }
        if !conflicted.isEmpty { lines.append("Left \(Self.list(conflicted)) as \(conflicted.count == 1 ? "it is" : "they are"): changed again since in the same lines, so \(which)'s changes there could not be taken out alone.") }
        guard !lines.isEmpty else { return }
        appendNotice(conflicted.isEmpty ? .info : .warning, lines.joined(separator: " "))
        sessionLogPending += SessionLog.append(lines, to: threadID)
        saveRevision += 1
        // The files changed on disk: every diff and preview of them is stale.
        diffRevision += 1
        scheduleSave()
    }

    private static func describe(_ scope: RevertScope, stretch: RevertStretch) -> String {
        switch scope {
        case .thread: return "this thread"
        case .turn, .from:
            guard let first = stretch.turns.first?.index, let last = stretch.turns.last?.index else { return "this thread" }
            return first == last ? "turn \(first + 1)" : "turns \(first + 1)–\(last + 1)"
        }
    }

    private static func list(_ paths: [String]) -> String {
        let shown = paths.prefix(8).joined(separator: ", ")
        return paths.count > 8 ? shown + " and \(paths.count - 8) more" : shown
    }

    /// Edits a sent message: the edited words go out as a new turn at the end of the
    /// conversation, which is kept as it is. With `undoFiles`, the file changes of the edited
    /// message's turn and every turn after it are undone first (see `revertFiles`); a file
    /// left alone there is said so in the thread, and the edit goes out all the same.
    /// Whatever was in the composer stays there.
    func resend(_ turnID: UUID, text: String, attachments: [Attachment], undoFiles: Bool) async throws {
        if undoFiles { try await revertFiles(.from(turnID)) }
        let kept = draft
        draft = ComposerDraft(text: text, attachments: attachments)
        send()
        draft = kept
    }

    /// Lets go of the file checkpoints this thread no longer keeps, as each turn finishes.
    /// A turn leaves two hidden refs and the snapshot commits behind them, and the threads
    /// of a long-lived repository would otherwise pile up thousands, slowing every
    /// `for-each-ref` and `gc` there. An archived thread keeps none. Otherwise the last
    /// `checkpointKeepTurns` turns keep theirs (Settings › Source control), and two older
    /// starts stay whatever the window: the thread's first, which the whole thread's diff
    /// and a revert to its first message are taken from, and that of the first turn no
    /// Crew merge has taken, which the merge's checkout sweep reads. A turn that lost its
    /// checkpoints reads as having none: its files can no longer be put back, and its
    /// diff comes from the edits its tools landed.
    private func pruneCheckpoints() {
        guard let app, let thread, let project = app.project(thread.projectID) else { return }
        // Refs live in the repository every worktree of it shares; the project's own
        // checkout stands when a hand's copy is long gone.
        let git = Git(project.path)
        let threadID = threadID
        if thread.isArchived {
            // The archive let them go already (see `AppModel.archive`); what the turn it
            // stopped wrote since goes too. The records keep their names: every reader
            // takes a missing ref for none, and the whole thread's diff then comes from
            // what the tools reported rather than from a later turn's snapshot alone.
            Task.detached(priority: .utility) { await git.deleteCheckpoints(thread: threadID) }
            return
        }
        guard let kept = app.settings.checkpointKeepTurns, let newest = turns.map(\.index).max() else { return }
        let firstKept = newest - kept + 1
        let preserved = Set([
            turns.first(where: { $0.baseCheckpoint != nil })?.baseCheckpoint,
            crewUnmergedTurns.first?.baseCheckpoint,
        ].compactMap { $0 })
        var updated = turns
        var changed = false
        for index in updated.indices where updated[index].index < firstKept {
            if let base = updated[index].baseCheckpoint, !preserved.contains(base) {
                updated[index].baseCheckpoint = nil
                changed = true
            }
            if updated[index].endCheckpoint != nil {
                updated[index].endCheckpoint = nil
                changed = true
            }
        }
        // Nothing outside the window holds one any more, so no git runs.
        guard changed else { return }
        // One write of `turns`: every write re-evaluates the timeline body.
        turns = updated
        saveRevision += 1
        scheduleSave()
        Task.detached(priority: .utility) {
            await git.pruneCheckpoints(thread: threadID, keepingTurnsFrom: firstKept, preserving: preserved)
        }
    }

    /// Parsed patches shared by the diff panel and the historical file
    /// cards. A finished turn's diff is fixed by its checkpoints and the edits that landed, so it is
    /// kept by turn for the runtime's life (a load replacing the turns takes their entries);
    /// the whole thread's and a running turn's move with the revision and go with it.
    /// Concurrent callers share one run.
    @ObservationIgnored private var turnDiffs = RecentCache<UUID, Task<[DiffFile], Never>>(limit: 64)
    @ObservationIgnored private var revisionDiffs = RecentCache<String, Task<[DiffFile], Never>>(limit: 4)
    @ObservationIgnored private var repositoryRootLookup: (directory: URL, id: UUID, task: Task<URL, Never>)?

    private func repositoryRoot(for git: Git) -> Task<URL, Never> {
        if let lookup = repositoryRootLookup, lookup.directory == git.directory { return lookup.task }
        let id = UUID()
        let task = Task { [weak self] in
            var repository = git.directory
            if let prefix = try? await git.output(["rev-parse", "--show-prefix"]) {
                for _ in prefix.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: "/") {
                    repository.deleteLastPathComponent()
                }
            }
            if let self, repositoryRootLookup?.id == id { repositoryRootLookup = nil }
            return repository
        }
        repositoryRootLookup = (git.directory, id, task)
        return task
    }

    /// A finished turn's diff outlives revisions; anything else is keyed by the revision.
    private func fixedTurn(_ selection: UUID?) -> UUID? {
        guard let selection, let turn = turns.first(where: { $0.id == selection }), turn.status != .running else { return nil }
        return selection
    }

    private func cachedDiff(selection: UUID?) -> Task<[DiffFile], Never>? {
        if let turn = fixedTurn(selection) { return turnDiffs.value(for: turn) }
        return revisionDiffs.value(for: selection?.uuidString ?? "all")
    }

    private func cacheDiff(_ task: Task<[DiffFile], Never>, selection: UUID?) {
        if let turn = fixedTurn(selection) {
            turnDiffs.insert(task, for: turn)
        } else {
            revisionDiffs.insert(task, for: selection?.uuidString ?? "all")
        }
    }

    func hasCachedDiff(selection: UUID?) -> Bool {
        cachedDiff(selection: selection) != nil
    }

    func turnsWithChanges() -> [TurnRecord] {
        var edited: Set<UUID> = []
        for entry in entries {
            guard let id = entry.turnID, case .tool(let call) = entry.item.content, call.hasLandedEdits else { continue }
            edited.insert(id)
        }
        return turns.filter { $0.endCheckpoint != nil || $0.providerDiff != nil || $0.touchedPaths?.isEmpty == false || edited.contains($0.id) }
    }

    func parsedDiff(selection: UUID?, providerFiles: [DiffFile]? = nil) async -> [DiffFile] {
        if let task = cachedDiff(selection: selection) { return await task.value }
        guard let app, let thread = app.thread(threadID), let project = app.project(thread.projectID) else { return [] }
        let root = thread.worktreePath ?? project.path
        let git = Git(root)
        let repository = repositoryRoot(for: git)
        let selectedTurns = selection.map { id in turns.filter { $0.id == id } } ?? turns
        guard !selectedTurns.isEmpty else { return [] }
        let selectedIDs = Set(selectedTurns.map(\.id))
        // The agent's writes to its own notes and settings, a scratch file in /tmp, a
        // stray write into a hand's copy: none of it is the chat's work, and a memory
        // note it saved showed on the turn's card as "Edited 1 file" in a chat that had
        // changed nothing. A file in another sidebar project is work, and stays.
        let projectRoots = app.projects.map(\.path)
        let edits = entries.flatMap { entry -> [FileEdit] in
            guard let id = entry.turnID, selectedIDs.contains(id), case .tool(let call) = entry.item.content else { return [] }
            return call.landedEdits.filter { Self.isProjectFile($0.path, root: root, projects: projectRoots) }
        }
        // Only the files this chat's turns recorded as changed count. A turn without a record (one cut off
        // by a crash) contributes nothing rather than lifting the filter: the snapshot spans the
        // whole checkout and would otherwise show every merge and other thread's work since the
        // chat began.
        let touched = Set(selectedTurns.flatMap { $0.touchedPaths ?? [] }.map { TouchedPaths.normalize($0, root: root) })
        var span: (first: String, last: String)?
        if let ended = selectedTurns.lastIndex(where: { $0.endCheckpoint != nil }),
           let first = selectedTurns[...ended].first(where: { $0.baseCheckpoint != nil })?.baseCheckpoint,
           let last = selectedTurns[ended].endCheckpoint {
            span = (first, last)
        }
        let task = Task { () -> [DiffFile] in
            var snapshot: String?
            if let span { snapshot = try? await git.diff(from: span.first, to: span.last) }
            let providerPatch = providerFiles == nil ? selectedTurns.compactMap(\.providerDiff).joined(separator: "\n") : ""
            return await Self.resolveDiff(repositoryRoot: repository.value.path, snapshot: snapshot, providerPatch: providerPatch, providerFiles: providerFiles, edits: edits, touched: touched, root: root)
        }
        cacheDiff(task, selection: selection)
        return await task.value
    }

    /// Whether a path an agent reported editing lies in the chat's checkout or in another
    /// sidebar project: the files a turn's card and diff can speak for.
    private static func isProjectFile(_ path: String, root: String, projects: [String]) -> Bool {
        TouchedPaths.relative(path, root: root) != nil || projects.contains { TouchedPaths.relative(path, root: $0) != nil }
    }

    @concurrent
    private nonisolated static func resolveDiff(repositoryRoot: String, snapshot: String?, providerPatch: String, providerFiles: [DiffFile]?, edits: [FileEdit], touched: Set<String>, root: String) async -> [DiffFile] {
        TurnDiff.merge(snapshot: snapshot.map(DiffParser.parse), providerPatch: providerPatch, edits: edits, touched: touched, root: root, repositoryRoot: repositoryRoot, providerFiles: providerFiles)
    }

    @concurrent
    private nonisolated static func parseDiff(_ patch: String) async -> [DiffFile] {
        DiffParser.parse(patch)
    }

    /// Only the session observed last is listened to: one observed before it may still
    /// speak as it closes, and nothing it says reaches the thread.
    func observe(_ provider: any ProviderSession) {
        sessionEpoch += 1
        let epoch = sessionEpoch
        provider.onEvent = { [weak self] event in
            guard let self, self.sessionEpoch == epoch else { return }
            self.handle(event)
        }
    }

    /// Lets a session go: from here on it drives nothing, whatever it still has to say as
    /// it closes. Its handler is cleared a turn of the loop later, because a session that
    /// is ending usually says so from inside that very handler.
    private func releaseSession(stop: Bool) {
        sessionEpoch += 1
        sessionSignature = nil
        unsentSessionID = nil
        // A new session has a new history: whatever the fallback prefix did in the old
        // one, it has to go out again, and a forwarded context needs a new session to
        // land in. Reset before any early return, so a session that was never made still
        // leaves the next one to deliver both.
        antiSlopFallbackSent = false
        continuationSent = false
        guard let old = session else { return }
        session = nil
        if stop { old.stop() }
        Task { old.onEvent = nil }
    }

    func stopSession() {
        releaseSession(stop: true)
        stopNativeHands()
        // The session's own end is dropped with it (see `releaseSession`), so a turn cut
        // off here is closed by hand: left running, the thread showed the working line
        // for good once reopened, the send queued instead of sending, a hand's lead heard
        // nothing until the budget fired. `finishTurn` saves what the turn had.
        guard phase != .idle else { return }
        // A turn cut off here has events its save timer has not written yet, and the timer
        // may not outlive the runtime.
        saveNow()
        // Nothing is left to end the turn, so it ends here, as interrupted. It leaves
        // `currentTurnID` first, the way a stop during a start does, so a message still on
        // its way to the session never goes.
        let turnID = currentTurnID
        currentTurnID = nil
        Task { await finishTurn(status: .interrupted, turnID: turnID) }
    }

    // MARK: - Events

    private func handle(_ event: ProviderEvent) {
        switch event {
        case .sessionReady(let sessionID):
            recordSessionID(sessionID)
        case .turnStarted(let providerTurnID):
            phase = .running
            if let currentTurnID, let providerTurnID {
                updateTurn(currentTurnID) { $0.providerTurnID = providerTurnID }
            }
        case .messageDelta(let id, let text):
            queueDelta(id, .message, text)
        case .messageCompleted(let id, let text):
            flushDeltas()
            completeMessage(id, text: text)
        case .reasoningDelta(let id, let text):
            queueDelta(id, .reasoning, text)
        case .reasoningCompleted(let id, let text):
            flushDeltas()
            completeReasoning(id, text: text)
        case .toolStarted(let id, let call):
            flushDeltas()
            upsertTool(id, call)
            if call.kind == .command { watchCommand(id, call) }
        case .toolOutput(let id, let text):
            queueDelta(id, .toolOutput, text)
        case .toolUpdated(let id, let update):
            flushDeltas()
            applyToolUpdate(id, update)
            if let entry = entryIndex[id], case .tool(let call) = entry.item.content,
               call.kind == .command, call.status != .running {
                settleCommand(id)
            }
            if let status = update.status, status != .running, let handID = crewToolHands[id] {
                crewToolFinished(handID, status: status, output: update.output)
            }
        case .planDelta(let id, let text):
            queueDelta(id, .plan, text)
        case .planCompleted(let id, let markdown):
            flushDeltas()
            completePlan(id, markdown: markdown)
        case .todos(let steps):
            upsertTodos(steps)
        case .approval(let request):
            // A hand stopped on a person is quiet from the moment it asks, not from its last
            // tool, so the watchdog's wait is counted from here. Only a new request starts
            // it: one a provider sends again does not put the clock back.
            if !approvals.contains(where: { $0.id == request.id }) { noteCrewHandEvent() }
            approvals.removeAll { $0.id == request.id }
            approvals.append(request)
            app?.threadNeedsAttention(threadID)
        case .question(let request):
            if !questions.contains(where: { $0.id == request.id }) { noteCrewHandEvent() }
            questions.removeAll { $0.id == request.id }
            questions.append(request)
            app?.threadNeedsAttention(threadID)
        case .requestResolved(let id):
            approvals.removeAll { $0.id == id }
            questions.removeAll { $0.id == id }
        case .usage(let usage):
            self.usage = usage
            saveRevision += 1
            // The turn just spent from the account, so the balance read before it is stale.
            if let provider = thread?.provider {
                app?.providers.invalidateCredits(provider)
                app?.providers.invalidatePlanLimits(provider)
            }
        case .tokenSpend(let spend):
            recordTokenSpend(spend)
        case .diff(let diff):
            if currentTurnID != nil { currentProviderDiff = diff }
        case .notice(let notice):
            appendNotice(notice.level, notice.message)
        case .usageLimit(let resetsAt):
            app?.autoContinue.noteLimit(threadID, resetsAt: resetsAt)
        case .modeChanged(let mode):
            app?.updateThread(threadID) { $0.interactionMode = mode }
        case .models(let list, _):
            if let provider = thread?.provider { app?.providers.updateCatalog(list, for: provider) }
        case .commands(let list):
            if let app, let thread, let project = app.project(thread.projectID) {
                app.providers.updateCommands(list, for: thread.provider, directory: thread.worktreePath ?? project.path)
            }
        case .title(let title):
            // Provider titles are only a fallback; they must not replace the one Bottega writes.
            guard turns.count <= 1, let app, let thread, !thread.hasCustomTitle,
                  app.textEngine(preferring: thread.provider) == nil else { break }
            app.updateThread(threadID) { $0.title = TextCleanup.withoutEmDashes(title) }
        case .assistantMessageID(let anchor):
            if currentTurnID != nil { currentProviderAnchor = anchor }
        case .turnCompleted(let status, let error):
            flushDeltas()
            if let error { appendNotice(.error, error) }
            let turnID = currentTurnID
            Task { await finishTurn(status: status, turnID: turnID) }
        case .exited(let error):
            flushDeltas()
            // A session that exits while it starts is reported by its `start()` throwing:
            // `ensureSession` starts over where it can, and otherwise the turn ends with
            // that error. Letting the session go here broke the start-over, which needs
            // the session it replaces still in place, and failed the turn before it.
            if isStartingSession { break }
            releaseSession(stop: false)
            approvals.removeAll()
            questions.removeAll()
            stopNativeHands()
            if currentTurnID != nil {
                let name = thread?.provider.displayName ?? "The agent"
                appendNotice(.error, error ?? "\(name) stopped unexpectedly.")
                let turnID = currentTurnID
                Task { await finishTurn(status: .failed, turnID: turnID) }
            }
        case .agentStarted(let spawn):
            crewAgentStarted(spawn)
        case .agentEvent(let agentID, let event):
            crewAgentEvent(agentID, event)
        case .agentProgress(let agentID, let summary, let lastTool, let tokens, let toolCalls):
            guard let handID = crewNativeHands[agentID], let handRuntime = app?.runtime(for: handID) else { break }
            if let summary, !summary.isEmpty { handRuntime.crewActivity = summary } else if let lastTool { handRuntime.crewActivity = lastTool }
            if let tokens { handRuntime.crewTokens = tokens }
            if let toolCalls { handRuntime.crewToolCalls = toolCalls }
            handRuntime.crewLastEventAt = .now
        case .agentFinished(let agentID, let status, let summary):
            crewAgentFinished(agentID, status: status, summary: summary)
        }
    }

    /// Closes the running turn. `turnID` names the turn when the caller has already taken
    /// it out of `currentTurnID`, as a stop during the start of a turn does, so that the
    /// message on its way can no longer be sent.
    private func finishTurn(status: TurnStatus, turnID explicitTurnID: UUID? = nil) async {
        interruptWatchdog?.cancel()
        interruptWatchdog = nil
        handBudget?.cancel()
        handBudget = nil
        guard let turnID = explicitTurnID ?? currentTurnID, let turn = turns.first(where: { $0.id == turnID }) else {
            phase = .idle
            turnStartedAt = nil
            drainPendingSend()
            return
        }
        // Claimed before the first suspension below: a completion and an exit can both
        // arrive for one turn, and the second finalizer finds it taken.
        guard finishingTurnID != turnID else { return }
        finishingTurnID = turnID
        defer { finishingTurnID = nil }
        // Command diffs land before the turn's own summary counts them, and
        // while the turn is still current so the snapshots can be compared. The hands'
        // commands settle with them: their files are part of this turn's work.
        for id in Array(commandTrees.keys) { settleCommand(id) }
        for key in Array(crewCommandTrees.keys) { settleCrewCommand(key) }
        // A completion queued behind a stop can arrive after the next turn has begun; the
        // turn it names is over, and the one running is left alone.
        guard currentTurnID == nil || currentTurnID == turnID else { return }
        // A finished turn waits for its diffs; a stopped or failed one gives them three
        // seconds and lets the rest land on their own, so Stop never hangs on git.
        closingTurnID = turnID
        defer { closingTurnID = nil }
        let settles = Array(commandSettles.values)
        commandSettles.removeAll()
        if status == .completed {
            for task in settles { await task.value }
        } else {
            await withTaskGroup(of: Void.self) { group in
                group.addTask { for task in settles { await task.value } }
                group.addTask { try? await Task.sleep(for: .seconds(3)) }
                await group.next()
                group.cancelAll()
            }
        }
        currentTurnID = nil
        flushDeltas()
        approvals.removeAll()
        questions.removeAll()
        // One pass over the turn's rows: they are closed, and the files their tools
        // changed are collected for the summary below rather than scanned for again. Only
        // edits that landed count (see `ToolCall.landedEdits`): a write still waiting on
        // an approval, a declined one and a failed one are on their rows, but changed nothing.
        var reportedEdits: [FileEdit] = []
        for entry in entries where entry.item.turnID == turnID {
            endStreaming(entry)
            guard case .tool(var call) = entry.item.content else { continue }
            if call.status == .running {
                call.finish(status == .completed ? .completed : .failed)
                entry.item.content = .tool(call)
                saveRevision += 1
            }
            reportedEdits.append(contentsOf: call.landedEdits)
        }
        // A native hand's tools are noted for the lead as their rows finish (see
        // `crewAgentEvent`). A row this close-out finished for them, a tool whose end the
        // provider never sent, is noted here, so the lead's turn counts it as it does the
        // lead's own rows closed above.
        if let info = thread?.crew, info.kind == .native, let parentID = thread?.parentThreadID {
            app?.existingRuntime(for: parentID)?.noteCrewEdits(reportedEdits)
        }
        let providerDiff = currentProviderDiff
        let providerAnchor = currentProviderAnchor
        currentProviderDiff = nil
        currentProviderAnchor = nil
        updateTurn(turnID) {
            $0.status = status
            $0.completedAt = .now
            if let providerDiff { $0.providerDiff = providerDiff }
            if let providerAnchor { $0.providerAnchor = providerAnchor }
        }
        if let providerAnchor, thread?.providerResumeAt != nil {
            app?.updateThread(threadID) { $0.providerResumeAt = providerAnchor }
        }

        let providerPatch = providerDiff ?? turns.first(where: { $0.id == turnID })?.providerDiff ?? ""
        // Parsed off the main actor, and only once: a turn's patch can be large, and
        // this lands exactly as the reply appears.
        let providerFiles = providerPatch.isEmpty ? [] : await Self.parseDiff(providerPatch)
        if let app, let thread = app.thread(threadID), let project = app.project(thread.projectID) {
            let root = thread.worktreePath ?? project.path
            let git = Git(root)
            var endCheckpoint: String?
            // An archived thread keeps no checkpoints (see `pruneCheckpoints`): a turn
            // stopped by the archive leaves no end behind it.
            if turn.baseCheckpoint != nil, !thread.isArchived {
                let ref = Git.checkpointRef(thread: threadID, turn: turn.index, phase: "end")
                if (try? await git.captureCheckpoint(ref)) != nil { endCheckpoint = ref }
            }
            // Only files this thread's agent edited count, never edits made elsewhere in
            // the repository meanwhile. A path outside the checkout is no part of the turn:
            // the agent writes to its own notes and settings too, and git cannot stage a
            // file it does not hold.
            var touched = Set<String>()
            for edit in reportedEdits {
                if let path = TouchedPaths.relative(edit.path, root: root) { touched.insert(path) }
            }
            // The provider's own repository diff is no part of a turn's record, because
            // it spans every file that changed in the checkout while the turn ran, another
            // chat's work included, and one such file was once merged out under a chat that
            // had changed nothing; a change of the turn that no edit reported is caught by
            // the merge's checkout sweep instead.
            // What the lead's native hands changed, which their own timelines carry: this
            // turn takes it, whether it came during the turn or between the last and this.
            touched.formUnion(crewTouched)
            crewTouched.removeAll()
            let touchedPaths = touched.sorted()
            // One write of `turns`: every write re-evaluates the timeline body.
            updateTurn(turnID) {
                if let endCheckpoint { $0.endCheckpoint = endCheckpoint }
                $0.touchedPaths = touchedPaths
            }
        }
        diffRevision += 1
        let changes = FileChangeSummary(files: await parsedDiff(selection: turnID, providerFiles: providerFiles))

        let summary = TurnSummary(
            turnID: turnID,
            status: status,
            duration: Date.now.timeIntervalSince(turn.startedAt),
            filesChanged: changes.files.count,
            additions: changes.additions,
            deletions: changes.deletions,
            changes: changes
        )
        append(TimelineItem(turnID: turnID, content: .turnEnd(summary)))
        phase = .idle
        turnStartedAt = nil
        // The turn's own events shared one save every few seconds; whatever they left is
        // written here, so a finished turn is always on disk.
        saveNow()
        pruneCheckpoints()
        settleForegroundHands(after: status)
        guard thread?.isArchived != true else {
            handBudgetSpent = false
            app?.turnFinished(threadID, status: status, continues: false)
            return
        }
        // The message sent while the turn ran jumps the queue: it goes right away
        // and anything queued waits for it. A reply that ends in a delegation block
        // sends the hands out instead, and their reports come back as a message of
        // their own; the lead is free meanwhile, so the next queued prompt starts on
        // it at once rather than waiting out the hands, and the reports follow that
        // turn. Either way the app hears whether a next turn is on its way, so
        // "finished" only sounds when nothing is.
        var continues: Bool
        if pendingSend != nil {
            continues = drainPendingSend()
        } else if status == .completed, case let delegation = spawnDelegatedHands(for: turnID), delegation != .none {
            continues = true
            if delegation == .handsOut { _ = drainFollowUps(after: status) }
        } else if status == .completed, flushCrewReports() {
            continues = true
        } else {
            // A turn that failed or was stopped with a block in its reply: the block leaves
            // the reply and a note says no hands went out, rather than the raw block
            // sitting there as if something were coming.
            if status != .completed { dropDelegationBlock(for: turnID, status: status) }
            continues = drainFollowUps(after: status)
        }
        // Reports that came in while this turn ran are heard whatever became of it. Only a
        // turn the user stopped is left alone, as their queued messages are: they are in
        // charge of the chat then, and the reports go out with their next word.
        if !continues, status != .interrupted, flushCrewReports() { continues = true }
        // Hands still out will report, and the lead will work again: the job is not
        // finished until it has heard from all of them.
        if !continues, status == .completed, let app, app.runningHostedHands(of: threadID) > 0 { continues = true }
        // A hand stopped for its budget writes its report as its next turn, so its lead
        // hears what it managed rather than only that it was stopped.
        if handBudgetSpent {
            handBudgetSpent = false
            if status == .interrupted, thread?.crew?.kind == .hosted {
                handReportsNext = true
                // This turn starts on its own (see autoStartedTurnToken).
                autoStartedTurnToken &+= 1
                Task { await ensureLoaded(); await startTurn(text: CrewBudget.finalNote, attachments: []) }
                continues = true
            }
        }
        app?.turnFinished(threadID, status: status, continues: continues)
        // The job is done: the lead has answered, every hand is back and nothing is
        // waiting. With the setting on, the work goes out and lands by itself, whether
        // hands took part or the lead did it all; a job that changed no file is let be.
        // Native hands are not in the batches: they run on inside the lead's session after
        // the lead's reply ends, and report in a turn of their own later. A merge while any
        // hand is still at work would commit its half-done files and then, the hand having
        // finished before the next turn began, never come back for the rest.
        if !continues, status == .completed,
           crewPendingReports.isEmpty, crewBatches.isEmpty, crewWaiting.isEmpty,
           let app, let thread, app.crewIsOn(thread), app.settings.crewAutoMerge,
           app.runningCrewHands(of: threadID) == 0 {
            Task { await app.autoMergeCrewWork(of: threadID) }
        }
    }

    /// Sends the message captured by Command-Return while a turn ran, once the stop
    /// lands. Runs for any finish status: if the turn completed on its own in
    /// the meantime, the message still goes right away. Returns whether a turn starts.
    @discardableResult
    private func drainPendingSend() -> Bool {
        guard phase == .idle, let pending = pendingSend else { return false }
        pendingSend = nil
        let text = pending.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !pending.attachments.isEmpty else {
            scheduleSave()
            return false
        }
        if handleLocalCommand(text) { return false }
        Task { await ensureLoaded(); await startTurn(text: pending.text, attachments: pending.attachments) }
        return true
    }

    // MARK: - Crew

    /// The auto-merge has landed these turns' work: the next merge starts from what came
    /// after them, rather than offering the same files again.
    func markCrewMerged(_ turnIDs: [UUID]) {
        let ids = Set(turnIDs)
        var changed = false
        for index in turns.indices where ids.contains(turns[index].id) && !turns[index].crewMerged {
            turns[index].crewMerged = true
            changed = true
        }
        guard changed else { return }
        saveRevision += 1
        saveNow()
    }

    /// The turns whose work has not been merged yet, oldest first.
    var crewUnmergedTurns: [TurnRecord] {
        turns.filter { !$0.crewMerged }
    }

    /// The auto-merge's latest outcome for this thread, kept for the lead's next message.
    func recordCrewMerge(_ record: CrewMergeRecord) {
        crewMerges.append(record)
        if crewMerges.count > 20 { crewMerges.removeFirst(crewMerges.count - 20) }
        saveRevision += 1
        saveNow()
    }

    /// The hand a tool row in this timeline sent out, if any.
    func crewHand(forTool toolID: String) -> UUID? {
        crewToolHands[toolID]
    }

    /// A hand the provider started inside this session, or more said about one already
    /// known: a name for the tool call that spawned it, its brief.
    private func crewAgentStarted(_ spawn: AgentSpawn) {
        guard let app, let thread, app.crewIsOn(thread) else { return }
        if let handID = crewNativeHands[spawn.id] {
            let named = !spawn.description.isEmpty && spawn.description != "Subagent"
            app.updateThread(handID) { hand in
                guard var info = hand.crew else { return }
                if named, info.task != spawn.description {
                    info.task = spawn.description
                    hand.title = "\(info.persona.name) · \(TextCleanup.singleLine(spawn.description, limit: 60))"
                }
                if info.toolUseID == nil { info.toolUseID = spawn.toolUseID }
                hand.crew = info
            }
            if let toolUseID = spawn.toolUseID { crewToolHands[toolUseID] = handID }
            if let prompt = spawn.prompt, let handRuntime = app.existingRuntime(for: handID), !handRuntime.hasSentPrompts {
                handRuntime.rehearseBrief(prompt)
            }
            return
        }
        guard let hand = app.spawnNativeHand(from: threadID, spawn: spawn) else { return }
        crewNativeHands[spawn.id] = hand.id
        if let toolUseID = spawn.toolUseID { crewToolHands[toolUseID] = hand.id }
    }

    /// An event from inside a hand's transcript, rehearsed on the hand's own timeline.
    private func crewAgentEvent(_ agentID: String, _ event: ProviderEvent) {
        guard let app else { return }
        // A hand heard from before it was announced: on Claude the agent id is the tool
        // call that spawned it, so the row already names its task.
        if crewNativeHands[agentID] == nil, let entry = entryIndex[agentID], case .tool(let call) = entry.item.content, call.kind == .agent {
            crewAgentStarted(AgentSpawn(id: agentID, taskID: nil, toolUseID: agentID, description: call.title, prompt: nil, model: nil, isBackground: false))
        }
        guard let handID = crewNativeHands[agentID], let handRuntime = app.existingRuntime(for: handID) else { return }
        switch event {
        case .turnStarted:
            // A hand briefed again by its lead works another turn.
            if !handRuntime.isRunning {
                handRuntime.rehearseTurn(nil)
                app.updateCrewHand(handID) {
                    $0.status = .running
                    $0.finishedAt = nil
                }
            }
        case .turnCompleted(let status, _):
            crewAgentFinished(agentID, status: status, summary: nil)
        case .toolStarted(let toolID, let call):
            handRuntime.crewToolCalls += 1
            handRuntime.crewActivity = ToolPresentation.label(for: call)
            handRuntime.crewLastEventAt = .now
            if call.kind == .command { watchCrewCommand("\(agentID)-\(toolID)", call) }
            handRuntime.rehearse(event)
            // The edits a tool row lists are what it was asked to write, and the row has
            // them from the moment it appears: they count for this turn once the hand's
            // own timeline says the tool landed them, not when the call was proposed.
            noteCrewEdits(handRuntime.landedEdits(ofTool: toolID))
        case .toolUpdated(let toolID, let update):
            // A declined command never ran: the tree is still read, so the next command
            // starts from it, but what changed meanwhile is not its work.
            if let status = update.status, status != .running {
                settleCrewCommand("\(agentID)-\(toolID)", attributesChanges: status != .declined)
            }
            handRuntime.rehearse(event)
            noteCrewEdits(handRuntime.landedEdits(ofTool: toolID))
        case .usage, .sessionReady, .models, .commands, .title, .assistantMessageID:
            break
        default:
            handRuntime.rehearse(event)
        }
    }

    /// A hand finished: its timeline closes and its status lands, and the tool row that
    /// sent it out completes if the provider left it running in the background.
    private func crewAgentFinished(_ agentID: String, status: TurnStatus, summary: String?) {
        guard let app, let handID = crewNativeHands[agentID] else { return }
        app.finishCrewHand(handID, status: status, summary: summary)
        if let handRuntime = app.existingRuntime(for: handID), handRuntime.isRunning {
            handRuntime.rehearse(.turnCompleted(status: status, error: nil))
        }
    }

    /// The tool call that sent out a foreground hand has returned: the hand is done, and
    /// the result is its report. A background hand's call returns at once and says
    /// nothing about the hand, which the provider ends on its own.
    private func crewToolFinished(_ handID: UUID, status: ToolCall.Status, output: String?) {
        guard let app, let hand = app.thread(handID), let info = hand.crew, info.kind == .native, !info.isBackground, !info.isFinished,
              let nativeID = info.nativeID else { return }
        let outcome: TurnStatus = switch status {
        case .completed: .completed
        case .declined: .interrupted
        default: .failed
        }
        crewAgentFinished(nativeID, status: outcome, summary: output)
    }

    /// The lead's turn is over, so any foreground hand still marked as working went with
    /// it: a stopped turn stopped them, a finished one finished them. Background hands
    /// outlive the turn and end on the provider's word.
    private func settleForegroundHands(after status: TurnStatus) {
        guard let app else { return }
        for (nativeID, handID) in crewNativeHands {
            guard let hand = app.thread(handID), let info = hand.crew, !info.isBackground, !info.isFinished else { continue }
            crewAgentFinished(nativeID, status: status == .completed ? .completed : .interrupted, summary: nil)
        }
    }

    /// The session the hands ran in has stopped: whichever were still running are done.
    private func stopNativeHands() {
        guard let app else { return }
        for handID in crewNativeHands.values {
            guard let hand = app.thread(handID), hand.crew?.isFinished == false else { continue }
            app.finishCrewHand(handID, status: .interrupted, summary: nil)
            if let handRuntime = app.existingRuntime(for: handID), handRuntime.isRunning {
                handRuntime.rehearse(.turnCompleted(status: .interrupted, error: nil))
            }
        }
        crewNativeHands.removeAll()
    }

    /// Asks the session to stop a native hand. Returns whether it could.
    func stopNativeHand(_ nativeID: String) async -> Bool {
        guard let session else { return false }
        return await session.stopAgent(nativeID)
    }

    /// The app finished a hand of this lead's (see `AppModel.finishCrewHand`): a native
    /// hand's tool row completes; a hosted hand's report goes to the lead, once its
    /// batch is complete. A hand reporting again (steered on from the panel) replaces
    /// any report of its own still waiting.
    func crewHandFinished(_ handID: UUID, info: CrewHandInfo, status: CrewHandInfo.Status, summary: String?, landing: CrewLanding? = nil, copyPath: String? = nil) {
        switch info.kind {
        case .native:
            guard let toolID = info.toolUseID, let entry = entryIndex[toolID], case .tool(let call) = entry.item.content, call.status == .running else { return }
            let outcome: ToolCall.Status = status == .completed ? .completed : .failed
            applyToolUpdate(toolID, ToolUpdate(output: summary, status: outcome))
            // A hand back with the lead idle and a prompt still queued (steered
            // behind the team while the lead sat idle) lets the lead take it now
            // rather than holding it until some later turn ends.
            flushFollowUpsIfIdle()
        case .hosted:
            // How long the hand worked: from its brief to the end of the turn it reports on.
            // Making its copy before and landing its work after are not its work; a hand
            // stopped before its brief went out has only the time since it was sent.
            let elapsed = info.executionStartedAt.map { (info.executionFinishedAt ?? .now).timeIntervalSince($0) }
                ?? Date.now.timeIntervalSince(info.startedAt)
            let report = CrewReport(
                handIndex: info.index, task: info.task, origin: info.origin, status: status, text: summary ?? "",
                landing: landing, copyPath: copyPath, elapsed: elapsed, toolCalls: info.toolCalls,
                reportPath: info.reportPath
            )
            crewPendingReports.removeAll { $0.handIndex == info.index }
            if let batchID = info.batchID, crewBatches[batchID] != nil {
                crewBatches[batchID]?.pending.remove(handID)
                crewBatches[batchID]?.reports.removeAll { $0.handIndex == info.index }
                crewBatches[batchID]?.reports.append(report)
            } else {
                crewPendingReports.append(report)
            }
            // A finished hand makes room: for a task still waiting in a batch first, then
            // for whatever the user queued while every hand was busy.
            spawnWaitingHands()
            dispatchQueuedFollowUps()
            settleBatches()
            // An idle lead hears at once; one the user stopped waits for their next word.
            if phase == .idle, thread?.lastStatus != .interrupted { flushCrewReports() }
            // After the reports: whatever is still queued behind an idle lead goes
            // next, rather than waiting out the hands still at work.
            flushFollowUpsIfIdle()
        }
    }

    /// A batch with every hand back and nothing left to send out hands its reports over.
    private func settleBatches() {
        for (batchID, batch) in crewBatches where batch.pending.isEmpty && !crewWaiting.contains(where: { $0.batchID == batchID }) {
            crewBatches[batchID] = nil
            crewPendingReports += batch.reports.sorted { $0.handIndex < $1.handIndex }
        }
    }

    /// Sends the reports waiting on an idle lead as one message. Returns whether a turn
    /// starts. The reports stay put until the turn takes them: hands finishing together
    /// then go out as one message, instead of each starting a turn the next one cuts short.
    @discardableResult
    private func flushCrewReports() -> Bool {
        guard phase == .idle, !crewFlushScheduled, !flushableCrewReports().isEmpty else { return false }
        crewFlushScheduled = true
        // This turn starts on its own (see autoStartedTurnToken).
        autoStartedTurnToken &+= 1
        Task { await startCrewReportTurn() }
        return true
    }

    /// The reports an idle lead can hear now: every one waiting. A delegation's reports
    /// only get here once its whole batch is back (see `settleBatches`), and a hand the
    /// user queued or sent out directly has nothing to do with a batch; neither waits on
    /// another delegation still out, which can mean waiting out a job of twenty minutes.
    private func flushableCrewReports() -> [CrewReport] {
        crewPendingReports.sorted { $0.handIndex < $1.handIndex }
    }

    /// The turn `flushCrewReports` scheduled. A lead no longer idle (the user got a word
    /// in first) keeps the reports waiting for the end of that turn instead.
    private func startCrewReportTurn() async {
        // Cleared first and the reports left where they are: a turn that cannot run now
        // must be able to be asked for again, by the next hand to finish or by the end of
        // whatever the lead is doing instead.
        crewFlushScheduled = false
        guard phase == .idle, !isReverting, let app else { return }
        let reports = flushableCrewReports()
        guard !reports.isEmpty else { return }
        let sent = Set(reports.map(\.handIndex))
        crewPendingReports.removeAll { sent.contains($0.handIndex) }
        let stillWorking = app.workingCrewHandNames(of: threadID, excluding: sent)
        let text = CrewPrompts.reportMessage(reports, stillWorking: stillWorking, reviewsHands: app.settings.crewReviewHands, verificationOwed: crewVerificationOwed)
        crewVerificationOwed = CrewPrompts.verificationStep(reports, stillWorking: stillWorking, owed: crewVerificationOwed) == .hold
        await ensureLoaded()
        await startTurn(text: text, attachments: [], crewHands: reports.map(\.handIndex))
    }

    /// What a reply's delegation block led to, for the end of the turn.
    private enum DelegationOutcome {
        /// No block, or nothing came of it.
        case none
        /// Hands went out; the lead is free until they report.
        case handsOut
        /// The lead is being told something instead, in a turn of its own already on its way.
        case turnStarted
    }

    /// A reply row of a turn that did not finish, holding a block: the block leaves it,
    /// and the timeline says the hands never went out.
    private func dropDelegationBlock(for turnID: UUID, status: TurnStatus) {
        if let stream = crewStream, stream.turnID == turnID, stream.sent > 0,
           let streamedEntry = entryIndex[stream.entryID],
           case .assistant(var streamedMessage) = streamedEntry.item.content {
            crewStream = nil
            streamedMessage.text = CrewPrompts.withoutDelegationBlock(streamedMessage.text)
            if streamedMessage.text.isEmpty { streamedMessage.text = "Asked for hands." }
            streamedEntry.item.content = .assistant(streamedMessage)
            saveRevision += 1
            scheduleSave()
            settleBatches()
            appendCrewNote("Crew sent \(stream.sent == 1 ? "one hand" : "\(stream.sent) hands") before the turn \(status == .interrupted ? "was stopped" : "failed"); the rest of the block never went out. Ask again to send them.")
            return
        }
        if let stream = crewStream, stream.turnID == turnID, stream.sent == 0 { crewStream = nil }
        guard let entry = entries.last(where: { $0.turnID == turnID && $0.kind == .assistant && Self.holdsDelegationBlock($0) }),
              case .assistant(var message) = entry.item.content else { return }
        message.text = CrewPrompts.withoutDelegationBlock(message.text)
        if message.text.isEmpty { message.text = "Asked for hands." }
        entry.item.content = .assistant(message)
        saveRevision += 1
        scheduleSave()
        appendCrewNote("Crew sent no hands: the turn \(status == .interrupted ? "was stopped" : "failed") before they could go out. Ask again to send them.")
    }

    private static func holdsDelegationBlock(_ entry: TimelineEntry) -> Bool {
        guard case .assistant(let message) = entry.item.content else { return false }
        return CrewPrompts.hasDelegationBlock(in: message.text)
    }

    /// A reply on any provider may end in a delegation block: its tasks go out as
    /// hosted hands, up to the pair's limit at a time, and the block leaves the reply.
    /// A block Bottega cannot read leaves the reply as well, and the lead hears so in
    /// a turn of its own: a block never stays in a reply doing nothing.
    private func spawnDelegatedHands(for turnID: UUID) -> DelegationOutcome {
        // The row holding the block: the last reply row that has one, since a reply can
        // come as several rows with tool calls between them and the block may sit in any
        // of them; with none, the last reply row (which may be empty of blocks).
        guard let app, let thread, let launch = app.crewLaunch(for: thread),
              let entry = entries.last(where: { $0.turnID == turnID && $0.kind == .assistant && Self.holdsDelegationBlock($0) })
                ?? entries.last(where: { $0.turnID == turnID && $0.kind == .assistant }),
              case .assistant(var message) = entry.item.content else { return .none }
        // A block read as it streamed: an entry that closed after the last flush goes out
        // now, then the block is finished here rather than read again from the top.
        scanStreamedDelegations(entryID: crewStream?.entryID ?? entry.id, turnID: turnID)
        if let stream = crewStream, stream.turnID == turnID,
           let streamedEntry = entryIndex[stream.entryID],
           case .assistant(var streamedMessage) = streamedEntry.item.content {
            crewStream = nil
            streamedMessage.text = CrewPrompts.markingDelegationBlockSent(streamedMessage.text)
            streamedEntry.item.content = .assistant(streamedMessage)
            saveRevision += 1
            let all = CrewPrompts.streamedDelegations(in: streamedMessage.text)?.delegations ?? []
            let total = all.isEmpty ? stream.sent : all.count
            // What the app's last read of the block missed goes out here rather than being
            // reported as lost: only the ceiling of a block, or a request that has had all
            // its rounds of hands, holds a task back, and only that is named in the note.
            var sent = stream.sent
            if total > sent, crewDelegationRounds < launch.limits.maxDelegationRounds {
                let rest = Array(all.dropFirst(sent).prefix(max(0, launch.limits.maxDelegatedTasks - sent)))
                if !rest.isEmpty {
                    crewWaiting += rest.map { (delegation: $0, batchID: stream.batchID, queuedAt: Date.now) }
                    sent += rest.count
                    crewStream = (turnID: turnID, entryID: stream.entryID, batchID: stream.batchID, sent: sent)
                    spawnWaitingHands(launch: launch)
                }
            }
            if total > sent {
                appendCrewNote("""
                    Not every task went out
                    \(sent) of the \(total) tasks in that block went to hands. The last \(total - sent) did not: ask for them again once these report back.
                    """)
            }
            holdSentDelegationBlock(streamedEntry.id)
            settleBatches()
            scheduleSave()
            return outcomeAfterQueueing(launch)
        }
        guard let delegations = CrewPrompts.delegations(in: message.text) else {
            guard CrewPrompts.hasDelegationBlock(in: message.text) else { return .none }
            message.text = CrewPrompts.withoutDelegationBlock(message.text)
            if message.text.isEmpty { message.text = "Asking for hands." }
            entry.item.content = .assistant(message)
            saveRevision += 1
            scheduleSave()
            return refuseBlock(CrewPrompts.unreadableBlockMessage(reason: nil), dropped: "Its delegation block could not be read, and it had been told so already.", limits: launch.limits)
        }
        let stripped = CrewPrompts.withoutDelegationBlock(message.text)
        // An empty block is the lead saying it did the work itself: the block leaves the
        // reply, a one-line note says no hands went out, and no turn is spent on it.
        if delegations.isEmpty {
            message.text = stripped.isEmpty ? "Done, with no hands." : stripped
            entry.item.content = .assistant(message)
            saveRevision += 1
            scheduleSave()
            appendCrewNote("No hands went out: the lead did this itself.")
            return .none
        }
        // One request gets so many rounds of hands; past that the lead hears why none went
        // out and finishes by itself, so no request chains hands without end.
        let limits = launch.limits
        guard crewDelegationRounds < limits.maxDelegationRounds else {
            message.text = stripped.isEmpty ? "Asking for more hands." : stripped
            entry.item.content = .assistant(message)
            saveRevision += 1
            scheduleSave()
            return refuseBlock(CrewPrompts.heldBackMessage(count: delegations.count, maxDelegationRounds: limits.maxDelegationRounds), dropped: "This request has had all \(limits.maxDelegationRounds) of its rounds of hands, and the lead had been told so already.", limits: limits)
        }
        crewDelegationRounds += 1
        // The card stays a moment in its sent state, then leaves the reply (see `holdSentDelegationBlock`).
        message.text = CrewPrompts.markingDelegationBlockSent(message.text)
        entry.item.content = .assistant(message)
        saveRevision += 1
        let batchID = UUID()
        crewBatches[batchID] = CrewBatch(pending: [])
        // The pair's cap paces these out through `crewWaiting`, so the ceiling is only on
        // how many tasks one block may hold. A lead that writes more than that hears which
        // ones did not go out, rather than losing the tail of its own list in silence.
        crewWaiting += delegations.prefix(limits.maxDelegatedTasks).map { (delegation: $0, batchID: batchID, queuedAt: Date.now) }
        if delegations.count > limits.maxDelegatedTasks {
            let dropped = delegations.count - limits.maxDelegatedTasks
            appendCrewNote("""
                Not every task went out
                \(limits.maxDelegatedTasks) of the \(delegations.count) tasks in that block went to hands. The last \(dropped) did not: ask for them again once these report back.
                """)
        }
        noteOverlappingClaims(Array(delegations.prefix(limits.maxDelegatedTasks)))
        if launch.holdsDelegations {
            noteHeldHands()
        } else {
            spawnWaitingHands(launch: launch)
        }
        holdSentDelegationBlock(entry.id)
        settleBatches()
        scheduleSave()
        return outcomeAfterQueueing(launch)
    }

    /// What the finished turn is told about the hands it queued.
    ///
    /// Held hands have not left, so the lead's turn really is over: the thread must go
    /// idle, because `send()` only takes a local command (`/go`) while idle — reporting
    /// `.handsOut` here would leave the chat reading as "working" with nothing running and
    /// no way to release anything.
    private func outcomeAfterQueueing(_ launch: CrewLaunch) -> DelegationOutcome {
        if launch.holdsDelegations { return .none }
        return crewBatches.isEmpty && crewWaiting.isEmpty ? .none : .handsOut
    }

    /// The delegation card keeps its place for `CrewPrompts.delegationSentHold` once the
    /// hands are out, wearing its done wave. Then it fades where it stands, still holding
    /// its room (`crew-leaving`), and only once it is gone from sight does the block leave
    /// the reply and the text below close up: two steps, so the card never fades over
    /// content that has already moved up under it.
    private func holdSentDelegationBlock(_ entryID: String) {
        Task { [weak self] in
            try? await Task.sleep(for: CrewPrompts.delegationSentHold)
            guard let self, let entry = self.entryIndex[entryID],
                  case .assistant(var message) = entry.item.content,
                  CrewPrompts.hasSentDelegationBlock(in: message.text) else { return }
            message.text = CrewPrompts.markingDelegationBlockLeaving(message.text)
            withAnimation(.easeOut(duration: 0.4)) { entry.item.content = .assistant(message) }
            try? await Task.sleep(for: CrewPrompts.delegationLeaveFade)
            guard case .assistant(var faded) = entry.item.content,
                  CrewPrompts.hasSentDelegationBlock(in: faded.text) else { return }
            faded.text = CrewPrompts.withoutDelegationBlock(faded.text)
            if faded.text.isEmpty { faded.text = "Sent out hands." }
            withAnimation(Chrome.panelSlide) { entry.item.content = .assistant(faded) }
            self.saveRevision += 1
            self.scheduleSave()
        }
    }

    /// Tells the lead, in a turn of its own, that the block its reply ended in sent no hands
    /// out, and why: `message` is what it hears. Only so many times per request (see
    /// `CrewLimits.maxRefusedBlocks`): a lead that keeps answering with the same block would
    /// otherwise be answered with the same refusal, turn after turn, for as long as the app
    /// runs. Past the cap the block is dropped with a note in the timeline saying `dropped`,
    /// and the turn ends as a plain reply would.
    private func refuseBlock(_ message: String, dropped: String, limits: CrewLimits) -> DelegationOutcome {
        guard crewRefusedBlocks < limits.maxRefusedBlocks else {
            appendCrewNote("No hands went out\n\(dropped) The block was dropped from the reply.")
            return .none
        }
        crewRefusedBlocks += 1
        Task { await ensureLoaded(); await startTurn(text: message, attachments: [], crewHands: []) }
        return .turnStarted
    }

    /// Hands go out as their entries close so the first is at work while the lead writes
    /// the rest; the batch is made on the first entry and the round counted then; the cap
    /// paces the rest through `crewWaiting`; `spawnDelegatedHands` finishes the block
    /// at the end of the turn.
    private func scanStreamedDelegations(entryID: String, turnID: UUID) {
        guard let app, let thread, let entry = entryIndex[entryID], entry.turnID == turnID,
              case .assistant(let message) = entry.item.content else { return }
        guard let launch = app.crewLaunch(for: thread) else { return }
        if let crewStream, crewStream.entryID != entryID || crewStream.turnID != turnID { return }
        guard let streamed = CrewPrompts.streamedDelegations(in: message.text) else { return }
        let sent = crewStream?.sent ?? 0
        let fresh = Array(streamed.delegations.dropFirst(sent).prefix(max(0, launch.limits.maxDelegatedTasks - sent)))
        guard !fresh.isEmpty else { return }
        guard crewDelegationRounds < launch.limits.maxDelegationRounds else { return }
        let batchID: UUID
        if crewStream == nil {
            batchID = UUID()
            crewBatches[batchID] = CrewBatch(pending: [])
            crewDelegationRounds += 1
            crewStream = (turnID: turnID, entryID: entryID, batchID: batchID, sent: 0)
        } else {
            batchID = crewStream!.batchID
        }
        crewWaiting += fresh.map { (delegation: $0, batchID: batchID, queuedAt: Date.now) }
        crewStream?.sent = sent + fresh.count
        noteOverlappingClaims(fresh)
        // Held back, the entries stay queued and the watermark still moves, so the rest of
        // the block streams in and joins them rather than being dispatched twice on release.
        guard !launch.holdsDelegations else { return noteHeldHands() }
        spawnWaitingHands(launch: launch)
    }

    /// Two hands on one file is the split the lead is told not to make, and the one thing
    /// parallel work cannot survive: they never see each other, so the later write wins and
    /// the earlier hand's report describes a file that no longer looks like that. Entries
    /// carry the files they claim, so the clash is caught here, while both are still
    /// queued, instead of in the reports afterwards. Entries without files are not checked.
    private func noteOverlappingClaims(_ fresh: [CrewDelegation]) {
        var clashes: [String] = []
        for delegation in fresh {
            for path in delegation.files {
                let key = path.trimmingCharacters(in: .whitespaces)
                guard !key.isEmpty else { continue }
                if let owner = crewClaimedPaths[key], owner != delegation.task {
                    clashes.append("\(key) — \(owner) and \(delegation.task)")
                } else {
                    crewClaimedPaths[key] = delegation.task
                }
            }
        }
        guard !clashes.isEmpty else { return }
        appendCrewNote("""
            Two hands claim the same file
            \(clashes.joined(separator: "; ")). They run in parallel and never see each other's work, so one overwrites the other. Send those tasks one after another, or give each hand files of its own.
            """)
    }

    /// The hands are queued but not sent. The user's own rules put the hand count and the
    /// cost before any fan-out, so this is the gate that makes the lead's estimate
    /// something to answer rather than something to read past.
    private func noteHeldHands() {
        let waiting = crewWaitingCount
        guard waiting > 0 else { return }
        crewHeldForGo = true
        appendCrewNote("""
            \(waiting == 1 ? "A hand is" : "\(waiting) hands are") waiting for your go-ahead
            Crew is set to hold hands until you release them (Settings › Crew). Send /go to start \(waiting == 1 ? "it" : "them"), or say what to change first.
            """)
    }

    /// Releases the hands held back for the user's go-ahead. Sent by `/go`.
    func releaseHeldHands() {
        guard crewWaitingCount > 0, let app, let thread else { return }
        let waiting = crewWaitingCount
        appendCrewNote("Releasing \(waiting == 1 ? "the hand" : "\(waiting) hands").")
        crewHeldForGo = false
        spawnWaitingHands(launch: app.crewLaunch(for: thread))
    }

    /// Stop all hands: the delegated tasks still waiting for a hand go with the hands at work,
    /// rather than going out one by one as each stopped hand makes room (`crewHandFinished`
    /// sends the queue on). The timeline names them, as it does a block cut short
    /// (`dropDelegationBlock`), so what was never done is not lost in silence. The user's own
    /// queued messages stay queued.
    func dropWaitingHands() {
        guard !crewWaiting.isEmpty else { return }
        let dropped = crewWaiting
        crewWaiting.removeAll()
        settleBatches()
        let listed = dropped.map { "- " + TextCleanup.singleLine($0.delegation.task, limit: 120) }.joined(separator: "\n")
        appendCrewNote("""
            \(dropped.count == 1 ? "A delegated task was" : "\(dropped.count) delegated tasks were") dropped
            Stop all hands stopped the \(dropped.count == 1 ? "task" : "tasks") still waiting for a hand too; \(dropped.count == 1 ? "it never went" : "none went") out:
            \(listed)
            """)
    }

    /// Sends out delegated tasks still waiting, as far as the cap allows (the pair's or the
    /// Crew page's, else `CrewLimits.uncappedConcurrency`); with Crew off, all of them. Held
    /// for the user's go-ahead, none (see `crewHeldForGo`).
    private func spawnWaitingHands(launch: CrewLaunch? = nil) {
        guard let app, let thread, !crewWaiting.isEmpty, !crewHeldForGo else { return }
        let launch = launch ?? app.crewLaunch(for: thread)
        // Hands not on the provider the pair names (it is off or not installed): said once
        // per job, in the lead's timeline, rather than never.
        if let pair = app.crewPair(for: thread), let note = app.crewHandsFallbackNote(of: pair), crewFallbackNoted != pair.id {
            crewFallbackNoted = pair.id
            appendCrewNote("Crew: \(note)")
        }
        // Every check stays with the lead unless the user said otherwise: a brief's check
        // lines come out, and the chat says which (see `noteRemovedChecks`).
        let stripsChecks = launch?.stripsVerificationSteps ?? app.settings.crewStripsVerificationSteps
        // The main chat as it stands, read once for the hands going out now: a delegated
        // hand has none of the lead's context and would otherwise know only its brief.
        var context: CrewPrompts.ChatContext?
        var removedChecks: [(name: String, lines: [String])] = []
        while !crewWaiting.isEmpty, launch?.hasRoom(running: app.runningHostedHands(of: threadID)) ?? true {
            let next = crewWaiting.removeFirst()
            let delegation = next.delegation
            // The lead's announced name is authoritative: resolving it here pins the
            // spawned hand to exactly that roster name. Unknown or already-taken names
            // resolve to nil and the hand goes out next in order, as before.
            let preferredIndex = delegation.name.flatMap(CrewRoster.index(named:))
            let chat = context ?? app.crewChatContext(for: threadID)
            context = chat
            guard let hand = app.spawnHostedHand(from: threadID, task: delegation.task, origin: .delegated, batchID: next.batchID, preferredIndex: preferredIndex, project: delegation.project, profile: delegation.profile, queuedAt: next.queuedAt, brief: { persona, workplace in
                CrewPrompts.delegatedHandPrompt(persona: persona, delegation: delegation, workplace: workplace, context: chat, stripsVerificationSteps: stripsChecks)
            }) else { continue }
            crewBatches[next.batchID, default: CrewBatch(pending: [])].pending.insert(hand.id)
            if stripsChecks, let name = hand.crew?.persona.name {
                let removed = CrewPrompts.verificationSteps(in: delegation.prompt).removed
                if !removed.isEmpty { removedChecks.append((name: name, lines: removed)) }
            }
        }
        noteRemovedChecks(removedChecks)
    }

    /// A brief's check lines were taken out before its hand saw them (see
    /// `CrewPrompts.verificationSteps(in:)`): the chat names them, hand by hand, so a step
    /// the filter took is never lost in silence. The hand's own brief quotes the same lines.
    private func noteRemovedChecks(_ removed: [(name: String, lines: [String])]) {
        guard !removed.isEmpty else { return }
        let count = removed.reduce(0) { $0 + $1.lines.count }
        let whose = removed.count == 1 ? "\(removed[0].name)'s brief" : "the hands' briefs"
        let listed = removed.map { entry -> String in
            let shown: [String] = entry.lines.prefix(4).map { "- \(entry.name): \(TextCleanup.singleLine($0, limit: 160))" }
            let more: [String] = entry.lines.count > 4 ? ["- \(entry.name): and \(entry.lines.count - 4) more"] : []
            return (shown + more).joined(separator: "\n")
        }.joined(separator: "\n")
        appendCrewNote("""
            \(count == 1 ? "A check was" : "\(count) checks were") taken out of \(whose)
            \(listed)

            Every check stays with the lead (Settings › Crew). The \(removed.count == 1 ? "hand was" : "hands were") told to make any edit these lines name and to run none of the checks.
            """)
    }

    // MARK: - Timeline mutations

    /// A note from Crew in the timeline, with no turn behind it: what it merged, what it
    /// could not. Its first line is its title. An `id` lets the note land under an identity
    /// the timeline is already drawing (the merging pill, see `crewMergeNoteID`).
    func appendCrewNote(_ text: String, id: String? = nil) {
        append(TimelineItem(id: id ?? UUID().uuidString, turnID: nil, content: .user(UserMessage(text: text, crewHands: []))))
        app?.updateThread(threadID) { $0.updatedAt = .now }
        scheduleSave()
    }

    private func append(_ item: TimelineItem) {
        for entry in entries.suffix(6) { endStreaming(entry) }
        let entry = TimelineEntry(item)
        entries.append(entry)
        entryIndex[item.id] = entry
        saveRevision += 1
    }

    private func remove(_ id: String) {
        entries.removeAll { $0.id == id }
        entryIndex[id] = nil
        saveRevision += 1
    }

    private func endStreaming(_ entry: TimelineEntry) {
        switch entry.item.content {
        case .assistant(var message) where message.isStreaming:
            message.isStreaming = false
            entry.item.content = .assistant(message)
        case .reasoning(var block) where block.isStreaming:
            block.isStreaming = false
            entry.item.content = .reasoning(block)
        case .plan(var plan) where plan.state == .drafting:
            plan.state = .proposed
            entry.item.content = .plan(plan)
        default:
            break
        }
    }

    private func queueDelta(_ id: String, _ kind: DeltaKind, _ text: String) {
        if entryIndex[id] == nil {
            switch kind {
            case .message:
                // Not until there is something to read: some providers open a text
                // block with an empty delta, which would leave a blank reply row behind.
                guard !text.isEmpty else { return }
                append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .assistant(AssistantMessage(text: "", isStreaming: true))))
            case .reasoning:
                // Not until there is something to read: a redacted thinking block
                // only ever sends empty deltas and would leave a blank entry behind.
                guard !text.isEmpty else { return }
                append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .reasoning(ReasoningBlock(text: "", isStreaming: true))))
            case .plan:
                append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .plan(ProposedPlan(markdown: "", state: .drafting))))
            case .toolOutput:
                return
            }
        }
        guard !text.isEmpty else { return }
        // Prose is flattened as it streams, so a reply can never render an em dash.
        // Tool output is process text, not the model's own writing, and passes through.
        let prose = kind == .toolOutput ? text : TextCleanup.withoutEmDashes(text)
        pendingDeltas[id, default: PendingDelta(kind: kind, text: "")].text += prose
        guard flushTask == nil else { return }
        // The copy in `flushDeltas` below is O(n) in the row's text: as the reply
        // grows the flush waits longer, so long replies flush half or a third as
        // often with no visible effect. The copy itself cannot go away while the
        // row's content is a value-typed enum behind an observed property.
        // The first delta after a lull shows at once, so the first token of a reply never
        // waits out the window; the ones behind it coalesce, one flush per pacing delay.
        // A hand shares the screen with every other hand streaming at the same time, and
        // each flush re-lays the panel: with twenty hands talking at once the main thread
        // never caught up and the app froze. Hands pace themselves by how many are
        // streaming (see `StreamLoad`); the lead's own reply keeps its pace.
        let isHand = thread?.crew != nil
        if isHand { StreamLoad.shared.noteStreaming(self) }
        if ContinuousClock.now - lastFlushAt >= (isHand ? StreamLoad.shared.handFlushWindow : Self.flushWindow) {
            flushDeltas()
            return
        }
        flushTask = Task { [weak self] in
            var delay = Self.flushDelay(for: self?.pendingFlushLength(), scrolling: ScrollActivity.shared.isScrolling)
            if isHand { delay = max(delay, StreamLoad.shared.handFlushDelayMilliseconds) }
            try? await Task.sleep(for: .milliseconds(delay))
            self?.flushDeltas()
        }
    }

    /// The text the next flush will write onto its row, used to pace the flush:
    /// short replies refresh fast, long ones less often.
    private func pendingFlushLength() -> Int {
        var longest = 0
        for (id, delta) in pendingDeltas {
            switch delta.kind {
            case .toolOutput:
                continue
            default:
                break
            }
            switch entryIndex[id]?.item.content {
            case .assistant(let message):
                longest = max(longest, message.text.count + delta.text.count)
            case .reasoning(let block):
                longest = max(longest, block.text.count + delta.text.count)
            case .plan(let plan):
                longest = max(longest, plan.markdown.count + delta.text.count)
            default:
                break
            }
        }
        return longest
    }

    /// How long to wait before flushing deltas onto their rows: 45 ms up to 4 KB,
    /// 90 ms up to 32 KB, 150 ms beyond.
    private nonisolated static func flushDelay(for length: Int?, scrolling: Bool) -> Int {
        let paced = switch length ?? 0 {
        case ..<4_096: 45
        case ..<32_768: 90
        default: 150
        }
        // Under a scrolling reader the row is not being read: it takes at most six
        // flushes a second then, so the frames of the scroll are not spent laying the
        // streaming text out again (in the chat and in every hand panel at once).
        return scrolling ? max(paced, 160) : paced
    }

    private static let flushWindow: Duration = .milliseconds(45)

    /// How many runtimes are streaming onto the screen right now, so a hand's flushes
    /// slow down as more hands talk at once. One hand flushes at its own pace; with N
    /// hands streaming, each waits about N times `handFlushBudget`, which keeps the
    /// whole team to roughly one panel re-layout every `handFlushBudget` regardless of
    /// how many hands went out. A runtime that has not flushed for two seconds no longer
    /// counts, so a hand that went quiet stops slowing the others.
    @MainActor
    final class StreamLoad {
        static let shared = StreamLoad()
        /// The interval the whole team of hands shares, in milliseconds.
        private static let handFlushBudget = 70
        private static let stale: Duration = .seconds(2)
        private var lastSeen: [ObjectIdentifier: ContinuousClock.Instant] = [:]

        func noteStreaming(_ runtime: ThreadRuntime) {
            lastSeen[ObjectIdentifier(runtime)] = .now
        }

        func forget(_ runtime: ThreadRuntime) {
            lastSeen[ObjectIdentifier(runtime)] = nil
        }

        /// Runtimes that flushed within the last two seconds.
        var streamingCount: Int {
            let now = ContinuousClock.now
            lastSeen = lastSeen.filter { now - $0.value < Self.stale }
            return lastSeen.count
        }

        /// The least a hand waits between two flushes while `streamingCount` hands talk.
        var handFlushDelayMilliseconds: Int {
            max(45, streamingCount * Self.handFlushBudget)
        }

        var handFlushWindow: Duration {
            .milliseconds(handFlushDelayMilliseconds)
        }
    }

    private func flushDeltas(scheduleSave: Bool = true) {
        flushTask?.cancel()
        flushTask = nil
        guard !pendingDeltas.isEmpty else { return }
        lastFlushAt = .now
        let deltas = pendingDeltas
        pendingDeltas.removeAll()
        var heard = false
        var streamedRows: [String] = []
        for (id, delta) in deltas {
            // Appended in place (see `appendStreamed`): binding the payload out of the enum
            // copied the whole message on every flush.
            guard let entry = entryIndex[id], entry.item.content.appendStreamed(delta.text, kind: delta.kind) else { continue }
            saveRevision += 1
            if delta.kind != .plan { heard = true }
            if delta.kind == .message, entry.kind == .assistant, !streamedRows.contains(id) { streamedRows.append(id) }
        }
        // Once per flush, not per word: a hand still talking is a hand still at work.
        if heard { noteCrewHandEvent() }
        if let turnID = currentTurnID { for id in streamedRows { scanStreamedDelegations(entryID: id, turnID: turnID) } }
        if scheduleSave { self.scheduleSave() }
    }

    private func completeMessage(_ id: String, text: String) {
        let text = TextCleanup.withoutEmDashes(text)
        guard let entry = entryIndex[id] else {
            guard !text.isEmpty else { return }
            append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .assistant(AssistantMessage(text: text))))
            return
        }
        guard case .assistant(var message) = entry.item.content else { return }
        if !text.isEmpty { message.text = text }
        message.isStreaming = false
        if !message.text.contains(where: { !$0.isWhitespace }) {
            remove(id)
        } else {
            entry.item.content = .assistant(message)
            saveRevision += 1
            if let turnID = currentTurnID { scanStreamedDelegations(entryID: id, turnID: turnID) }
        }
        scheduleSave()
    }

    private func completeReasoning(_ id: String, text: String) {
        let text = TextCleanup.withoutEmDashes(text)
        guard let entry = entryIndex[id], case .reasoning(var block) = entry.item.content else {
            guard !text.isEmpty else { return }
            append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .reasoning(ReasoningBlock(text: text))))
            return
        }
        if !text.isEmpty { block.text = text }
        block.isStreaming = false
        if !block.text.contains(where: { !$0.isWhitespace }) {
            remove(id)
        } else {
            entry.item.content = .reasoning(block)
            saveRevision += 1
        }
        scheduleSave()
    }

    private func completePlan(_ id: String, markdown: String) {
        let markdown = TextCleanup.withoutEmDashes(markdown)
        if let entry = entryIndex[id], case .plan(var plan) = entry.item.content {
            if !markdown.isEmpty { plan.markdown = markdown }
            plan.state = .proposed
            entry.item.content = .plan(plan)
            saveRevision += 1
        } else if !markdown.isEmpty {
            append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .plan(ProposedPlan(markdown: markdown, state: .proposed))))
        }
        scheduleSave()
    }

    private func upsertTool(_ id: String, _ call: ToolCall) {
        noteToolMayWrite(call.kind)
        var call = call
        call.edits = Self.capped(call.edits)
        guard let entry = entryIndex[id], case .tool(var existing) = entry.item.content else {
            append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .tool(call)))
            noteCrewHandEvent(tool: true)
            return
        }
        if !call.title.isEmpty { existing.title = call.title }
        existing.detail = call.detail ?? existing.detail
        existing.kind = call.kind
        if !call.edits.isEmpty { existing.edits = call.edits }
        if existing.status == .running, call.status != .running { existing.finish(call.status) }
        entry.item.content = .tool(existing)
        saveRevision += 1
        noteCrewHandEvent()
    }

    /// Something happened on this runtime's own timeline, and it is a hand's (a tool, a
    /// word, a request put to a person, a person's answer): its watchdog clock restarts
    /// (see `AppModel.startCrewWatchdog`). A hosted hand counts its tools here, where its
    /// own session reports them; a native hand's count comes with its lead's events (see
    /// `crewAgentEvent`), which also rehearse here, so it is not counted twice. The files
    /// its tools wrote are not counted here: they are read off the rows when the watchdog
    /// asks (see `crewLandedEdits`).
    private func noteCrewHandEvent(tool: Bool = false) {
        guard let info = thread?.crew else { return }
        crewLastEventAt = .now
        if tool, info.kind == .hosted { crewToolCalls += 1 }
    }

    /// A tool that is not plainly a read can have written a file, which makes the tree a
    /// command left behind no answer for what is on disk now. Commands keep their own
    /// count in `watchCommand`, which knows whether this one can write at all.
    private func noteToolMayWrite(_ kind: ToolCall.Kind?) {
        guard let kind, kind != .read, kind != .search, kind != .web, kind != .command else { return }
        settledTree = nil
        treeEpoch += 1
    }

    private func applyToolUpdate(_ id: String, _ update: ToolUpdate) {
        noteToolMayWrite(update.kind)
        if entryIndex[id] == nil {
            append(TimelineItem(id: id, turnID: turnIDForNewRows, content: .tool(ToolCall(kind: update.kind ?? .other, title: update.title ?? "Tool"))))
        }
        guard let entry = entryIndex[id], case .tool(var call) = entry.item.content else { return }
        if let title = update.title, !title.isEmpty { call.title = title }
        if let detail = update.detail { call.detail = detail }
        if let kind = update.kind { call.kind = kind }
        if let edits = update.edits, !edits.isEmpty { call.edits = Self.capped(edits) }
        if let output = update.output { call.setOutput(output) }
        if let exitCode = update.exitCode { call.exitCode = exitCode }
        if let status = update.status {
            if status == .running { call.status = .running } else { call.finish(status) }
        }
        entry.item.content = .tool(call)
        saveRevision += 1
        noteCrewHandEvent()
        scheduleSave()
    }

    // MARK: - Command edits

    /// Agents that edit files from the shell (python heredocs, sed -i, patch)
    /// never report an edit, so a command's row gets its edits from the
    /// working tree instead: a tree known to predate the command, another
    /// captured as it finishes, and the diff between the two. Git-backed projects only.
    ///
    /// Nothing is captured as the command starts. The agent runs the command itself,
    /// the moment its tool call arrives, and a heredoc is done in a blink while a
    /// snapshot of a big checkout takes seconds: a snapshot started here raced the
    /// command and, more often than not, already held its write. Before and after then
    /// matched, the row got no edits, the turn counted no file, and the lead's merge had
    /// nothing to take; a sibling chat's sweep landed the work under its own title
    /// instead. The "before" is now always a tree that was complete before the command
    /// began (see `treeBeforeCommand`).
    private func watchCommand(_ id: String, _ call: ToolCall) {
        // A command that cannot write needs no snapshots at all: reading the checkout is
        // most of what an agent runs, and each snapshot is seconds of git on a big tree.
        guard commandTrees[id] == nil, ShellCommandKind.mayWriteFiles(call.title), repositoryGit != nil,
              let before = treeBeforeCommand() else { return }
        commandTrees[id] = Task<String?, Never> { before }
        settledTree = nil
        treeEpoch += 1
    }

    /// A tree-ish that stood complete before a command starting now: the tree the last
    /// command left when nothing has run since, else the last snapshot any command in
    /// this turn finished, else the turn's own start checkpoint, which the turn waited
    /// for before its first word reached the model. Each is older than the command, so
    /// a diff from it can only hold more than the command wrote, never less; what an
    /// earlier tool of the turn already reported is taken back out as the row settles.
    private func treeBeforeCommand() -> String? {
        if let settled = settledTree, settled.epoch == treeEpoch { return settled.tree }
        if let latestTree { return latestTree }
        guard let currentTurnID, let turn = turns.first(where: { $0.id == currentTurnID }) else { return nil }
        return turn.baseCheckpoint
    }

    /// Repository-relative paths that other rows of a turn already carry as landed edits,
    /// so a command settling against an older tree does not show the turn's earlier work as
    /// its own. A path the command also touched stays on the row that reported it first
    /// and still counts for the turn. A row whose edits never landed (declined, failed,
    /// still waiting on an approval) takes nothing from the command: a file the command
    /// really wrote must not vanish from the turn because a refused write named it first.
    private func attributedPaths(inTurnOf id: String, git: Git, repositoryRoot: URL) -> Set<String> {
        guard let turnID = entryIndex[id]?.item.turnID else { return [] }
        var paths = Set<String>()
        for entry in entries where entry.id != id && entry.item.turnID == turnID {
            guard case .tool(let call) = entry.item.content else { continue }
            for edit in call.landedEdits {
                let trimmed = edit.path.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty else { continue }
                let absolute = trimmed.hasPrefix("/") || trimmed.hasPrefix("~") ? trimmed : (git.directory.path as NSString).appendingPathComponent(trimmed)
                if let relative = TouchedPaths.relative(absolute, root: repositoryRoot.path) { paths.insert(relative) }
            }
        }
        return paths
    }

    private func settleCommand(_ id: String) {
        guard let before = commandTrees.removeValue(forKey: id), let git = repositoryGit else { return }
        let epoch = treeEpoch
        commandSettles[id] = Task { [weak self] in
            guard let base = await before.value, let self else { return }
            guard let after = await captureTree(git).value else { return }
            latestTree = after
            // Nothing else started or ran alongside while the snapshot was taken, so it
            // still stands for the working tree and the next command starts from it.
            if treeEpoch == epoch, commandTrees.isEmpty, crewCommandTrees.isEmpty { settledTree = (after, epoch) }
            guard base != after, let changed = try? await git.changedPaths(from: base, to: after) else { return }
            let repository = repositoryRoot(for: git)
            let attributed = attributedPaths(inTurnOf: id, git: git, repositoryRoot: await repository.value)
            // Only the files that are work: a hand that builds in its copy leaves thousands of
            // compiler-cache records under build.noindex where the project's .gitignore missed
            // them, and a diff of all of those once put 7,632 edits and 7 MB on two rows, then
            // into the merge as strays. Listed first and diffed by name, they never get that far.
            let real = changed.filter { !$0.isEmpty && !TouchedPaths.isBuildOutput($0) && !attributed.contains($0) }
            guard !real.isEmpty, let patch = try? await git.diff(from: base, to: after, paths: real), !patch.isEmpty else { return }
            let edits = await Self.fileEdits(from: patch, repositoryRoot: repository.value)
            guard !edits.isEmpty, let entry = entryIndex[id], case .tool(var call) = entry.item.content else { return }
            // A provider that did report its edits keeps them.
            guard call.edits.allSatisfy({ $0.diff == nil }) else { return }
            call.edits = edits
            entry.item.content = .tool(call)
            saveRevision += 1
            noteCrewHandEvent()
            scheduleSave()
        }
    }

    /// One working-tree snapshot at a time for this thread, each waiting for the one
    /// before it: they are heavy, and a turn's commands would otherwise run several at
    /// once, with eight hands doing the same in the same checkout.
    private func captureTree(_ git: Git) -> Task<String?, Never> {
        let previous = treeCaptures
        let capture = Task { () -> String? in
            _ = await previous?.value
            return await Task.detached(priority: .utility) { try? await git.captureTree() }.value
        }
        treeCaptures = capture
        return capture
    }

    /// The edits of the tool row `id` on this timeline that landed (see `ToolCall.landedEdits`);
    /// none when there is no such row.
    private func landedEdits(ofTool id: String) -> [FileEdit] {
        guard let entry = entryIndex[id], case .tool(let call) = entry.item.content else { return [] }
        return call.landedEdits
    }

    /// Files a native hand's tools changed. They land on the hand's own timeline, but the
    /// work is done in the lead's checkout and counts as the lead's turn.
    private func noteCrewEdits(_ edits: [FileEdit]) {
        guard !edits.isEmpty, let root = workingRoot else { return }
        for edit in edits {
            if let path = TouchedPaths.relative(edit.path, root: root) { crewTouched.insert(path) }
        }
    }

    /// A native hand's shell command, watched exactly as the lead's own are and for the
    /// same reason: agents edit from the shell without reporting it. What changed goes to
    /// the turn's files rather than onto a row, because the row lives in the hand's
    /// runtime and it is the lead's turn that a merge takes its files from.
    private func watchCrewCommand(_ key: String, _ call: ToolCall) {
        guard crewCommandTrees[key] == nil, ShellCommandKind.mayWriteFiles(call.title), repositoryGit != nil,
              let before = treeBeforeCommand() else { return }
        crewCommandTrees[key] = Task<String?, Never> { before }
        settledTree = nil
        treeEpoch += 1
    }

    private func settleCrewCommand(_ key: String, attributesChanges: Bool = true) {
        guard let before = crewCommandTrees.removeValue(forKey: key), let git = repositoryGit else { return }
        let epoch = treeEpoch
        // Kept with the lead's own settles, so the turn waits for them before it counts
        // its files.
        commandSettles[key] = Task { [weak self] in
            guard let base = await before.value, let self else { return }
            guard let after = await captureTree(git).value else { return }
            latestTree = after
            if treeEpoch == epoch, commandTrees.isEmpty, crewCommandTrees.isEmpty { settledTree = (after, epoch) }
            guard attributesChanges, base != after, let paths = try? await git.changedPaths(from: base, to: after) else { return }
            crewTouched.formUnion(paths.filter { !$0.isEmpty && !TouchedPaths.isBuildOutput($0) })
        }
    }

    /// The folder this thread's agent works in: its own copy of the checkout, or the
    /// project itself.
    private var workingRoot: String? {
        guard let app, let thread = app.thread(threadID), let project = app.project(thread.projectID) else { return nil }
        return thread.worktreePath ?? project.path
    }

    private var repositoryGit: Git? {
        guard let currentTurnID, let turn = turns.first(where: { $0.id == currentTurnID }), turn.baseCheckpoint != nil,
              let app, let thread = app.thread(threadID), let project = app.project(thread.projectID) else { return nil }
        return Git(thread.worktreePath ?? project.path)
    }

    /// How much of a patch an edit carries. Past it the edit is stats only, so neither the
    /// timeline nor the thread file grows by the whole of a written file.
    private nonisolated static let editDiffLimit = 200_000

    /// Edits as a provider reported them, with an outsized patch dropped: a Write tool
    /// hands over the entire new file, which can be megabytes, and every save would then
    /// write all of it again. The same cap the edits taken from a command's diff have.
    private static func capped(_ edits: [FileEdit]) -> [FileEdit] {
        edits.map { edit in
            guard let diff = edit.diff, diff.utf8.count > editDiffLimit else { return edit }
            var capped = edit
            capped.diff = nil
            return capped
        }
    }

    /// One edit per file in a multi-file patch, with the file's own section as its diff.
    @concurrent
    nonisolated static func fileEdits(from patch: String, repositoryRoot: URL) async -> [FileEdit] {
        var sections: [String] = []
        var current: [Substring] = []
        for line in patch.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.hasPrefix("diff --git ") {
                if !current.isEmpty { sections.append(current.joined(separator: "\n")) }
                current = [line]
            } else if !current.isEmpty {
                current.append(line)
            }
        }
        if !current.isEmpty { sections.append(current.joined(separator: "\n")) }
        return sections.compactMap { section in
            guard let file = DiffParser.parse(section).first else { return nil }
            // Huge sections are stats only, so the thread file stays small.
            let diff = section.utf8.count <= editDiffLimit ? section : nil
            let path = URL(fileURLWithPath: file.path, relativeTo: repositoryRoot).standardizedFileURL.path
            return FileEdit(path: path, diff: diff, additions: file.additions, deletions: file.deletions)
        }
    }

    private func upsertTodos(_ steps: [TodoStep]) {
        // The row is remembered rather than searched for: a turn rewrites its list many
        // times, and the timeline behind it can hold thousands of rows. Without it, the
        // running turn's entries are the last ones, and an earlier turn ends the walk.
        if let id = todosEntryID, let entry = entryIndex[id], entry.item.turnID == currentTurnID {
            entry.item.content = .todos(steps)
            saveRevision += 1
            return
        }
        var existing: TimelineEntry?
        for entry in entries.reversed() {
            if let turnID = entry.item.turnID, turnID != currentTurnID { break }
            if entry.item.turnID == currentTurnID, case .todos = entry.item.content { existing = entry; break }
        }
        if let entry = existing {
            todosEntryID = entry.id
            entry.item.content = .todos(steps)
            saveRevision += 1
        } else if !steps.isEmpty {
            let item = TimelineItem(turnID: turnIDForNewRows, content: .todos(steps))
            todosEntryID = item.id
            append(item)
        }
        scheduleSave()
    }

    private func appendNotice(_ level: Notice.Level, _ message: String) {
        append(TimelineItem(turnID: turnIDForNewRows, content: .notice(Notice(level: level, message: message))))
        scheduleSave()
    }

    private func updateTurn(_ id: UUID, _ change: (inout TurnRecord) -> Void) {
        guard let index = turns.firstIndex(where: { $0.id == id }) else { return }
        change(&turns[index])
        saveRevision += 1
    }

    /// What a provider's ledger counted on this turn, kept on the turn itself so a run's
    /// spend is still attributable after the app has moved on. Reports from the same
    /// provider, model and hand merge into one entry; a report carrying a hand's id stays
    /// that hand's alone, because a hand rehearses its own events on its own runtime.
    private func recordTokenSpend(_ spend: TokenSpend) {
        guard spend.totalTokens > 0, let turnID = currentTurnID else { return }
        updateTurn(turnID) { turn in
            var spends = turn.tokenSpends ?? []
            if let index = spends.firstIndex(where: {
                $0.provider == spend.provider && $0.model == spend.model && $0.agentID == spend.agentID
            }) {
                spends[index].merge(spend)
            } else {
                spends.append(spend)
            }
            turn.tokenSpends = spends
        }
    }

    /// Names the chat the moment its first message goes in, so the row never sits at
    /// 'New thread' while a session starts. Only a chat with no name of its own is ever
    /// named here, so a later turn cannot overwrite a title.
    private func generateTitle(from text: String, attachments: [Attachment]) {
        guard let app, let thread = app.thread(threadID), !thread.hasCustomTitle else { return }
        guard turns.count == 1 || thread.title == ChatThread.untitled else { return }
        let names = attachments.map(\.name)
        let input = names.isEmpty ? text : text + "\nAttached: " + names.joined(separator: ", ")
        app.updateThread(threadID) { $0.title = TextCleanup.singleLine(text, limit: 48) }
        guard let engine = app.textEngine(preferring: thread.provider) else { return }
        let threadID = threadID
        Task {
            guard let title = await TextGeneration.threadTitle(for: input, engine: engine) else { return }
            app.updateThread(threadID) { thread in
                if !thread.hasCustomTitle { thread.title = title }
            }
        }
    }

    // MARK: - Persistence

    /// How soon a change reaches the thread's file. Every save rewrites the whole file, which
    /// runs to megabytes, so a shorter wait costs disk writes and a longer one is what a crash
    /// can lose: idle, the save waits for edits to pause; while a turn runs, its events share
    /// one save per interval, and `finishTurn` saves what is left at the end.
    enum SaveCadence {
        /// The pause after an edit, outside a turn, before the file is written.
        static let idleDebounce: Duration = .milliseconds(800)
        /// The most a running turn's events wait to reach the file: the bound on what a
        /// crash mid-turn costs.
        static let runningInterval: Duration = .seconds(5)
    }

    /// Whether the thread's file may be written. Not while it is still there unread (see
    /// `loadProblem`): the runtime started without what it holds, and a save would put this
    /// run's timeline in its place.
    private var savesDocument: Bool {
        loadProblem?.blocksSaving != true
    }

    func scheduleSave() {
        guard persistenceEnabled, savesDocument else { return }
        guard phase != .idle else {
            saveTask?.cancel()
            saveTask = Task { [weak self] in
                try? await Task.sleep(for: SaveCadence.idleDebounce)
                guard !Task.isCancelled else { return }
                self?.saveNow()
            }
            return
        }
        // A running turn fires this after nearly every event it streams, and every save
        // rewrites the whole thread file, which runs to megabytes. While the turn runs its
        // events share one save every few seconds: the timer already running writes
        // whatever the thread holds by then, and `finishTurn` saves what is left.
        guard saveTask == nil else { return }
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: SaveCadence.runningInterval)
            guard !Task.isCancelled else { return }
            self?.saveNow()
        }
    }

    func saveNow() {
        // The timer that calls this is finished the moment it runs, so its handle goes
        // whatever happens below. Left standing across a skipped save, `scheduleSave` would
        // see a task already over and return, and nothing would be written until the turn
        // ended: the five-second bound on what a crash costs is exactly this handle.
        saveTask?.cancel()
        saveTask = nil
        // A save before the history is installed would write an empty thread over the file,
        // and so would one over a file that could not be read at all.
        guard persistenceEnabled, !isLoadingHistory, savesDocument else { return }
        // The periodic save fires every few seconds while a turn runs, even when the
        // turn's events changed nothing since the last write: skip the snapshot then.
        // `finishTurn` and friends still land on disk, as their mutations bump the
        // revision before the save below.
        guard saveRevision != lastSavedRevision else { return }
        lastSavedRevision = saveRevision
        DiskWriter.shared.encodeAndWrite(snapshotForPersistence(), to: Storage.threadURL(threadID))
    }

    /// The current document, or nil when the last save already holds it, or when the thread's
    /// file is there unread and must not be written over: quit writes only the threads that
    /// changed, and none of those.
    func unsavedSnapshot() -> ThreadDocument? {
        guard !isLoadingHistory, savesDocument, saveRevision != lastSavedRevision else { return nil }
        return snapshotForPersistence()
    }

    /// Whether dropping this runtime loses nothing the saved document does not hold: no
    /// turn or session, nothing typed or mid-edit, nothing queued or owed to the lead,
    /// no landing under way. Panel geometry and the hand links of old tool rows are UI
    /// conveniences a fresh runtime starts without, as it does after a relaunch. A runtime
    /// whose file was left unread saves nothing, so it holds on to what it has, the notice
    /// saying so among it, until the app quits.
    var holdsOnlyPersistedState: Bool {
        phase == .idle && session == nil && !isReverting && pendingSend == nil && !isLoadingHistory && savesDocument
            && draft.isEmpty && followUpEdit == nil && messageEdit == nil
            && approvals.isEmpty && questions.isEmpty
            && crewPendingReports.isEmpty && crewBatches.isEmpty && crewWaiting.isEmpty && crewNativeHands.isEmpty
            && !crewFlushScheduled && crewLanding == nil && !isCrewMerging && handBudget == nil
            && commandTrees.isEmpty && commandSettles.isEmpty
    }

    func cancelPersistence() {
        persistenceEnabled = false
        saveTask?.cancel()
        saveTask = nil
        flushTask?.cancel()
        flushTask = nil
    }

    func snapshotForPersistence() -> ThreadDocument {
        flushDeltas(scheduleSave: false)
        var document = ThreadDocument(threadID: threadID)
        document.items = entries.map(\.item)
        document.turns = turns
        document.usage = usage
        document.continuation = continuation
        document.sessionLogPending = sessionLogPending
        document.followUps = followUps
        document.crewMerges = crewMerges
        return document
    }
}

private extension TimelineItem.Content {
    /// Appends streamed text to the payload in place, and says whether the payload took it.
    /// Binding the payload out of the enum while the enum still holds it left two
    /// references, so every flush copied the whole message before appending; releasing
    /// the enum's reference first keeps the string unique and the append amortized
    /// O(delta). Written through the entry's `item`, this is still one observed mutation.
    mutating func appendStreamed(_ text: String, kind: ThreadRuntime.DeltaKind) -> Bool {
        switch (kind, self) {
        case (.message, .assistant(var message)):
            self = .todos([])
            message.text += text
            self = .assistant(message)
        case (.reasoning, .reasoning(var block)):
            self = .todos([])
            block.text += text
            self = .reasoning(block)
        case (.toolOutput, .tool(var call)):
            self = .todos([])
            call.appendOutput(text)
            self = .tool(call)
        case (.plan, .plan(var plan)):
            self = .todos([])
            plan.markdown += text
            self = .plan(plan)
        default:
            return false
        }
        return true
    }
}

// MARK: - Rehearsal

extension ThreadRuntime {
    /// Starts a turn with no session of its own behind it: a native Crew hand's, which works
    /// inside its lead's session (see `spawnNativeHand` and `crewAgentEvent`). The brief
    /// lands as the user message, the working line appears, and `rehearse(_:)` then feeds
    /// the events the lead's session reports for the hand.
    func rehearseTurn(_ text: String?) {
        let turnIndex = (turns.map(\.index).max() ?? -1) + 1
        var turn = TurnRecord(index: turnIndex)
        if let text {
            let userItem = TimelineItem(turnID: turn.id, content: .user(UserMessage(text: text)))
            turn.userItemID = userItem.id
            turns.append(turn)
            saveRevision += 1
            append(userItem)
        } else {
            turns.append(turn)
            saveRevision += 1
        }
        currentTurnID = turn.id
        phase = .running
        turnStartedAt = .now
        app?.updateThread(threadID) {
            $0.updatedAt = .now
            $0.lastStatus = .running
        }
    }

    /// The brief a hand was given, once the provider says what it was: a user message at
    /// the top of a timeline that started without one.
    func rehearseBrief(_ text: String) {
        guard let turn = turns.last, turn.userItemID == nil else { return }
        let userItem = TimelineItem(turnID: turn.id, content: .user(UserMessage(text: text)))
        updateTurn(turn.id) { $0.userItemID = userItem.id }
        let entry = TimelineEntry(userItem)
        entries.insert(entry, at: 0)
        entryIndex[userItem.id] = entry
        saveRevision += 1
        scheduleSave()
    }

    /// One provider event, through the same path a live session's events take.
    func rehearse(_ event: ProviderEvent) {
        handle(event)
    }

    /// Tells the diff panel and the turn's cards that the turn's diff changed.
    func noteDiffChanged() {
        diffRevision += 1
    }

    /// A summary computed for a turn recorded before summaries were stored with the turn
    /// (see `TurnFinishedBlock`): kept on the turn-end entry, so the thread's next open reads
    /// it instead of running git for every historical card again.
    func storeLegacyChanges(_ changes: FileChangeSummary, turnID: UUID) {
        guard let entry = entries.last(where: { $0.turnID == turnID && $0.kind == .turnEnd }),
              case .turnEnd(var summary) = entry.item.content, summary.changes == nil else { return }
        summary.changes = changes
        entry.item.content = .turnEnd(summary)
        scheduleSave()
    }
}
