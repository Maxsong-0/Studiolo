import AppKit
import Foundation

/// A provider saying the account's usage limit is spent, and when it comes back.
enum UsageLimitSignal {
    /// Whether an error message is the provider refusing to work until a usage limit resets.
    static func matches(_ text: String) -> Bool {
        let lower = text.lowercased()
        return lower.contains("usage limit") || lower.contains("hit your limit") || lower.contains("usage_limit")
            || lower.contains("limit reached") || lower.contains("out of extra usage")
    }

    /// The reset time an error message carries, when it does: a Unix timestamp after a bar
    /// ("usage limit reached|1757865600"), or a clock time such as "resets 3pm" or "resets at
    /// 3:30 pm", taken as the next such time from now.
    static func resetTime(in text: String, now: Date = .now) -> Date? {
        if let match = text.range(of: #"\|\s*(\d{9,})"#, options: .regularExpression) {
            let digits = text[match].drop { !$0.isNumber }
            if let seconds = TimeInterval(digits) {
                // Some providers write milliseconds.
                return Date(timeIntervalSince1970: seconds > 1e11 ? seconds / 1_000 : seconds)
            }
        }
        if let match = text.range(of: #"(?i)resets?(?:\s+at)?\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)"#, options: .regularExpression) {
            let clock = String(text[match])
            let numbers = clock.components(separatedBy: CharacterSet.decimalDigits.inverted).filter { !$0.isEmpty }
            guard let hourText = numbers.first, var hour = Int(hourText) else { return nil }
            let minute = numbers.count > 1 ? Int(numbers[1]) ?? 0 : 0
            let isPM = clock.lowercased().hasSuffix("pm")
            if isPM, hour < 12 { hour += 12 }
            if !isPM, hour == 12 { hour = 0 }
            var calendar = Calendar.current
            if let zone = text.range(of: #"\(([A-Za-z_]+/[A-Za-z_]+)\)"#, options: .regularExpression),
               let timeZone = TimeZone(identifier: text[zone].trimmingCharacters(in: CharacterSet(charactersIn: "()"))) {
                calendar.timeZone = timeZone
            }
            let components = DateComponents(hour: hour, minute: minute)
            return calendar.nextDate(after: now, matching: components, matchingPolicy: .nextTime)
        }
        return nil
    }
}

/// Picks a chat back up once its provider's usage limit resets. A turn that stops on the
/// limit is noted by the runtime; when the turn then fails, a wait is set for the reset
/// time, and a minute after it the chat is told to continue where it left off. Only
/// with the setting on; heads are left to their leads.
@MainActor
@Observable
final class AutoContinue {
    /// The message sent to pick the task back up. Every provider gets the same one.
    static let continueMessage = "The usage limit has reset. Continue exactly where you left off and finish the task."

    /// How long past the reset time the chat waits, so the limit has really lifted.
    static let margin: TimeInterval = 60

    /// The chats waiting for their limit, and when they go on.
    private(set) var resumesAt: [UUID: Date] = [:]

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
              let thread = app.thread(threadID), !thread.isHydraHead else { return false }
        waits[threadID]?.cancel()
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
        notice(threadID, .info, "Usage limit reached. Continuing automatically at \(when).")
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

    private func notice(_ threadID: UUID, _ level: Notice.Level, _ message: String) {
        app?.existingRuntime(for: threadID)?.rehearse(.notice(Notice(level: level, message: message)))
    }

    private func notify(_ threadID: UUID, _ body: String) {
        guard let app, app.settings.notifyWhenFinished, let thread = app.thread(threadID) else { return }
        let onScreen = NSApp.isActive && app.selectedThreadID == threadID
        guard !onScreen else { return }
        app.notify(threadID: threadID, title: thread.title, body: body)
    }

    /// "3:31 PM" today, "tomorrow at 9:00 AM" or "Thursday at 9:00 AM" further out.
    static func format(_ date: Date) -> String {
        let calendar = Calendar.current
        let time = date.formatted(date: .omitted, time: .shortened)
        if calendar.isDateInToday(date) { return time }
        if calendar.isDateInTomorrow(date) { return "tomorrow at \(time)" }
        if let week = calendar.date(byAdding: .day, value: 6, to: .now), date < week {
            return "\(date.formatted(.dateTime.weekday(.wide))) at \(time)"
        }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}
