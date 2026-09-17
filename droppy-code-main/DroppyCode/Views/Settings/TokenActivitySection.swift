import Foundation
import SwiftUI

/// Converts a cumulative token counter into per-event spend.
///
/// Providers report usage with different shapes: Claude hands over session
/// totals, Codex a thread total, ACP a session counter. All three only ever
/// grow within a session, so the spend behind one event is the positive
/// delta since the previous event. A drop (compaction, a fresh session on a
/// reused object) reseeds the baseline without recording: the growth after
/// it is new spend and gets counted from there.
struct TokenSpendTracker {
    private var lastTotal = 0

    mutating func spend(total: Int) -> Int {
        defer { lastTotal = max(0, total) }
        return max(0, total - lastTotal)
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
/// so thousands of transcripts cost one seek each. Per-file fingerprints
/// (mtime plus size) keep later scans incremental, and files written after
/// launch are skipped because live recording already owns them; counting
/// both would double every active session. Claude history is live-only for
/// now: its usage rows are scattered through gigabytes of JSONL, which is
/// not a Settings-open price worth paying.
@MainActor
@Observable
final class TokenLedger {
    static let shared = TokenLedger()

    /// Merged history plus live spend, keyed by local day.
    private(set) var dailyTotals: [String: Int] = [:]

    private static let liveDefaultsKey = "droppycode.tokenActivity.liveDaily"
    /// A capture run keeps to its own suite, like the rest of the app's settings.
    private static let defaults: UserDefaults = WebsiteCaptures.defaults ?? .standard

    /// Live-recorded spend, kept apart from history so a rescan never drops
    /// usage that arrived after the scan.
    private var liveDaily: [String: Int] = [:]
    private var historyDaily: [String: Int] = [:]
    private var didStartLoad = false
    /// The pending write of the live spend, if one is due.
    @ObservationIgnored private var liveSave: Task<Void, Never>?
    /// Live recording owns every session from this moment, so the history
    /// scan only takes files last written before it.
    private let launchDate = Date()

    private init() {
        if let cached = Self.defaults.dictionary(forKey: Self.liveDefaultsKey) as? [String: Int] {
            liveDaily = cached
        }
        dailyTotals = liveDaily
    }

    /// Adds spend to a day. Every provider session runs on the main actor,
    /// so this needs no lock.
    func record(spend: Int, on date: Date = Date()) {
        guard spend > 0 else { return }
        let day = Self.dayKey(for: date)
        liveDaily[day, default: 0] += spend
        dailyTotals[day, default: 0] += spend
        // Usage arrives many times a turn from every running session: the defaults
        // write, a plist and a hop to cfprefsd, waits for a lull and lands whole.
        guard liveSave == nil else { return }
        liveSave = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard let self, !Task.isCancelled else { return }
            liveSave = nil
            Self.defaults.set(liveDaily, forKey: Self.liveDefaultsKey)
        }
    }

    /// Writes the live spend now, for the app quitting with a write still due.
    func flushLiveSpend() {
        guard let liveSave else { return }
        liveSave.cancel()
        self.liveSave = nil
        Self.defaults.set(liveDaily, forKey: Self.liveDefaultsKey)
    }

    /// Runs the Codex history scan once per process. Later calls are no-ops;
    /// live recording keeps the grid fresh after that.
    func refreshIfNeeded() async {
        guard !didStartLoad else { return }
        didStartLoad = true
        let liveCutoff = launchDate
        // Reading, scanning and writing all happen off the main actor: the fingerprint
        // file grows with the number of rollouts, and none of it is wanted on the thread
        // that is drawing Settings.
        let history = await Task.detached(priority: .utility) {
            let result = TokenHistoryScanner.scanCodexHistory(
                cached: CodexFingerprintStore.load(),
                liveCutoff: liveCutoff
            )
            CodexFingerprintStore.save(result.fingerprints)
            return result
        }.value
        historyDaily = history.daily
        dailyTotals = historyDaily.merging(liveDaily) { $0 + $1 }
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

    init(mtime: Double, size: UInt64, tokens: Int, day: String) {
        self.mtime = mtime
        self.size = size
        self.tokens = tokens
        self.day = day
    }

    init?(dict: [String: Any]) {
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
        self.init(mtime: mtime, size: size, tokens: tokens, day: day)
    }
}

/// Where the fingerprints live. There is one per Codex rollout and a busy machine has
/// thousands of them, which has no business in the preferences plist: that file is read
/// whole into every process that reads a default and rewritten whole on every write, so a
/// few megabytes of fingerprints slow down every setting the app touches. They sit in
/// their own JSON file in Application Support instead, beside the library.
private enum CodexFingerprintStore {
    /// Where earlier builds kept them. Read once, then cleared.
    private static let legacyDefaultsKey = "droppycode.tokenActivity.codexFiles"

    private static var fileURL: URL {
        Storage.root.appendingPathComponent("token-activity-codex.json")
    }

    static func load() -> [String: CodexFileFingerprint] {
        if let data = try? Data(contentsOf: fileURL),
           let stored = try? JSONDecoder().decode([String: CodexFileFingerprint].self, from: data) {
            return stored
        }
        guard let legacy = UserDefaults.standard.dictionary(forKey: legacyDefaultsKey) as? [String: [String: Any]] else {
            return [:]
        }
        UserDefaults.standard.removeObject(forKey: legacyDefaultsKey)
        let migrated = legacy.compactMapValues(CodexFileFingerprint.init(dict:))
        save(migrated)
        return migrated
    }

    static func save(_ fingerprints: [String: CodexFileFingerprint]) {
        guard let data = try? JSONEncoder().encode(fingerprints) else { return }
        try? data.write(to: fileURL, options: .atomic)
    }
}

/// Background Codex history scan. Static functions over value types only, so
/// the detached task stays Sendable.
private enum TokenHistoryScanner {
    struct Result: Sendable {
        var daily: [String: Int]
        var fingerprints: [String: CodexFileFingerprint]
    }

    /// How much of each rollout tail to read. token_count events repeat
    /// through a session, so the final one sits near the end.
    private static let tailBytes = 131_072
    private static let historyMonths = 12

    static func scanCodexHistory(cached: [String: CodexFileFingerprint], liveCutoff: Date) -> Result {
        var fingerprints = cached
        var seen: Set<String> = []
        var daily: [String: Int] = [:]
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
                    if known.tokens > 0 { daily[known.day, default: 0] += known.tokens }
                    continue
                }
                let parsed = tailContribution(at: url)
                let fingerprint = CodexFileFingerprint(
                    mtime: mtime.timeIntervalSince1970,
                    size: size,
                    tokens: parsed?.tokens ?? 0,
                    day: parsed?.day ?? ""
                )
                fingerprints[path] = fingerprint
                if let parsed, parsed.tokens > 0 { daily[parsed.day, default: 0] += parsed.tokens }
            }
        }
        // Files deleted since the last scan stop contributing instead of
        // lingering in the cache forever.
        fingerprints = fingerprints.filter { seen.contains($0.key) }
        return Result(daily: daily, fingerprints: fingerprints)
    }

    private static var codexDirectories: [URL] {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return [
            home.appendingPathComponent(".codex/sessions", isDirectory: true),
            home.appendingPathComponent(".codex/archived_sessions", isDirectory: true),
        ]
    }

    /// Reads the tail of one rollout and returns its session total with the
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
            return (tokens, day)
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

/// Contribution grid over the current year, January to December, Sunday rows.
/// Columns are week buckets starting on the Sunday on or before New Year's
/// Day; every column keeps all seven rows so the grid never shears mid-month.
/// Days still to come stay in the grid as faint cells, so the year keeps its
/// shape and the months read Jan through Dec whatever today's date.
private struct TokenActivityGrid {
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

private enum TokenActivityStyle {
    /// Gap between cells, both axes.
    static let cellGap: CGFloat = 4
    static let cellRadius: CGFloat = 2
    /// Height of the month label strip.
    static let labelHeight: CGFloat = 18
    /// Narrowest a month label may squeeze before its neighbors hide it.
    static let minLabelAdvance: CGFloat = 30

    /// Five-step intensity ladder: empty, then four steps of the theme's
    /// accent (the system accent for System/Light/Dark), so the grid wears
    /// whatever the rest of the window wears. A day still to come is fainter
    /// than a quiet one, so the year keeps its shape without reading as data.
    static func color(for value: Int, maxValue: Int, future: Bool) -> Color {
        if future { return Chrome.overlay(0.035) }
        guard value > 0, maxValue > 0 else { return Chrome.overlay(0.07) }
        let accent = Chrome.accent
        let fraction = Double(value) / Double(maxValue)
        switch fraction {
        case ..<0.25: return accent.opacity(0.32)
        case ..<0.5: return accent.opacity(0.55)
        case ..<0.75: return accent.opacity(0.8)
        default: return accent
        }
    }
}

/// The heatmap section at the top of Settings, General. Plain page content,
/// not a ChromeCard: the grid needs the full column width, and a card would
/// inset it into a padded strip the mockup does not have. The mode is
/// view-local and the totals publish from the shared ledger, so switching
/// modes never touches the rest of General.
struct TokenActivitySection: View {
    @State private var mode: TokenActivityMode = .daily
    @State private var ledger = TokenLedger.shared
    /// The grid the last mode showed and a counter the canvas animates across, so a
    /// mode switch blends every cell from its old colour to its new one.
    @State private var previousGrid: TokenActivityGrid?
    @State private var switchCount = 0
    /// Built once per (totals, mode): a year of calendar arithmetic that every usage
    /// event of every running session would otherwise redo while General is open.
    @State private var grid = TokenActivityGrid.build(daily: [:], mode: .daily)

    private struct GridKey: Equatable {
        var daily: [String: Int]
        var mode: TokenActivityMode
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(verbatim: "Token activity")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 12)
                ForEach(TokenActivityMode.allCases, id: \.self) { option in
                    Button {
                        guard option != mode else { return }
                        // The old mode's grid is the blend's starting point and stays until
                        // the blend has settled; the canvas shows it as-is for the first frame
                        // and the new grid as-is for the last.
                        previousGrid = grid
                        let switchIndex = switchCount + 1
                        withAnimation(.smooth(duration: 0.45)) {
                            mode = option
                            switchCount = switchIndex
                        } completion: {
                            if switchCount == switchIndex { previousGrid = nil }
                        }
                    } label: {
                        Text(verbatim: option.title)
                            .font(.system(size: 13, weight: mode == option ? .medium : .regular))
                    }
                    // A capsule chip, like every other small switch in the app: these
                    // were bare words with no hover, no hit target and no button trait.
                    .buttonStyle(.chip(active: mode == option))
                    .help(Self.modeHelp(option))
                    .accessibilityLabel(Text(verbatim: option.title))
                    .accessibilityAddTraits(mode == option ? [.isButton, .isSelected] : .isButton)
                }
            }
            .animation(.smooth(duration: 0.25), value: mode)

            heatmap(grid: grid)
            monthStrip(grid: grid)
            legend(total: ledger.dailyTotals.values.reduce(0, +))
        }
        .onChange(of: GridKey(daily: ledger.dailyTotals, mode: mode), initial: true) { _, key in
            grid = TokenActivityGrid.build(daily: key.daily, mode: key.mode)
        }
        .task {
            await ledger.refreshIfNeeded()
        }
    }

    /// Equal-width week columns of square cells. Each column takes an equal
    /// share of the width and each cell squares off to its column, so the grid
    /// fills any Settings width without clipping or scrolling. One canvas draws
    /// every cell: as a stack of some 360 shape views this was the heaviest
    /// layout in Settings, felt each time General opened.
    private func heatmap(grid: TokenActivityGrid) -> some View {
        HeatmapCanvas(grid: grid, previous: previousGrid, blend: Double(switchCount), switchIndex: switchCount)
            .accessibilityHidden(true)
    }

    /// Month names pinned to the column holding each month's first day.
    /// Labels that would land closer than a readable advance are dropped,
    /// so a narrow window shows fewer months instead of overlapping them.
    private func monthStrip(grid: TokenActivityGrid) -> some View {
        ZStack(alignment: .topLeading) {
            Color.clear.frame(height: TokenActivityStyle.labelHeight)
            GeometryReader { proxy in
                let columnCount = max(grid.columns.count, 1)
                let stride = (proxy.size.width + TokenActivityStyle.cellGap) / CGFloat(columnCount)
                let placed = placedTicks(grid.monthTicks, stride: stride)
                ForEach(placed.indices, id: \.self) { index in
                    Text(verbatim: placed[index].label)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                        .fixedSize()
                        .position(
                            x: min(
                                max(CGFloat(placed[index].column) * stride, 12),
                                proxy.size.width - 12
                            ),
                            y: TokenActivityStyle.labelHeight / 2
                        )
                }
            }
        }
    }

    /// The line under the grid: what the ledger adds up to on the left, and the intensity
    /// ladder on the right. The canvas itself is hidden from VoiceOver (365 cells say
    /// nothing one at a time), so this is where the grid gets a readable value.
    private func legend(total: Int) -> some View {
        HStack(spacing: 8) {
            Text(verbatim: "\(UsagePanel.tokens(total)) tokens recorded")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
            Spacer(minLength: 12)
            Text(verbatim: "Less")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
            ForEach(Array(Self.ladder.enumerated()), id: \.offset) { _, step in
                RoundedRectangle(cornerRadius: TokenActivityStyle.cellRadius, style: .continuous)
                    .fill(step)
                    .frame(width: 10, height: 10)
            }
            Text(verbatim: "More")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("Token activity"))
        .accessibilityValue(Text("\(UsagePanel.tokens(total)) tokens recorded"))
    }

    /// The five steps `TokenActivityStyle.color(for:maxValue:future:)` paints, as a ladder.
    private static var ladder: [Color] {
        [0, 1, 3, 5, 7].map { TokenActivityStyle.color(for: $0, maxValue: 8, future: false) }
    }

    private static func modeHelp(_ mode: TokenActivityMode) -> String {
        switch mode {
        case .daily: "Each day on its own, against the busiest day of the year"
        case .weekly: "Each week as one shade, against the busiest week"
        case .cumulative: "The running total, so the year fills as it goes"
        }
    }

    private func placedTicks(_ ticks: [(label: String, column: Int)], stride: CGFloat) -> [(label: String, column: Int)] {
        var placed: [(label: String, column: Int)] = []
        var lastX = -TokenActivityStyle.minLabelAdvance
        for tick in ticks {
            let x = CGFloat(tick.column) * stride
            guard x - lastX >= TokenActivityStyle.minLabelAdvance else { continue }
            placed.append(tick)
            lastX = x
        }
        return placed
    }
}

/// The heatmap's cells, drawn in one pass. The cell size follows the width the canvas is
/// given; the height follows from that, so the grid stays square-celled at any width.
///
/// A mode switch blends every cell from the colour it had under the previous mode to its
/// new one, so the grid melts between modes instead of snapping. The blend runs on an
/// animatable modifier (`HeatmapBlendModifier`): `switchIndex` lands at once, `blend`
/// travels to it over the animation, and their difference is how far the melt has come.
private struct HeatmapCanvas: View {
    let grid: TokenActivityGrid
    let previous: TokenActivityGrid?
    var blend: Double
    var switchIndex: Int

    @State private var width: CGFloat = 0

    var body: some View {
        let columns = CGFloat(max(grid.columns.count, 1))
        let rows = CGFloat(TokenActivityGrid.rows)
        let gap = TokenActivityStyle.cellGap
        let cell = max(0, (width - gap * (columns - 1)) / columns)
        Color.clear
            .modifier(HeatmapBlendModifier(grid: grid, previous: previous, blend: blend, base: Double(switchIndex - 1)))
            .frame(maxWidth: .infinity)
            .frame(height: max(1, cell * rows + gap * (rows - 1)))
            .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { width = $0 }
    }
}

/// Draws the grid with each cell mixed `progress` of the way from the previous mode's
/// colour to the current one. Animatable over `blend`, so SwiftUI redraws it every frame
/// of a mode switch with the interpolated value.
private struct HeatmapBlendModifier: ViewModifier, Animatable {
    let grid: TokenActivityGrid
    let previous: TokenActivityGrid?
    var blend: Double
    /// Where the latest switch started; `blend - base` is its progress, 0 to 1.
    let base: Double

    var animatableData: Double {
        get { blend }
        set { blend = newValue }
    }

    func body(content: Content) -> some View {
        let progress = min(max(blend - base, 0), 1)
        let gap = TokenActivityStyle.cellGap
        content.overlay {
            Canvas { context, size in
                let columns = grid.columns.count
                guard columns > 0 else { return }
                let cell = (size.width - gap * CGFloat(columns - 1)) / CGFloat(columns)
                guard cell > 0 else { return }
                let pitch = cell + gap
                for c in 0..<columns {
                    for r in 0..<TokenActivityGrid.rows {
                        guard let date = grid.columns[c][r] else { continue }
                        let future = date >= grid.tomorrow
                        var color = TokenActivityStyle.color(for: grid.values[c][r], maxValue: grid.maxValue, future: future)
                        if let previous, progress < 1, c < previous.values.count {
                            let old = TokenActivityStyle.color(for: previous.values[c][r], maxValue: previous.maxValue, future: future)
                            color = progress <= 0 ? old : old.mix(with: color, by: progress)
                        }
                        let rect = CGRect(x: CGFloat(c) * pitch, y: CGFloat(r) * pitch, width: cell, height: cell)
                        context.fill(
                            Path(roundedRect: rect, cornerRadius: TokenActivityStyle.cellRadius, style: .continuous),
                            with: .color(color)
                        )
                    }
                }
            }
        }
    }
}
