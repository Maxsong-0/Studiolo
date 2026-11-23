// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Foundation

/// Picks a chat back up once its provider's usage limit resets. A turn that stops on the
/// limit is noted by the runtime; when the turn then fails, a wait is set for the reset
/// time, and a minute after it the chat is told to continue where it left off. Only
/// with the setting on; hands are left to their leads.
@MainActor
@Observable
final class AutoContinue {
    /// The message sent to pick the task back up. Every provider gets the same one.
    static let continueMessage = "The usage limit has reset. Continue exactly where you left off and finish the task."

    /// How long past the reset time the chat waits, so the limit has really lifted.
    static let margin: TimeInterval = 60

    /// The chats waiting for their limit, and when they go on.
    private(set) var resumesAt: [UUID: Date] = [:]

    /// How a chat's wait ended, once it is over. The notice's row reads this for its words
    /// after the wait; a fresh wait clears it.
    enum LimitOutcome: Equatable, Sendable {
        case cancelled
        case handedOff(UUID)
        case resumed
    }

    private(set) var outcomes: [UUID: LimitOutcome] = [:]

    @ObservationIgnored private weak var app: AppModel?
    /// Limits hit by the running turn: the thread and the reset time, when the provider said.
    @ObservationIgnored private var noted: [UUID: Date?] = [:]
    @ObservationIgnored private var waits: [UUID: Task<Void, Never>] = [:]
    /// Which wait each thread's countdown belongs to. A wait that ends after it was
    /// replaced leaves its successor's countdown and task where they are.
    @ObservationIgnored private var waitTokens: [UUID: UUID] = [:]

    init(app: AppModel) {
        self.app = app
    }

    /// The provider said its usage limit is spent, during a turn of this thread.
    func noteLimit(_ threadID: UUID, resetsAt: Date?) {
        // A later note with a time beats an earlier one without.
        if let existing = noted[threadID], existing != nil, resetsAt == nil { return }
        noted[threadID] = resetsAt
    }

    /// The thread's turn ended. Returns whether the thread now waits for its limit to reset,
    /// in which case the app does not chime or notify about the failure.
    func turnFinished(_ threadID: UUID, status: TurnStatus, continues: Bool) -> Bool {
        guard let note = noted.removeValue(forKey: threadID) else { return false }
        guard let app, app.settings.autoContinueAfterLimit, status == .failed, !continues,
              let thread = app.thread(threadID), !thread.isCrewHand else { return false }
        waits[threadID]?.cancel()
        outcomes[threadID] = nil
        let turnID = app.existingRuntime(for: threadID)?.turns.last?.id
        let token = UUID()
        waitTokens[threadID] = token
        waits[threadID] = Task { [weak self] in
            await self?.wait(threadID, turnID: turnID, resetsAt: note, provider: thread.provider, token: token)
        }
        return true
    }

    /// Cancels a wait, when the user picks the thread up themselves.
    func cancel(_ threadID: UUID) {
        waits[threadID]?.cancel()
        waits[threadID] = nil
        waitTokens[threadID] = nil
        resumesAt[threadID] = nil
        outcomes[threadID] = .cancelled
    }

    /// Cancels the wait and picks the work up in a new chat of the same project, whose
    /// composer holds a hand-off of the turn that stopped. The user chooses the model
    /// there and sends it themselves.
    func handOff(_ threadID: UUID) {
        guard let app, let thread = app.thread(threadID), let runtime = app.existingRuntime(for: threadID) else { return }
        let text = LimitHandoff.text(for: runtime, title: thread.title)
        cancel(threadID)
        guard let project = app.project(thread.projectID), let made = app.newThread(in: project) else { return }
        outcomes[threadID] = .handedOff(made.id)
        app.runtime(for: made.id).draft.text = text
    }

    private func wait(_ threadID: UUID, turnID: UUID?, resetsAt: Date?, provider: ProviderKind, token: UUID) async {
        guard let app else { return }
        var resetsAt = resetsAt
        // Without a time from the turn itself, the account's limits say when the spent window resets.
        if resetsAt == nil { resetsAt = await readResetTime(provider) }
        guard !Task.isCancelled, waitTokens[threadID] == token else { return }
        guard let resetsAt else {
            notice(threadID, .warning, "Usage limit reached, and the provider did not say when it resets. Send a message to continue.")
            notify(threadID, "Usage limit reached. Send a message to continue.")
            waits[threadID] = nil
            waitTokens[threadID] = nil
            return
        }
        let resumeAt = max(resetsAt.addingTimeInterval(Self.margin), Date.now.addingTimeInterval(5))
        resumesAt[threadID] = resumeAt
        let when = Self.format(resumeAt)
        notice(threadID, .info, "Usage limit reached. Continuing automatically at \(when).", limitMark: true)
        notify(threadID, "Usage limit reached. Continuing at \(when).")
        try? await Task.sleep(for: .seconds(resumeAt.timeIntervalSinceNow))
        // A wait that was cancelled or replaced while it slept leaves the countdown and
        // the task alone: they belong to whatever took its place.
        guard !Task.isCancelled, waitTokens[threadID] == token else { return }
        resumesAt[threadID] = nil
        waits[threadID] = nil
        waitTokens[threadID] = nil
        guard app.settings.autoContinueAfterLimit,
              let runtime = app.existingRuntime(for: threadID), !runtime.isRunning,
              // The user has since picked the chat up themselves: nothing to continue.
              runtime.turns.last?.id == turnID else { return }
        outcomes[threadID] = .resumed
        runtime.enqueueFollowUp(text: Self.continueMessage, attachments: [])
        if let prompt = runtime.followUps.last(where: { $0.text == Self.continueMessage }) {
            runtime.sendFollowUpNow(prompt.id)
        }
    }

    /// The reset time of the account's most-spent window, read fresh.
    private func readResetTime(_ provider: ProviderKind) async -> Date? {
        guard let app, PlanLimitsReader.exposesLimits(provider) else { return nil }
        let limits: PlanLimits?
        if provider.isAPIKeyBased {
            let apiKey = app.settings.apiKey(for: provider)
            guard !apiKey.isEmpty else { return nil }
            limits = await PlanLimitsReader.read(provider, apiKey: apiKey)
        } else {
            guard let executable = app.providers.executable(for: provider) else { return nil }
            limits = await PlanLimitsReader.read(provider, executable: executable, environment: app.providers.environment(for: provider))
        }
        var spent: PlanLimits.Window?
        for window in limits?.windows ?? [] {
            guard let resetsAt = window.resetsAt, resetsAt > Date.now else { continue }
            if spent == nil || window.percent > spent!.percent { spent = window }
        }
        guard let spent, spent.percent >= 95 else { return nil }
        return spent.resetsAt
    }

    private func notice(_ threadID: UUID, _ level: Notice.Level, _ message: String, limitMark: Bool = false) {
        var notice = Notice(level: level, message: message)
        if limitMark { notice.limit = LimitNotice(threadID: threadID) }
        app?.existingRuntime(for: threadID)?.rehearse(.notice(notice))
    }

    private func notify(_ threadID: UUID, _ body: String) {
        guard let app, app.settings.notifyWhenFinished, let thread = app.thread(threadID) else { return }
        let onScreen = NSApp.isActive && app.selectedThreadID == threadID
        guard !onScreen else { return }
        app.notify(threadID: threadID, title: thread.title, body: body)
    }

    /// The reset time as the notices print it; the formatter lives with the signal so provider sessions can use it too.
    static func format(_ date: Date) -> String { UsageLimitSignal.format(date) }
}
