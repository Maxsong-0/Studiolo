// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A project archive in the user's Studiolo vault: the folder `项目/<name>/` and what its
/// `00-总览/INDEX.md` says about the project. The prefix is the id, the same string
/// Studiolo uses for the project everywhere (its signpost, whitelist and web links).
struct StudioloArchive: Identifiable, Hashable, Sendable {
    /// `项目/<name>`, relative to the vault root.
    let prefix: String
    let name: String
    /// Nil when the folder has no INDEX yet; the archive still exists and can be linked.
    let card: StudioloIndexCard?

    var id: String { prefix }

    init(name: String, card: StudioloIndexCard?) {
        self.name = name
        prefix = StudioloIndex.projectsFolder + "/" + name
        self.card = card
    }
}

/// The dashboard rows of an archive's INDEX, read the way Studiolo reads them for its
/// signpost, so the sidebar and the agents are never told two different things.
struct StudioloIndexCard: Hashable, Sendable {
    var version: String?
    var status: String?
    var latestLog: String?
    var openBugs: String?
    var repository: String?
    /// The first item under `## 当前重点`. Empty when the bullet is there but blank, as in
    /// a fresh archive; Studiolo keeps it empty too.
    var focus: String?
    /// Local paths named in the repository row, as written (`~/…` unexpanded).
    var repositoryPaths: [String] = []

    var tone: StudioloTone { StudioloTone(status: status) }
}

/// The colour an archive's status leads with: the dashboards all start the status cell
/// with one of four circles. `neutral` is a status without one (not `none`, which an
/// optional tone would read as its own nil).
enum StudioloTone: Hashable, Sendable {
    case green, yellow, blue, red, neutral

    /// By the first scalar, so a circle written with a variation selector after it (one
    /// character with it to Swift) still counts.
    init(status: String?) {
        switch status?.unicodeScalars.first {
        case "\u{1F7E2}": self = .green
        case "\u{1F7E1}": self = .yellow
        case "\u{1F535}": self = .blue
        case "\u{1F534}": self = .red
        default: self = .neutral
        }
    }
}

/// Reading and writing archive dashboards. Pure text in, text out: the vault on disk
/// belongs to the Services layer.
///
/// The reading rules are Studiolo's own, rule for rule, from
/// `crates/supply/src/signpost/source.rs` (`first_table_fields`, `section_first_bullet`,
/// `strip_markdown`): only the first two-column table counts, only its known keys are
/// kept, and the focus is the first `- ` item of its section. A dashboard that reads one
/// way here and another way in the signpost would have the user and the agents working
/// from different states.
///
/// Rust's `char` is a Unicode scalar and it compares strings by their bytes, so those rules
/// work on scalars here too. A Swift `Character` would take a combining mark into the `|`,
/// `-` or space before it, and Swift's string equality folds canonically equivalent
/// spellings that Studiolo keeps apart; either would make the two read one INDEX two ways.
enum StudioloIndex {
    static let projectsFolder = "项目"
    static let indexFile = "00-总览/INDEX.md"
    static let templateFolder = "_模板/项目骨架"

    private static let focusHeading = "## 当前重点"
    private static let versionKey = "当前版本"
    private static let statusKey = "状态"
    private static let latestLogKey = "最近日志"
    private static let openBugsKey = "未关闭 BUG"
    private static let repositoryKey = "代码仓库"

    static func parse(_ text: String) -> StudioloIndexCard {
        var card = StudioloIndexCard()
        for (key, raw) in firstTableRows(text) {
            let value = stripMarkdown(raw)
            if same(key, versionKey) {
                card.version = card.version ?? value
            } else if same(key, statusKey) {
                card.status = card.status ?? value
            } else if same(key, latestLogKey) {
                card.latestLog = card.latestLog ?? value
            } else if same(key, openBugsKey) {
                card.openBugs = card.openBugs ?? value
            } else if same(key, repositoryKey), card.repository == nil {
                card.repository = value
                card.repositoryPaths = localPaths(in: raw)
            }
        }
        card.focus = firstBullet(in: text, under: focusHeading)
        return card
    }

    // MARK: - Linking a project folder to its archive

    /// The archive whose repository row names the project's folder or a folder above it.
    /// When several do, the nearest wins: a repository row naming the checkout itself says
    /// more than one naming the directory it sits in. The home folder alone never counts.
    static func pairing(projectPath: String, archives: [StudioloArchive], home: String) -> StudioloArchive? {
        let project = standardized(projectPath)
        let homePath = standardized(home)
        var best: (archive: StudioloArchive, length: Int)?
        for archive in archives {
            for path in expandedPaths(archive.card?.repositoryPaths ?? [], home: homePath) {
                guard path != homePath, path != "/" else { continue }
                guard project == path || project.hasPrefix(path + "/") else { continue }
                if path.count > (best?.length ?? -1) { best = (archive, path.count) }
            }
        }
        return best?.archive
    }

    /// The repository row's paths as folders on this Mac: `~` expanded, standardized.
    static func expandedPaths(_ written: [String], home: String) -> [String] {
        let homePath = standardized(home)
        return written.map { standardized(expandingHome($0, home: homePath)) }
    }

    /// The local paths in a repository cell: every backquoted span that is one, whole
    /// (spaces and all), then any bare `/Users/…` or `~/…` outside the quotes, up to the
    /// first space or punctuation. Server paths (`/opt/…`, `/etc/…`) and links are not local
    /// checkouts and are left out.
    static func localPaths(in cell: String) -> [String] {
        var paths: [String] = []
        var outside = ""
        for (index, piece) in cell.components(separatedBy: "`").enumerated() {
            if index.isMultiple(of: 2) {
                outside += piece + " "
            } else if isLocalPath(piece.trimmingCharacters(in: .whitespaces)) {
                paths.append(trimmedPath(piece.trimmingCharacters(in: .whitespaces)))
            }
        }
        // A path runs to a space, a backquote, or the punctuation that closes it in prose:
        // brackets and quotation marks, ASCII and full-width alike.
        let bare = /(?:~\/|\/Users\/)[^\s`（）()，,；;、。|\]】」』》〉]*/
        for match in outside.matches(of: bare) {
            // A path starts a word: `/Users/…` inside a link (`https://host/Users/…`) or
            // after another path segment is part of something else. A CJK character right
            // before it is prose around the path, not part of it (`位于/Users/…`).
            if match.range.lowerBound > outside.startIndex {
                let before = outside[outside.index(before: match.range.lowerBound)]
                if (before.isASCII && (before.isLetter || before.isNumber)) || "/._-~".contains(before) { continue }
            }
            // A full stop after a path ends the sentence, not the path.
            var raw = String(match.output)
            while raw.hasSuffix(".") { raw.removeLast() }
            let path = trimmedPath(raw)
            if isLocalPath(path) { paths.append(path) }
        }
        var seen = Set<String>()
        return paths.filter { seen.insert($0).inserted }
    }

    // MARK: - A new archive's dashboard

    /// The skeleton's INDEX filled in for a new archive, or nil when the project path
    /// cannot stand in a table cell (a pipe, backquote or line break would break the row).
    ///
    /// The status, repository and latest log rows are always written: into the skeleton's
    /// first table where it has them, added to that table where it does not, and a table of
    /// their own under the title when the skeleton has none. The one-line summary and the
    /// focus item likewise go in whatever shape the skeleton is in, so the new archive always
    /// reads back with its repository (which is what links its other checkouts) and its
    /// status, and the dashboard says what the result page and the briefs say it does.
    static func filledTemplate(_ template: String, name: String, summary: String, projectPath: String, today: String) -> String? {
        guard !projectPath.contains(where: { $0 == "|" || $0 == "`" || $0.isNewline }) else { return nil }
        let summary = summary.components(separatedBy: .newlines).joined(separator: " ").trimmingCharacters(in: .whitespaces)
        let summaryLine = "> 一句话：\(summary)"
        let focusItem = "- \(today) 经 Bottega 建档"
        var rows: [(key: String, value: String)] = [
            (statusKey, "🟢 开发中"),
            (repositoryKey, "`\(projectPath)`"),
            (latestLogKey, "无"),
        ]

        let source = textLines(of: template).map { $0.replacingOccurrences(of: "｛项目名｝", with: name) }
        let isTableLine: (String) -> Bool = { $0.trimmingCharacters(in: .whitespaces).hasPrefix("|") }
        let isBlank: (String) -> Bool = { $0.trimmingCharacters(in: .whitespaces).isEmpty }
        let titleIndex = source.firstIndex { $0.hasPrefix("# ") }
        let summaryIndex = source.firstIndex { $0.hasPrefix("> 一句话：") }
        let tableStart = source.firstIndex(where: isTableLine)
        let tableEnd = tableStart.map { start in (source[start...].firstIndex { !isTableLine($0) } ?? source.endIndex) - 1 }
        let focusHeadingIndex = source.firstIndex { $0.trimmingCharacters(in: .whitespaces) == focusHeading }
        let focusBulletIndex = focusHeadingIndex.flatMap { heading -> Int? in
            let body = source.index(after: heading)
            let end = source[body...].firstIndex { $0.hasPrefix("## ") } ?? source.endIndex
            return source[body..<end].firstIndex { line in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                return trimmed == "-" || trimmed.hasPrefix("- ")
            }
        }
        // With no table of its own, the skeleton gets one after its summary, or its title.
        let newTableAnchor = tableStart == nil ? (summaryIndex ?? titleIndex) : nil

        var lines: [String] = []
        func appendTable(before next: Int) {
            lines += ["", "| 项 | 当前值 |", "|---|---|"] + rows.map { "| \($0.key) | \($0.value) |" }
            rows = []
            if next < source.count, !isBlank(source[next]) { lines.append("") }
        }
        // With neither a title nor a summary line to follow, the summary and the table open
        // the dashboard.
        if titleIndex == nil, summaryIndex == nil, !summary.isEmpty {
            lines.append(summaryLine)
            if tableStart != nil, let first = source.first, !isBlank(first) { lines.append("") }
        }
        if newTableAnchor == nil, tableStart == nil { appendTable(before: 0) }
        if lines.first == "" { lines.removeFirst() }
        for (index, original) in source.enumerated() {
            var line = original
            if index == summaryIndex, !summary.isEmpty { line = summaryLine }
            if let start = tableStart, let end = tableEnd, (start...end).contains(index),
               let key = tableKey(line), let found = rows.firstIndex(where: { $0.key == key }) {
                line = "| \(key) | \(rows.remove(at: found).value) |"
            }
            if index == focusBulletIndex { line = focusItem }
            lines.append(line)
            if index == titleIndex, summaryIndex == nil, !summary.isEmpty { lines += ["", summaryLine] }
            if index == tableEnd, !rows.isEmpty {
                lines += rows.map { "| \($0.key) | \($0.value) |" }
                rows = []
            }
            if index == newTableAnchor { appendTable(before: index + 1) }
            if index == focusHeadingIndex, focusBulletIndex == nil { lines += ["", focusItem] }
        }
        if focusHeadingIndex == nil { lines += ["", focusHeading, "", focusItem] }
        // The skeleton's own line ends, CRLF where it was written with them. A CRLF is one
        // Character, so the last line end is looked for among the scalars.
        let lineEnd = template.contains("\r\n") ? "\r\n" : "\n"
        return lines.joined(separator: lineEnd) + (template.unicodeScalars.last == "\n" || focusHeadingIndex == nil ? lineEnd : "")
    }

    // MARK: - Studiolo's reading rules

    /// `first_table_fields` without its key filter: the rows of the first table whose
    /// cells split into exactly two, alignment rows skipped.
    private static func firstTableRows(_ text: String) -> [(String, String)] {
        var rows: [(String, String)] = []
        var inTable = false
        for line in lines(of: text) {
            let trimmed = trim(line)
            guard trimmed.first == "|" else {
                if inTable { break }
                continue
            }
            inTable = true
            let cells = trimmingPipes(trimmed)
                .split(separator: "|", omittingEmptySubsequences: false)
                .map(trim)
            guard cells.count == 2, !cells[0].allSatisfy({ $0 == "-" || $0 == ":" }) else { continue }
            rows.append((string(cells[0]), string(cells[1])))
        }
        return rows
    }

    /// `section_first_bullet`: the heading must match the whole line, trailing whitespace
    /// aside; the next `## ` heading ends the section; the first `- ` line inside it is the
    /// answer.
    private static func firstBullet(in text: String, under heading: String) -> String? {
        var inSection = false
        for line in lines(of: text) {
            if trimEnd(line).elementsEqual(heading.unicodeScalars) {
                inSection = true
                continue
            }
            guard inSection else { continue }
            if line.starts(with: "## ".unicodeScalars) { return nil }
            let leading = trimStart(line)
            if leading.starts(with: "- ".unicodeScalars) { return stripMarkdown(string(leading.dropFirst(2))) }
        }
        return nil
    }

    /// `strip_markdown`: bold markers, wiki-link brackets and backquotes go, left to right
    /// and each marker in its own pass as Rust's `replace` does, then the ends are trimmed.
    private static func stripMarkdown(_ text: String) -> String {
        let stripped = text
            .replacingOccurrences(of: "**", with: "", options: .literal)
            .replacingOccurrences(of: "[[", with: "", options: .literal)
            .replacingOccurrences(of: "]]", with: "", options: .literal)
            .replacingOccurrences(of: "`", with: "", options: .literal)
        return string(trim(Substring(stripped).unicodeScalars))
    }

    // MARK: - Rust's string primitives, on scalars

    private typealias Scalars = Substring.UnicodeScalarView

    /// `str::lines`: split on line feeds, a carriage return before one dropped. A carriage
    /// return that ends the text without a line feed after it stays, as it does in Rust.
    private static func lines(of text: String) -> [Scalars] {
        var parts = Substring(text).unicodeScalars.split(separator: "\n", omittingEmptySubsequences: false)
        for index in parts.indices.dropLast() where parts[index].last == "\r" {
            parts[index] = parts[index].dropLast()
        }
        if parts.last?.isEmpty == true { parts.removeLast() }
        return parts
    }

    /// The same lines as strings, for filling in a skeleton.
    private static func textLines(of text: String) -> [String] {
        lines(of: text).map(string)
    }

    /// `trim`, `trim_start`, `trim_end`: what Rust trims is exactly Unicode's White_Space.
    private static func trim(_ scalars: Scalars) -> Scalars {
        trimStart(trimEnd(scalars))
    }

    private static func trimStart(_ scalars: Scalars) -> Scalars {
        scalars.drop(while: \.properties.isWhitespace)
    }

    private static func trimEnd(_ scalars: Scalars) -> Scalars {
        var end = scalars.endIndex
        while end > scalars.startIndex, scalars[scalars.index(before: end)].properties.isWhitespace {
            end = scalars.index(before: end)
        }
        return scalars[scalars.startIndex..<end]
    }

    /// `trim_matches('|')`: every pipe at either end.
    private static func trimmingPipes(_ scalars: Scalars) -> Scalars {
        var slice = scalars
        while slice.first == "|" { slice = slice.dropFirst() }
        while slice.last == "|" { slice = slice.dropLast() }
        return slice
    }

    /// Byte-for-byte equality, as Rust compares strings.
    private static func same(_ lhs: String, _ rhs: String) -> Bool {
        lhs.unicodeScalars.elementsEqual(rhs.unicodeScalars)
    }

    private static func string(_ scalars: Scalars) -> String {
        String(Substring(scalars))
    }

    private static func tableKey(_ line: String) -> String? {
        let trimmed = trim(Substring(line).unicodeScalars)
        guard trimmed.first == "|" else { return nil }
        let cells = trimmingPipes(trimmed).split(separator: "|", omittingEmptySubsequences: false)
        guard cells.count == 2 else { return nil }
        return string(trim(cells[0]))
    }

    // MARK: - Paths

    private static func isLocalPath(_ path: String) -> Bool {
        path.hasPrefix("~/") || path.hasPrefix("/Users/")
    }

    private static func trimmedPath(_ path: String) -> String {
        var path = path
        while path.count > 1, path.hasSuffix("/") { path.removeLast() }
        return path
    }

    private static func expandingHome(_ path: String, home: String) -> String {
        path.hasPrefix("~/") ? home + path.dropFirst(1) : path
    }

    private static func standardized(_ path: String) -> String {
        URL(fileURLWithPath: path).standardizedFileURL.path
    }
}
