// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The numbers a Crew team runs to: how many rounds of hands one request may send out,
/// how many tasks one delegation block may hold, how many hands Bottega runs at once
/// when nothing caps them, how much a hand it runs gets for one turn, and when the
/// watchdog calls a quiet hand thinking or stalled. Set on Settings › Crew › Limits and
/// handed to Core as a value (`CrewLaunch.limits`, `SessionConfiguration.crewLimits`), so
/// nothing that reads them reaches back into the settings.
///
/// An optional limit is one that can be switched off: nil puts no bound there.
struct CrewLimits: Hashable, Sendable {
    // MARK: Sending hands out

    /// How many times in a row a lead may send hands out for one request of the user's:
    /// one round for the work, one for what failed or what the reports showed was missing.
    /// Past that the lead finishes by itself, so no request can chain hands that verify
    /// hands that verify hands.
    var maxDelegationRounds: Int
    /// How many hands Bottega runs at once for a chat with no cap of its own (neither
    /// its pair's nor the Crew page's). The tasks past it wait their turn
    /// (`ThreadRuntime.spawnWaitingHands`) and go out as hands finish, so a lead may still
    /// ask for as many as the job needs. Twenty-one hands at once each streaming into the
    /// app, each on its own copy of the checkout and its own CLI, kept the main thread and
    /// the GPU busy past the point of answering, and the app had to be force quit: the
    /// range (`CrewPair.maxHandsRange`) stops short of that.
    var uncappedConcurrency: Int
    /// How many tasks one delegation block may send out. Well past what a lead writes in
    /// practice, and the cap still decides how many run at a time; a lead that writes more
    /// hears which ones did not go out.
    var maxDelegatedTasks: Int
    /// How many times one request may tell the lead, each in a turn of its own, that its
    /// block was unreadable or held back. Enough for a lead to fix a malformed block and to
    /// hear once that its rounds are spent; past it a reply that still ends in a block is
    /// treated as a plain reply, so no chat ever loops turn after turn on the same refusal.
    var maxRefusedBlocks: Int

    // MARK: A hand's turn

    /// Tools in a row without a change before a hand Bottega runs is asked to act or
    /// report; nil never asks.
    var pacingTools: Int?
    /// Tools, then time, before the hand is told to wrap up, and before its tools go and it
    /// writes its report; nil puts no bound there. The stop is wide enough for a real piece
    /// of work: a hand cut off in the middle of a refactor costs the lead more than a slow
    /// hand does, and a model on a high effort thinks for minutes between tools.
    var wrapUpTools: Int?
    var maxTools: Int?
    var wrapUpSeconds: TimeInterval?
    var maxSeconds: TimeInterval?
    /// What a stopped hand gets to write its report in.
    var reportSeconds: TimeInterval

    // MARK: The watchdog

    /// How long a hand Bottega runs is quiet (no tool, no word of a reply or its
    /// reasoning) before its row says it is thinking.
    var watchdogThinkingAfter: TimeInterval
    /// How long it is quiet, with no tool since the watchdog's last look, before it is
    /// stopped as stalled; nil never stops one. Quiet means stalled in two cases: the hand
    /// is waiting on an approval or a question that nobody answered, or nothing its tools
    /// did all turn wrote a file (a write only proposed, or refused, does not count). A
    /// hand that has written something and waits on no one is left to think. A read-only
    /// hand never writes, and a long think is not a stall, so the default leaves room for
    /// both. The quiet is counted from the last thing the hand did, and a turn beginning,
    /// a request put to a person and a person's answer each count as that.
    var watchdogStalledAfter: TimeInterval?

    static let `default` = CrewLimits(
        maxDelegationRounds: 2,
        uncappedConcurrency: 4,
        maxDelegatedTasks: 16,
        maxRefusedBlocks: 3,
        pacingTools: 24,
        wrapUpTools: 120,
        maxTools: 160,
        wrapUpSeconds: 25 * 60,
        maxSeconds: 60 * 60,
        reportSeconds: 3 * 60,
        watchdogThinkingAfter: 3 * 60,
        watchdogStalledAfter: 15 * 60
    )

    /// Whether a hand this far into its turn has spent its budget: its tools go and it
    /// writes its report.
    func handBudgetSpent(tools: Int, elapsed: TimeInterval) -> Bool {
        Self.reached(tools, maxTools) || Self.reached(elapsed, maxSeconds)
    }

    /// Whether a hand this far into its turn is told its budget is nearly spent.
    func handShouldWrapUp(tools: Int, elapsed: TimeInterval) -> Bool {
        Self.reached(tools, wrapUpTools) || Self.reached(elapsed, wrapUpSeconds)
    }

    private static func reached<Value: Comparable>(_ value: Value, _ limit: Value?) -> Bool {
        limit.map { value >= $0 } ?? false
    }

    /// The limits kept inside the ranges the app can live with, as a save from an older
    /// build or one edited by hand may not be: no round, no task and no hand at a time
    /// would switch Crew off by stealth, and a budget of seconds would stop every hand
    /// before its first tool.
    func normalized() -> CrewLimits {
        var limits = self
        limits.maxDelegationRounds = Self.clamp(maxDelegationRounds, 1...10)
        limits.uncappedConcurrency = Self.clamp(uncappedConcurrency, CrewPair.maxHandsRange)
        limits.maxDelegatedTasks = Self.clamp(maxDelegatedTasks, 1...64)
        limits.maxRefusedBlocks = Self.clamp(maxRefusedBlocks, 1...10)
        limits.pacingTools = pacingTools.map { Self.clamp($0, 1...1_000) }
        limits.wrapUpTools = wrapUpTools.map { Self.clamp($0, 1...10_000) }
        limits.maxTools = maxTools.map { Self.clamp($0, 1...10_000) }
        limits.wrapUpSeconds = wrapUpSeconds.map { Self.clamp($0, 60...(24 * 60 * 60)) }
        limits.maxSeconds = maxSeconds.map { Self.clamp($0, 60...(24 * 60 * 60)) }
        limits.reportSeconds = Self.clamp(reportSeconds, 30...(60 * 60))
        limits.watchdogThinkingAfter = Self.clamp(watchdogThinkingAfter, 30...(60 * 60))
        limits.watchdogStalledAfter = watchdogStalledAfter.map { Self.clamp($0, 60...(24 * 60 * 60)) }
        return limits
    }

    private static func clamp<Value: Comparable>(_ value: Value, _ range: ClosedRange<Value>) -> Value {
        min(max(value, range.lowerBound), range.upperBound)
    }
}

extension CrewLimits: Codable {
    private enum CodingKeys: String, CodingKey {
        case maxDelegationRounds, uncappedConcurrency, maxDelegatedTasks, maxRefusedBlocks
        case pacingTools, wrapUpTools, maxTools, wrapUpSeconds, maxSeconds, reportSeconds
        case watchdogThinkingAfter, watchdogStalledAfter
    }

    /// Every field falls back to its default on its own, so a save from before a field
    /// existed keeps the rest of what the user chose, and a limit switched off (a null)
    /// reads back as off rather than as its default.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let fallback = Self.default
        self.init(
            maxDelegationRounds: container.value(.maxDelegationRounds, default: fallback.maxDelegationRounds),
            uncappedConcurrency: container.value(.uncappedConcurrency, default: fallback.uncappedConcurrency),
            maxDelegatedTasks: container.value(.maxDelegatedTasks, default: fallback.maxDelegatedTasks),
            maxRefusedBlocks: container.value(.maxRefusedBlocks, default: fallback.maxRefusedBlocks),
            pacingTools: Self.optionalLimit(.pacingTools, in: container, default: fallback.pacingTools),
            wrapUpTools: Self.optionalLimit(.wrapUpTools, in: container, default: fallback.wrapUpTools),
            maxTools: Self.optionalLimit(.maxTools, in: container, default: fallback.maxTools),
            wrapUpSeconds: Self.optionalLimit(.wrapUpSeconds, in: container, default: fallback.wrapUpSeconds),
            maxSeconds: Self.optionalLimit(.maxSeconds, in: container, default: fallback.maxSeconds),
            reportSeconds: container.value(.reportSeconds, default: fallback.reportSeconds),
            watchdogThinkingAfter: container.value(.watchdogThinkingAfter, default: fallback.watchdogThinkingAfter),
            watchdogStalledAfter: Self.optionalLimit(.watchdogStalledAfter, in: container, default: fallback.watchdogStalledAfter)
        )
        self = normalized()
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(maxDelegationRounds, forKey: .maxDelegationRounds)
        try container.encode(uncappedConcurrency, forKey: .uncappedConcurrency)
        try container.encode(maxDelegatedTasks, forKey: .maxDelegatedTasks)
        try container.encode(maxRefusedBlocks, forKey: .maxRefusedBlocks)
        try Self.encodeLimit(pacingTools, .pacingTools, in: &container)
        try Self.encodeLimit(wrapUpTools, .wrapUpTools, in: &container)
        try Self.encodeLimit(maxTools, .maxTools, in: &container)
        try Self.encodeLimit(wrapUpSeconds, .wrapUpSeconds, in: &container)
        try Self.encodeLimit(maxSeconds, .maxSeconds, in: &container)
        try container.encode(reportSeconds, forKey: .reportSeconds)
        try container.encode(watchdogThinkingAfter, forKey: .watchdogThinkingAfter)
        try Self.encodeLimit(watchdogStalledAfter, .watchdogStalledAfter, in: &container)
    }

    /// A limit that can be off: missing (an older save) or unreadable, it takes its
    /// default; a null is the user's own "no limit" and stays so.
    private static func optionalLimit<Value: Decodable>(_ key: CodingKeys, in container: KeyedDecodingContainer<CodingKeys>, default fallback: Value?) -> Value? {
        guard container.contains(key) else { return fallback }
        if (try? container.decodeNil(forKey: key)) == true { return nil }
        return (try? container.decode(Value.self, forKey: key)) ?? fallback
    }

    /// A limit that is off goes out as a null rather than as no key at all, which would
    /// read back as the default.
    private static func encodeLimit<Value: Encodable>(_ value: Value?, _ key: CodingKeys, in container: inout KeyedEncodingContainer<CodingKeys>) throws {
        if let value {
            try container.encode(value, forKey: key)
        } else {
            try container.encodeNil(forKey: key)
        }
    }
}
