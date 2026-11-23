// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Observation

/// Converts a cumulative token counter into per-event spend.
///
/// Providers report usage with different shapes: Claude hands over session
/// totals, Codex a thread total, ACP a session counter. All three only ever
/// grow within a session, so the spend behind one event is the positive
/// delta since the previous event. A drop (compaction, a fresh session on a
/// reused object) reseeds the baseline without recording: the growth after
/// it is new spend and gets counted from there. The first call may pass
/// `before`, the total the counter had before this session, which is not
/// new spend.
struct TokenSpendTracker {
    private var lastTotal = 0
    private var seeded = false

    mutating func spend(total: Int, before: Int = 0) -> Int {
        if !seeded { seeded = true; lastTotal = max(0, min(total, before)) }
        defer { lastTotal = max(0, total) }
        return max(0, total - lastTotal)
    }

    /// Sees a total without counting it, seeding the baseline for the deltas
    /// that follow. For a resumed session whose first reported total is the
    /// conversation's inherited history rather than spend: the baseline is
    /// unknown, so the observation is left uncounted instead of being charged
    /// or reported as zero.
    mutating func seedBaseline(total: Int) {
        guard !seeded else { return }
        seeded = true
        lastTotal = max(0, total)
    }
}

/// Daily AI coding token spend across every provider, backing the Token
/// activity heatmap at the top of Settings, General.
///
/// Two feeds merge here. Live spend is recorded as sessions stream: each
/// provider adapter reports with the semantics it knows (per-response
/// totals for the native API sessions, tracker deltas for the cumulative
/// counters), so the ledger itself only ever adds. History comes from a
/// one-time background tail scan of local Codex rollouts: only the last
/// 128 KB of each file is read, looking for its final token_count event,
/// each file counting only its own growth, since a resumed thread carries
/// its history into a new file, so thousands of transcripts cost one seek each. Per-file fingerprints
/// (mtime plus size) keep later scans incremental. Files written after
/// launch are skipped because live recording already owns them; counting
/// both would double every active session.
///
/// Codex is the one provider whose usage is both observed live and read back
/// from disk, so it needs provenance. Live Codex spend is recorded under the
/// rollout identity — the Codex thread (or hand) id, which is the session id
/// its rollout file is named for — and the spend of earlier processes is
/// reconciled against the scan by subtracting it from that identity's own
/// growth. That survives restarts, resumed threads (a new file sharing the
/// session id) and repeated scans: the subtraction is recomputed from the
/// fingerprints every time, never accumulated. Claude history is live-only for
/// now: its usage rows are scattered through gigabytes of JSONL, which is
/// not a Settings-open price worth paying.
@MainActor
@Observable
final class TokenLedger {
    static let shared = TokenLedger()

    /// Merged history plus live spend, keyed by local day.
    private(set) var dailyTotals: [String: Int] = [:]

    /// Live spend recorded before the storage was versioned: per-day totals with
    /// every provider mixed together, kept readable and counted, and marked as
    /// uncertain because a day's source can no longer be reconstructed. Spend
    /// recorded since carries provenance and is not in here.
    private(set) var legacyDaily: [String: Int] = [:]

    /// How many of the tokens the grid draws for the current year rest on the
    /// pre-provenance totals above. Days from before that year cannot change
    /// what the grid shows, so they are not counted here.
    var uncertainTokensThisYear: Int {
        let year = String(format: "%04d-", Calendar.current.component(.year, from: Date()))
        return legacyDaily.reduce(0) { $0 + ($1.key.hasPrefix(year) ? $1.value : 0) }
    }

    /// Where v1 kept live spend: one mixed-provider total per day, in the
    /// preferences plist. Read once into `legacyDaily` and left on disk as it was.
    private static let legacyLiveDefaultsKey = "bottega.tokenActivity.liveDaily"
    private static let defaults: UserDefaults = .standard

    /// Live spend, its Codex provenance and the legacy values, persisted whole.
    private var state = LiveSpendState()
    private var historyDaily: [String: Int] = [:]
    private var didStartLoad = false
    /// The pending write of the live spend, if one is due.
    @ObservationIgnored private var liveSave: Task<Void, Never>?
    /// Live recording owns every session from this moment, so the history
    /// scan only takes files last written before it.
    private let launchDate = Date()
    /// What went wrong reading the live spend's file, if anything did (see `StoredFile`): it
    /// was kept aside and the ledger started over, or it is still there unread and the spend
    /// recorded this run stays in memory rather than being written over it.
    let loadProblem: StorageProblem?

    private init() {
        let loaded = LiveSpendStateStore.load(defaults: Self.defaults)
        loadProblem = loaded.problem
        state = loaded.state
        legacyDaily = state.legacyDaily
        dailyTotals = mergedTotals()
    }

    /// Adds spend to a day. Every provider session runs on the main actor,
    /// so this needs no lock. Not for Codex, whose usage the history scan also
    /// reads back from its rollouts: it goes through
    /// `record(codexSpend:rolloutID:on:)`, or it is counted twice.
    func record(spend: Int, on date: Date = Date()) {
        record(spend: spend, codexRollout: nil, on: date)
    }

    /// Adds spend reported as a `TokenSpend`. Codex callers must use
    /// `record(codexSpend:rolloutID:on:)` instead: without the rollout identity
    /// the scan cannot tell live usage from the rollout it will read back.
    func record(_ spend: TokenSpend, on date: Date = Date()) {
        record(spend: spend.totalTokens, codexRollout: nil, on: date)
    }

    /// Adds live Codex spend under the rollout identity it came from, so the
    /// history scan subtracts it from that rollout instead of counting the same
    /// usage a second time. `rolloutID` is the Codex thread or hand id, which is
    /// the session id the rollout file is named for.
    func record(codexSpend spend: Int, rolloutID: String, on date: Date = Date()) {
        // Without identity, only the rollout scan can own the daily contribution.
        guard !rolloutID.isEmpty else { return }
        record(spend: spend, codexRollout: rolloutID.lowercased(), on: date)
    }

    private func record(spend: Int, codexRollout: String?, on date: Date) {
        guard spend > 0 else { return }
        let day = Self.dayKey(for: date)
        state.daily[day, default: 0] += spend
        dailyTotals[day, default: 0] += spend
        if let codexRollout {
            state.liveCodex[codexRollout, default: 0] += spend
        }
        // Usage arrives many times a turn from every running session: the write, a
        // JSON encode and a file write, waits for a lull and lands whole. None while the
        // file is there unread: this run's spend would take the place of what it holds.
        guard liveSave == nil, loadProblem?.blocksSaving != true else { return }
        liveSave = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard let self, !Task.isCancelled else { return }
            liveSave = nil
            LiveSpendStateStore.save(state)
        }
    }

    /// Writes the live spend now, for the app quitting with a write still due.
    func flushLiveSpend() {
        guard let liveSave else { return }
        liveSave.cancel()
        self.liveSave = nil
        LiveSpendStateStore.save(state)
    }

    /// Runs the Codex history scan once per process. Later calls are no-ops;
    /// live recording keeps the grid fresh after that.
    func refreshIfNeeded() async {
        guard !didStartLoad else { return }
        didStartLoad = true
        let liveCutoff = launchDate
        // The live Codex spend of earlier processes, per rollout identity: their
        // rollouts are on disk and would otherwise be counted a second time.
        // This process's own live spend is not in here — its rollouts are still
        // being written, so the scan skips them either way.
        let settled = state.settledCodex
        // Reading, scanning and writing all happen off the main actor: the fingerprint
        // file grows with the number of rollouts, and none of it is wanted on the thread
        // that is drawing Settings.
        let history = await Task.detached(priority: .utility) {
            let result = TokenHistoryScanner.scanCodexHistory(
                cached: CodexFingerprintStore.load(),
                liveCutoff: liveCutoff,
                settledLive: settled
            )
            CodexFingerprintStore.save(result.fingerprints)
            return result
        }.value
        historyDaily = history.daily
        // Keep provenance even when a rollout is temporarily skipped because it is
        // active, moved, or unavailable. Its daily live total remains recorded.
        dailyTotals = mergedTotals()
    }

    private func mergedTotals() -> [String: Int] {
        var totals = state.legacyDaily
        for (day, tokens) in state.daily { totals[day, default: 0] += tokens }
        for (day, tokens) in historyDaily { totals[day, default: 0] += tokens }
        return totals
    }

    /// Local day key for a date. Nonisolated so the background scan shares it.
    nonisolated static func dayKey(for date: Date) -> String {
        let parts = Calendar.current.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    }
}

/// One Codex rollout's contribution: its session total attributed to the day
/// of its last token_count event. Cached by path so rescans only tail files
/// whose mtime or size moved.
private struct CodexFileFingerprint: Sendable, Codable {
    var mtime: Double
    var size: UInt64
    var tokens: Int
    var day: String
    /// The rollout's session id, which a resumed thread keeps across the files
    /// it is resumed in. Empty in records written before it was kept; the path
    /// supplies it on read.
    var identity: String = ""

    init(mtime: Double, size: UInt64, tokens: Int, day: String, identity: String) {
        self.mtime = mtime
        self.size = size
        self.tokens = tokens
        self.day = day
        self.identity = identity
    }

    init?(dict: [String: Any], path: String) {
        guard let mtime = dict["mtime"] as? Double,
              let day = dict["day"] as? String else { return nil }
        let size: UInt64
        if let number = dict["size"] as? NSNumber {
            size = number.uint64Value
        } else if let int = dict["size"] as? Int {
            size = UInt64(int)
        } else {
            return nil
        }
        let tokens = (dict["tokens"] as? NSNumber)?.intValue ?? (dict["tokens"] as? Int ?? 0)
        let identity = dict["identity"] as? String ?? ""
        self.init(
            mtime: mtime,
            size: size,
            tokens: tokens,
            day: day,
            identity: identity.isEmpty ? Self.rolloutIdentity(path: path) : identity
        )
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        mtime = try container.decode(Double.self, forKey: .mtime)
        size = try container.decode(UInt64.self, forKey: .size)
        tokens = container.value(.tokens, default: 0)
        day = container.value(.day, default: "")
        identity = container.value(.identity, default: "")
    }

    /// The session a rollout belongs to, taken from its name
    /// (`rollout-<timestamp>-<session id>.jsonl`). Codex writes a fresh file
    /// each time a thread is resumed and keeps the session id in it, which is
    /// what lets one thread's files reconcile with the live spend recorded for
    /// it. A name with no session id falls back to the path, which reconciles
    /// only with itself.
    static func rolloutIdentity(path: String) -> String {
        let name = (path as NSString).lastPathComponent
        let pattern = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
        guard let range = name.range(of: pattern, options: [.regularExpression, .backwards]) else { return path }
        return String(name[range]).lowercased()
    }
}

/// Where the fingerprints live. There is one per Codex rollout and a busy machine has
/// thousands of them, which has no business in the preferences plist: that file is read
/// whole into every process that reads a default and rewritten whole on every write, so a
/// few megabytes of fingerprints slow down every setting the app touches. They sit in
/// their own JSON file in Application Support instead, beside the library.
private enum CodexFingerprintStore {
    /// Where earlier builds kept them. Read once, then cleared.
    private static let legacyDefaultsKey = "bottega.tokenActivity.codexFiles"

    private static var fileURL: URL {
        Storage.root.appendingPathComponent("token-activity-codex-v3.json")
    }

    static func load() -> [String: CodexFileFingerprint] {
        // The first shape counted a resumed thread's history once per file it
        // was resumed in; the v2 file holds each file's own growth, so the
        // old one is not migrated.
        let oldURL = Storage.root.appendingPathComponent("token-activity-codex.json")
        if FileManager.default.fileExists(atPath: oldURL.path) {
            try? FileManager.default.removeItem(at: oldURL)
        }
        if let data = try? Data(contentsOf: fileURL),
           let stored = try? JSONDecoder().decode([String: CodexFileFingerprint].self, from: data) {
            return stored
        }
        // v2 kept each file's own growth but not the rollout identity, which v3
        // needs to reconcile live spend; the session id comes from the path, so
        // the record carries over whole.
        let v2URL = Storage.root.appendingPathComponent("token-activity-codex-v2.json")
        if let data = try? Data(contentsOf: v2URL),
           let stored = try? JSONDecoder().decode([String: CodexFileFingerprint].self, from: data) {
            var rebuilt: [String: CodexFileFingerprint] = [:]
            for (path, fingerprint) in stored {
                var migrated = fingerprint
                if migrated.identity.isEmpty { migrated.identity = CodexFileFingerprint.rolloutIdentity(path: path) }
                rebuilt[path] = migrated
            }
            try? FileManager.default.removeItem(at: v2URL)
            save(rebuilt)
            return rebuilt
        }
        guard let legacy = UserDefaults.standard.dictionary(forKey: legacyDefaultsKey) as? [String: [String: Any]] else {
            return [:]
        }
        UserDefaults.standard.removeObject(forKey: legacyDefaultsKey)
        let migrated = legacy.reduce(into: [String: CodexFileFingerprint]()) { result, entry in
            if let fingerprint = CodexFileFingerprint(dict: entry.value, path: entry.key) { result[entry.key] = fingerprint }
        }
        save(migrated)
        return migrated
    }

    static func save(_ fingerprints: [String: CodexFileFingerprint]) {
        guard let data = try? JSONEncoder().encode(fingerprints) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}

/// Live spend, versioned.
///
/// v1 kept one mixed-provider total per local day in the preferences plist.
/// That cannot be reconciled against the rollout scan — it says nothing about
/// which rollout, or even which provider, the tokens came from — so v2 keeps
/// the per-day totals and the Codex provenance beside a copy of the v1 values.
/// The v1 key is left on disk untouched and its days stay visible, marked
/// uncertain, rather than being dropped or passed off as reconstructible.
private struct LiveSpendState: Codable, Sendable {
    /// Live spend per local day across every provider.
    var daily: [String: Int] = [:]
    /// Live Codex spend recorded by earlier processes, per rollout identity.
    /// Subtracted from that rollout's own growth when history is scanned.
    var settledCodex: [String: Int] = [:]
    /// Live Codex spend recorded by this process, per rollout identity. Rolled
    /// into `settledCodex` when the next process loads the file: until then the
    /// rollouts holding it are still being written and the scan skips them.
    var liveCodex: [String: Int] = [:]
    /// The pre-provenance per-day values, as they were recorded.
    var legacyDaily: [String: Int] = [:]

    init() {}

    /// A key missing (an older file) or null reads as empty. One that is there and is not
    /// the table it should be makes the whole file unreadable, so it is kept aside rather
    /// than read as empty and saved over (see `StoredFile`).
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        daily = try container.decodeIfPresent([String: Int].self, forKey: .daily) ?? [:]
        settledCodex = try container.decodeIfPresent([String: Int].self, forKey: .settledCodex) ?? [:]
        liveCodex = try container.decodeIfPresent([String: Int].self, forKey: .liveCodex) ?? [:]
        legacyDaily = try container.decodeIfPresent([String: Int].self, forKey: .legacyDaily) ?? [:]
    }
}

private enum LiveSpendStateStore {
    private static var fileURL: URL {
        Storage.root.appendingPathComponent("token-activity-live-v2.json")
    }

    /// The live spend to start from, and what went wrong reading its file: a file that did
    /// not read is not taken for no spend at all (see `StoredFile`). Without a file's spend,
    /// the v1 totals are read again, as a first launch reads them.
    static func load(defaults: UserDefaults) -> (state: LiveSpendState, problem: StorageProblem?) {
        let read = StoredFile.read(fileURL, subject: "The token activity ledger") { data in
            try JSONDecoder().decode(LiveSpendState.self, from: data)
        }
        var state: LiveSpendState
        if let stored = read.value {
            state = stored
        } else {
            state = LiveSpendState()
            // The v1 per-day totals, read once. They stay on disk as they were.
            state.legacyDaily = defaults.dictionary(forKey: "bottega.tokenActivity.liveDaily") as? [String: Int] ?? [:]
        }
        // The process that recorded `liveCodex` is gone: its Codex spend is
        // settled now, and the rollouts holding it are files on disk that the
        // next scan will count.
        for (identity, tokens) in state.liveCodex {
            state.settledCodex[identity, default: 0] += tokens
        }
        state.liveCodex = [:]
        return (state, read.problem)
    }

    static func save(_ state: LiveSpendState) {
        guard let data = try? JSONEncoder().encode(state) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}

/// Background Codex history scan. Static functions over value types only, so
/// the detached task stays Sendable.
private enum TokenHistoryScanner {
    struct Result: Sendable {
        var daily: [String: Int]
        var fingerprints: [String: CodexFileFingerprint]
        /// Every rollout identity the scan touched, for pruning settled live
        /// spend whose rollouts are gone.
        var identities: Set<String>
    }

    /// How much of each rollout tail to read. token_count events repeat
    /// through a session, so the final one sits near the end.
    private static let tailBytes = 131_072
    private static let historyMonths = 12
    static func scanCodexHistory(
        cached: [String: CodexFileFingerprint],
        liveCutoff: Date,
        settledLive: [String: Int]
    ) -> Result {
        var fingerprints = cached
        var seen: Set<String> = []
        var identities: Set<String> = []
        // Each rollout's own growth, kept per identity rather than summed at
        // once: the live spend an earlier process recorded for a thread has to
        // come off that thread's growth, not off some other thread's.
        var contributions: [String: [(day: String, tokens: Int)]] = [:]
        let cutoff = Calendar.current.date(byAdding: .month, value: -historyMonths, to: Date()) ?? Date.distantPast
        let manager = FileManager()
        for directory in codexDirectories {
            guard let enumerator = manager.enumerator(
                at: directory,
                includingPropertiesForKeys: [.contentModificationDateKey, .fileSizeKey],
                options: [.skipsHiddenFiles]
            ) else { continue }
            for case let url as URL in enumerator {
                guard url.pathExtension == "jsonl" else { continue }
                guard let values = try? url.resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey]),
                      let mtime = values.contentModificationDate,
                      mtime >= cutoff,
                      // Sessions written after launch are live-tracked event
                      // by event; taking their session total here too would
                      // count every token twice.
                      mtime < liveCutoff else { continue }
                let size = UInt64(values.fileSize ?? 0)
                let path = url.path
                seen.insert(path)
                if let known = fingerprints[path], known.mtime == mtime.timeIntervalSince1970, known.size == size {
                    let identity = known.identity.isEmpty
                        ? CodexFileFingerprint.rolloutIdentity(path: path)
                        : known.identity
                    identities.insert(identity)
                    if known.tokens > 0 { contributions[identity, default: []].append((known.day, known.tokens)) }
                    continue
                }
                let parsed = tailContribution(at: url)
                let identity = CodexFileFingerprint.rolloutIdentity(path: path)
                let fingerprint = CodexFileFingerprint(
                    mtime: mtime.timeIntervalSince1970,
                    size: size,
                    tokens: parsed?.tokens ?? 0,
                    day: parsed?.day ?? "",
                    identity: identity
                )
                fingerprints[path] = fingerprint
                identities.insert(identity)
                if let parsed, parsed.tokens > 0 { contributions[identity, default: []].append((parsed.day, parsed.tokens)) }
            }
        }
        // Files deleted since the last scan stop contributing instead of
        // lingering in the cache forever.
        fingerprints = fingerprints.filter { seen.contains($0.key) }
        return Result(
            daily: reconcile(contributions, settledLive: settledLive),
            fingerprints: fingerprints,
            identities: identities
        )
    }

    /// Folds each thread's rollouts into the day buckets, after removing the live
    /// spend an earlier process already recorded for that thread. Recomputed from
    /// the fingerprints on every scan, so a scan repeated in the same process, or
    /// after a restart, lands the same numbers instead of accumulating.
    private static func reconcile(
        _ contributions: [String: [(day: String, tokens: Int)]],
        settledLive: [String: Int]
    ) -> [String: Int] {
        var daily: [String: Int] = [:]
        for (identity, files) in contributions {
            var remaining = max(0, settledLive[identity] ?? 0)
            // Newest first: the live spend happened after the history the older
            // files carry, so it comes off the most recent day's growth. A live
            // amount larger than the scanned growth leaves the rest unclaimed
            // rather than going negative.
            for file in files.sorted(by: { $0.day > $1.day }) {
                guard file.tokens > 0 else { continue }
                let cleared = min(remaining, file.tokens)
                remaining -= cleared
                let kept = file.tokens - cleared
                if kept > 0 { daily[file.day, default: 0] += kept }
            }
        }
        return daily
    }

    private static var codexDirectories: [URL] {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return [
            home.appendingPathComponent(".codex/sessions", isDirectory: true),
            home.appendingPathComponent(".codex/archived_sessions", isDirectory: true),
        ]
    }

    private static let headBytes = 8 * 1_048_576

    /// The total the file inherited from the thread it resumed, which is the
    /// total the first token_count already stood at before its own request.
    private static func inheritedTotal(at url: URL) -> Int {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return 0 }
        defer { try? handle.close() }
        let chunkSize = 512 * 1024
        let marker = Data("\"token_count\"".utf8)
        var budget = headBytes
        var buffer = Data()
        while budget > 0 {
            let want = min(chunkSize, budget)
            guard let chunk = try? handle.read(upToCount: want), !chunk.isEmpty else { break }
            budget -= chunk.count
            buffer.append(chunk)
            // One pass over the chunk: each line is a slice between newlines, and only a
            // line carrying the marker is decoded. A resumed file replays its whole
            // history before its first token_count, so the head can run to megabytes.
            var start = buffer.startIndex
            while let newline = buffer[start...].firstIndex(of: 0x0A) {
                let lineData = buffer[start..<newline]
                start = buffer.index(after: newline)
                guard lineData.range(of: marker) != nil,
                      let event = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any],
                      let payload = event["payload"] as? [String: Any],
                      let info = payload["info"] as? [String: Any],
                      let total = info["total_token_usage"] as? [String: Any],
                      let tokens = (total["total_tokens"] as? NSNumber)?.intValue ?? total["total_tokens"] as? Int,
                      tokens > 0 else { continue }
                let last: Int
                if let lastUsage = info["last_token_usage"] as? [String: Any] {
                    last = (lastUsage["total_tokens"] as? NSNumber)?.intValue ?? lastUsage["total_tokens"] as? Int ?? 0
                } else {
                    last = 0
                }
                return max(0, tokens - last)
            }
            buffer = Data(buffer[start...])
            if chunk.count < want { break }
        }
        return 0
    }

    /// Reads the tail of one rollout and returns the file's own growth, its
    /// final total less what it inherited from a resumed thread, with the
    /// event day, or nil when the tail holds no token_count event.
    private static func tailContribution(at url: URL) -> (tokens: Int, day: String)? {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? handle.close() }
        let fileSize = (try? handle.seekToEnd()) ?? 0
        guard fileSize > 0 else { return nil }
        let offset = fileSize > UInt64(tailBytes) ? fileSize - UInt64(tailBytes) : 0
        do {
            try handle.seek(toOffset: offset)
        } catch {
            return nil
        }
        let data = (try? handle.readToEnd()) ?? Data()
        guard !data.isEmpty else { return nil }
        // Lossy on purpose: one stray byte must not throw away the whole
        // tail when the token_count line itself is intact.
        let text = String(decoding: data, as: UTF8.self)
        for line in text.split(separator: "\n").reversed() {
            guard line.contains("\"token_count\"") else { continue }
            guard let lineData = line.data(using: .utf8),
                  let event = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any],
                  let payload = event["payload"] as? [String: Any],
                  let info = payload["info"] as? [String: Any],
                  let total = info["total_token_usage"] as? [String: Any],
                  let tokens = (total["total_tokens"] as? NSNumber)?.intValue ?? total["total_tokens"] as? Int,
                  tokens > 0 else { continue }
            let day: String
            if let stamp = event["timestamp"] as? String, let date = Self.eventDate(from: stamp) {
                day = TokenLedger.dayKey(for: date)
            } else {
                day = ""
            }
            guard !day.isEmpty else { continue }
            let own = tokens - inheritedTotal(at: url)
            return (max(0, own), day)
        }
        return nil
    }

    /// Parses both fractional and whole-second event stamps. One formatter
    /// with fractional seconds required rejects the other shape, so both
    /// are tried rather than dropping events. Built per call because
    /// ISO8601DateFormatter is not Sendable; only token_count lines reach
    /// here, so the cost is negligible.
    private static func eventDate(from stamp: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: stamp) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: stamp)
    }
}

/// Which scaling the heatmap colors use.
enum TokenActivityMode: String, Sendable, CaseIterable {
    case daily
    case weekly
    case cumulative

    var title: String {
        switch self {
        case .daily: "Daily"
        case .weekly: "Weekly"
        case .cumulative: "Cumulative"
        }
    }
}

/// Contribution grid over the current year, January to December, Sunday rows.
/// Columns are week buckets starting on the Sunday on or before New Year's
/// Day; every column keeps all seven rows so the grid never shears mid-month.
/// Days still to come stay in the grid as faint cells, so the year keeps its
/// shape and the months read Jan through Dec whatever today's date.
struct TokenActivityGrid {
    /// One column of the grid: the day in each Sunday-first row, nil where
    /// the cell falls outside the year.
    var columns: [[Date?]]
    /// Display value per cell, already scaled for the mode.
    var values: [[Int]]
    var maxValue: Int
    var monthTicks: [(label: String, column: Int)]
    /// The first day after today, so the canvas can tell a quiet day from one not yet here.
    var tomorrow: Date

    static let rows = 7
    static let months = 12

    /// Fixed English abbreviations: the calendar's own symbols come out as
    /// "M01" without a locale, and the rest of the app reads in English.
    private static let monthNames = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]

    static func build(daily: [String: Int], mode: TokenActivityMode, now: Date = Date()) -> TokenActivityGrid {
        var calendar = Calendar(identifier: .gregorian)
        calendar.firstWeekday = 1
        let startOfToday = calendar.startOfDay(for: now)
        let rangeStart = calendar.date(from: calendar.dateComponents([.year], from: startOfToday)) ?? startOfToday
        let rangeEnd = calendar.date(byAdding: DateComponents(year: 1, day: -1), to: rangeStart) ?? startOfToday
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: startOfToday) ?? startOfToday

        // First column starts on the Sunday on or before the range start.
        let leadingGap = (calendar.component(.weekday, from: rangeStart) - calendar.firstWeekday + 7) % 7
        let gridStart = calendar.date(byAdding: .day, value: -leadingGap, to: rangeStart) ?? rangeStart

        // Day totals inside the range, keyed by day start.
        var totalsByDay: [Date: Int] = [:]
        var cursor = rangeStart
        while cursor <= startOfToday {
            let key = TokenLedger.dayKey(for: cursor)
            if let total = daily[key] { totalsByDay[calendar.startOfDay(for: cursor)] = total }
            guard let next = calendar.date(byAdding: .day, value: 1, to: cursor) else { break }
            cursor = next
        }

        // Pack days into Sunday-first columns through the week that holds
        // New Year's Eve.
        var columns: [[Date?]] = []
        var day = gridStart
        while true {
            var column: [Date?] = []
            for _ in 0..<rows {
                column.append(day < rangeStart || day > rangeEnd ? nil : day)
                guard let next = calendar.date(byAdding: .day, value: 1, to: day) else { break }
                day = next
            }
            columns.append(column)
            if day > rangeEnd { break }
        }

        // Per-mode display values. The shape never changes, only the value
        // each cell is colored by.
        var values = Array(repeating: Array(repeating: 0, count: rows), count: columns.count)
        switch mode {
        case .daily:
            for c in columns.indices {
                for r in 0..<rows {
                    if let date = columns[c][r] { values[c][r] = totalsByDay[date] ?? 0 }
                }
            }
        case .weekly:
            for c in columns.indices {
                var week = 0
                for r in 0..<rows {
                    if let date = columns[c][r] { week += totalsByDay[date] ?? 0 }
                }
                for r in 0..<rows { values[c][r] = columns[c][r] == nil ? 0 : week }
            }
        case .cumulative:
            var running = 0
            for c in columns.indices {
                for r in 0..<rows {
                    if let date = columns[c][r] { running += totalsByDay[date] ?? 0 }
                    values[c][r] = running
                }
            }
        }
        let maxValue = values.flatMap { $0 }.max() ?? 0

        // One tick per month at the column holding its first day.
        var monthTicks: [(label: String, column: Int)] = []
        for offset in 0..<months {
            guard let monthDate = calendar.date(byAdding: .month, value: offset, to: rangeStart) else { continue }
            let monthStart = calendar.date(from: calendar.dateComponents([.year, .month], from: monthDate)) ?? monthDate
            let clamped = max(monthStart, rangeStart)
            let daysOut = calendar.dateComponents([.day], from: gridStart, to: clamped).day ?? 0
            let column = daysOut / rows
            let label = monthNames[(calendar.component(.month, from: monthDate) - 1 + 12) % 12]
            if column < columns.count { monthTicks.append((label, column)) }
        }

        return TokenActivityGrid(columns: columns, values: values, maxValue: maxValue, monthTicks: monthTicks, tomorrow: tomorrow)
    }
}
