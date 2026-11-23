// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// A hands' report batch read as a digest: the intro, then one entry per hand with
/// its outcome, its files and its report. The lead's closing instructions ride in
/// the last hand's body and are dropped, since they are for the lead, not the user.
struct CrewReportDigest {
    let intro: String?
    let hands: [Hand]

    struct Hand: Identifiable {
        let id: String
        let name: String
        let task: String
        let outcome: Outcome
        let effort: String?
        let landing: Landing
        /// The Markdown file the hand's report was kept in, when the section names one.
        let reportPath: String?
        let body: String
    }

    enum Outcome {
        case done, failed, stopped
    }

    enum Landing {
        case landed([LandedFile])
        case notLanded([LandedFile], error: String?)
        case nothing
        case unfinished(copyPath: String?)
        case none
    }

    struct LandedFile: Identifiable {
        let path: String
        let additions: Int
        let deletions: Int
        let hasConflicts: Bool
        var id: String { path }
    }

    /// The report off the lead's `reportMessage`: everything before the first hand
    /// header is the intro, each hand header starts a hand, other `## ` headings
    /// stay in the report, and trailing `(...)` groups on the header are the
    /// outcome or the effort rather than the task.
    static func parse(_ text: String) -> CrewReportDigest {
        let lines = text.components(separatedBy: "\n")
        var introLines: [String] = []
        var sections: [(header: String, lines: [String])] = []
        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("## "), Self.isHandHeader(String(trimmed.dropFirst(3))) {
                sections.append((String(trimmed.dropFirst(3)), []))
            } else if sections.isEmpty {
                introLines.append(line)
            } else {
                sections[sections.count - 1].lines.append(line)
            }
        }
        let intro = introLines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        var hands: [Hand] = []
        for (index, section) in sections.enumerated() {
            let (name, task, outcome, effort) = parseHeader(section.header)
            var rest = section.lines
            while let first = rest.first, first.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                rest.removeFirst()
            }
            var landing = Landing.none
            if let first = rest.first, Self.landingPrefix(first) {
                landing = parseLanding(first.trimmingCharacters(in: .whitespacesAndNewlines))
                rest.removeFirst()
            }
            // The report file's line comes next, when the report was kept on disk: it is
            // no part of what the hand said.
            var reportPath: String?
            if let first = rest.first?.trimmingCharacters(in: .whitespacesAndNewlines), first.hasPrefix(CrewPrompts.reportFilePrefix) {
                let path = String(first.dropFirst(CrewPrompts.reportFilePrefix.count)).trimmingCharacters(in: .whitespaces)
                reportPath = path.isEmpty ? nil : path
                rest.removeFirst()
            }
            var body = rest.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if index == sections.count - 1 { body = withoutLeadInstructions(body) }
            hands.append(Hand(id: name, name: name, task: task, outcome: outcome, effort: effort, landing: landing, reportPath: reportPath, body: body))
        }
        return CrewReportDigest(intro: intro.isEmpty ? nil : intro, hands: hands)
    }

    /// A `## ` line starts a hand only when the text before its first colon is a
    /// roster name; anything else (a hand's own markdown headings) stays in the body.
    private static func isHandHeader(_ header: String) -> Bool {
        guard let colon = header.firstIndex(of: ":") else { return false }
        let name = String(header[..<colon]).trimmingCharacters(in: .whitespaces)
        return CrewRoster.index(named: name) != nil
    }

    /// The name off the colon, then trailing `(...)` groups stripped from the task:
    /// `failed` or `stopped...` is the outcome, anything else the effort.
    private static func parseHeader(_ header: String) -> (name: String, task: String, outcome: Outcome, effort: String?) {
        let trimmed = header.trimmingCharacters(in: .whitespaces)
        let name: String
        var rest: String
        if let colon = trimmed.firstIndex(of: ":") {
            name = String(trimmed[..<colon]).trimmingCharacters(in: .whitespaces)
            rest = String(trimmed[trimmed.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
        } else {
            name = trimmed
            rest = ""
        }
        var outcome = Outcome.done
        var efforts: [String] = []
        let metaEfforts: Set<String> = ["low", "medium", "high", "xhigh", "max", "minimal", "fast", "default"]
        while rest.hasSuffix(")"), let open = rest.lastIndex(of: "(") {
            let group = String(rest[rest.index(after: open)..<rest.index(before: rest.endIndex)])
            let lower = group.lowercased()
            let isMeta = lower == "failed" || lower.hasPrefix("stopped")
                || group.rangeOfCharacter(from: .decimalDigits) != nil
                || metaEfforts.contains(lower)
            guard isMeta else { break }
            rest = String(rest[..<open]).trimmingCharacters(in: .whitespaces)
            if lower == "failed" {
                outcome = .failed
            } else if lower.hasPrefix("stopped") {
                outcome = .stopped
            } else if !group.isEmpty {
                efforts.insert(group.replacingOccurrences(of: ", ", with: " · "), at: 0)
            }
        }
        return (name, rest, outcome, efforts.isEmpty ? nil : efforts.joined(separator: " · "))
    }

    private static func landingPrefix(_ line: String) -> Bool {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        return trimmed.hasPrefix("Landed in your checkout:")
            || trimmed.hasPrefix("Did not land")
            || trimmed.hasPrefix("Changed nothing.")
            || trimmed.hasPrefix("Nothing landed")
    }

    /// The landing line off its prefix: the files part is comma-separated
    /// `path (+A −D)` items, before any `, and N more` or build-output tail.
    private static func parseLanding(_ line: String) -> Landing {
        if line.hasPrefix("Changed nothing.") { return .nothing }
        if line.hasPrefix("Nothing landed") {
            if let range = line.range(of: "in its copy at ") {
                var path = String(line[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
                if path.hasSuffix(".") { path = String(path.dropLast()) }
                return .unfinished(copyPath: path.isEmpty ? nil : path)
            }
            return .unfinished(copyPath: nil)
        }
        if line.hasPrefix("Landed in your checkout:") {
            return .landed(parseFiles(String(line.dropFirst("Landed in your checkout:".count))))
        }
        if line.hasPrefix("Did not land") {
            var rest = String(line.dropFirst("Did not land".count))
            var error: String?
            if rest.hasPrefix(" ("), let close = rest.firstIndex(of: ")") {
                error = String(rest[rest.index(rest.startIndex, offsetBy: 2)..<close]).trimmingCharacters(in: .whitespaces)
                rest = String(rest[rest.index(after: close)...])
            }
            if rest.hasPrefix(":") { rest = String(rest.dropFirst()) }
            return .notLanded(parseFiles(rest), error: error.flatMap { $0.isEmpty ? nil : $0 })
        }
        return .none
    }

    private static let filePattern: NSRegularExpression = {
        // The minus the lead writes is U+2212; a hyphen still matches for safety.
        try! NSRegularExpression(pattern: #"^(\S+) \(\+(\d+) [−\-](\d+)\)( with conflicts)?$"#)
    }()

    private static func parseFiles(_ raw: String) -> [LandedFile] {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasSuffix(".") { text = String(text.dropLast()) }
        // The tails the lead appends after the files, longest first.
        for suffix in [#"; \d+ build-output files? (was|were) left out$"#, #", and \d+ more$"#] {
            if let range = text.range(of: suffix, options: .regularExpression) {
                text = String(text[..<range.lowerBound])
            }
        }
        return text.components(separatedBy: ", ").compactMap { item in
            matchFile(item.trimmingCharacters(in: .whitespaces))
        }
    }

    private static func matchFile(_ item: String) -> LandedFile? {
        let range = NSRange(item.startIndex..., in: item)
        guard let match = filePattern.firstMatch(in: item, range: range), match.range == range else { return nil }
        func group(_ i: Int) -> String? {
            let r = match.range(at: i)
            guard r.location != NSNotFound, let sr = Range(r, in: item) else { return nil }
            return String(item[sr])
        }
        guard let path = group(1), let a = group(2).flatMap(Int.init), let d = group(3).flatMap(Int.init) else { return nil }
        return LandedFile(path: path, additions: a, deletions: d, hasConflicts: group(4) != nil)
    }

    /// The closing paragraph the lead appends for itself is not the hands' report;
    /// it leaves with the last hand's final paragraph when it reads like instructions.
    /// These are the sentences `CrewPrompts.reportMessage` can open it with. "The user
    /// sent" opens it in reports from builds that could send a message straight to a hand.
    private static let leadPrefixes = [
        "The changes listed above", "The hands worked in your checkout", "A file listed with conflicts",
        "Do not check any of this", "Do not wait for", "A hand that failed", "A hand the user stopped",
        "The user queued", "The user sent",
    ]

    private static func withoutLeadInstructions(_ body: String) -> String {
        guard let range = body.range(of: "\n\n", options: .backwards) else { return body }
        let tail = String(body[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard leadPrefixes.contains(where: { tail.hasPrefix($0) }) || opensWithHandsNotice(tail) else { return body }
        return String(body[..<range.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Whether the closing opens with one of the two sentences that start with a hand's
    /// name rather than a fixed phrase: two hands on one file ("Otto and Nova both changed
    /// …") or a patch that would not apply ("Otto's changes would not apply …"). With no
    /// landing, no checkout work and no conflict ahead of them, either can come first, and
    /// the prefixes alone left the lead's instructions in the last hand's report.
    private static func opensWithHandsNotice(_ tail: String) -> Bool {
        if let range = tail.range(of: "'s changes would not apply") {
            return CrewRoster.index(named: String(tail[..<range.lowerBound])) != nil
        }
        if let range = tail.range(of: " both changed ") {
            let names = tail[..<range.lowerBound]
                .replacingOccurrences(of: " and ", with: ", ")
                .components(separatedBy: ", ")
            return names.count > 1 && names.allSatisfy { CrewRoster.index(named: $0) != nil }
        }
        return false
    }
}

/// The digest of a hands' report batch in the popover its pill opens, laid out like a
/// native popover: a header that stays put, then one plain card per hand with its
/// outcome, its files and its report, scrolling under it. Everything the first layout
/// shows is ready before it: the digest, and one hand's report (which reads whole) as
/// blocks the pill warmed or a cold tap parses now; with several hands the reports
/// start collapsed and warm in the background. `PopoverScroll` lays the cards out once
/// at the popover's width, so it opens at its final size instead of growing. With one
/// hand the header carries the hand and its task, and the card is just its files and report.
struct CrewReportsPopover: View {
    let title: String
    let text: String
    let personas: [CrewPersona]
    private let digest: CrewReportDigest
    private var single: CrewReportDigest.Hand? { digest.hands.count == 1 ? digest.hands.first : nil }

    @State private var expanded: Set<String>
    @State private var blocks: [String: [MarkdownBlock]]

    /// The popover's width; the cards are measured at it.
    private static let width: CGFloat = 460

    @MainActor
    init(title: String, text: String, personas: [CrewPersona]) {
        self.title = title
        self.text = text
        self.personas = personas
        let parsed = CrewReportDigest.parse(text)
        digest = parsed
        // One hand's report reads whole; several start collapsed to their files.
        if parsed.hands.count == 1, let hand = parsed.hands.first {
            _expanded = State(initialValue: [hand.name])
            _blocks = State(initialValue: [hand.name: MarkdownView.blocks(for: hand.body)])
        } else {
            _expanded = State(initialValue: [])
            _blocks = State(initialValue: [:])
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            let collapsible = digest.hands.count > 1
            PopoverScroll(maxHeight: 520, width: Self.width) {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(digest.hands.enumerated()), id: \.element.id) { index, hand in
                        handCard(hand, index: index, collapsible: collapsible)
                    }
                }
                .padding(collapsible ? 10 : 0)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(width: Self.width)
        // The popover reads at its own size, not the conversation's zoom.
        .environment(\.chatZoom, 1)
        .task(id: text) {
            // The reports not on screen yet parse off the main thread, so opening a card
            // finds its blocks ready.
            let missing = digest.hands.filter { blocks[$0.name] == nil }
            guard !missing.isEmpty else { return }
            try? await MarkdownView.warm(missing.map(\.body))
            guard !Task.isCancelled else { return }
            var warmed = blocks
            for hand in missing {
                warmed[hand.name] = MarkdownView.blocks(for: hand.body)
            }
            blocks = warmed
        }
    }

    // MARK: Header

    /// Who reported, and the one sentence saying who is still at work, if anyone is.
    /// With one hand the header carries the hand itself: its name, outcome, files and task.
    @ViewBuilder
    private var header: some View {
        if let hand = single {
            singleHeader(hand)
        } else {
            HStack(alignment: .center, spacing: 10) {
                HStack(spacing: -6) {
                    ForEach(Array(personas.prefix(9)), id: \.self) { persona in
                        CrewGlyph(persona: persona, size: 22)
                    }
                    if personas.count > 9 {
                        // Faces past a handful overrun the header: the rest are counted
                        // here, and the totals line beside names the full number.
                        Text(verbatim: "+\(personas.count - 9)")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3)
                            .background(Capsule(style: .continuous).fill(Chrome.overlay(0.1)))
                            .padding(.leading, 10)
                    }
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: Self.cleanedTitle(title, names: personas.map(\.name)))
                        .foregroundColor(Chrome.primaryText)
                        .font(.system(size: 13, weight: .semibold))
                        .lineLimit(2)
                    Text(Self.totalsLine(for: digest))
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                        .monospacedDigit()
                    if let intro = digest.intro, let sentence = Self.stillAtWorkSentence(in: intro) {
                        Text(sentence)
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .lineLimit(2)
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
        }
    }

    private func singleHeader(_ hand: CrewReportDigest.Hand) -> some View {
        HStack(alignment: .center, spacing: 10) {
            CrewGlyph(persona: persona(for: hand), size: 24, status: Self.status(hand.outcome))
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(hand.name)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Chrome.primaryText)
                    Text(Self.outcomeCaption(for: hand))
                        .font(.system(size: 11))
                        .foregroundStyle(Self.outcomeColor(for: hand.outcome))
                        .monospacedDigit()
                    if let files = Self.filesLine(for: hand) {
                        Text("· " + files)
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .monospacedDigit()
                            .lineLimit(1)
                    }
                }
                let task = Self.withoutNamePrefix(Self.cleanedTitle(hand.task, names: [hand.name]), name: hand.name)
                Text(task.isEmpty ? Self.cleanedTitle(title, names: [hand.name]) : task)
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                if let intro = digest.intro, let sentence = Self.stillAtWorkSentence(in: intro) {
                    Text(sentence)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(2)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private static func outcomeCaption(for hand: CrewReportDigest.Hand) -> String {
        let base: String
        switch hand.outcome {
        case .done: base = "Finished"
        case .failed: base = "Failed"
        case .stopped: base = "Stopped"
        }
        guard let effort = hand.effort, !effort.isEmpty else { return base }
        return base + " · " + effort
    }

    private static func outcomeColor(for outcome: CrewReportDigest.Outcome) -> Color {
        switch outcome {
        case .failed: return Chrome.danger
        case .stopped: return Chrome.warning
        case .done: return Chrome.secondaryText
        }
    }

    private static func filesLine(for hand: CrewReportDigest.Hand) -> String? {
        let files: [CrewReportDigest.LandedFile]
        switch hand.landing {
        case .landed(let f): files = f
        case .notLanded(let f, _): files = f
        default: return nil
        }
        guard !files.isEmpty else { return nil }
        let f = files.count
        let a = files.reduce(0) { $0 + $1.additions }
        let d = files.reduce(0) { $0 + $1.deletions }
        return "\(f) file" + (f == 1 ? "" : "s") + " · +\(a) −\(d)"
    }

    /// The title with a hand's name said once: a lead that writes "Juno: Juno: streaming
    /// path" has the repeated `Name: ` (or `Name `) prefix dropped until one name is left.
    static func cleanedTitle(_ title: String, names: [String]) -> String {
        var cleaned = title
        for name in names where !name.isEmpty {
            let prefixes = ["\(name): ", "\(name) "]
            func namePrefix(of string: String) -> String? {
                prefixes.first { string.hasPrefix($0) }
            }
            while let prefix = namePrefix(of: cleaned), namePrefix(of: String(cleaned.dropFirst(prefix.count))) != nil {
                cleaned = String(cleaned.dropFirst(prefix.count))
            }
        }
        return cleaned
    }

    /// The string without one leading `Name: `, for a line that sits under the name already.
    private static func withoutNamePrefix(_ string: String, name: String) -> String {
        let prefix = "\(name): "
        guard !name.isEmpty, string.hasPrefix(prefix) else { return string }
        return String(string.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces)
    }

    /// '<N> hands · <F> files · +<A> −<D>', dropping the files part when no files.
    private static func totalsLine(for digest: CrewReportDigest) -> String {
        let n = digest.hands.count
        var files: [CrewReportDigest.LandedFile] = []
        for hand in digest.hands {
            switch hand.landing {
            case .landed(let f): files.append(contentsOf: f)
            case .notLanded(let f, _): files.append(contentsOf: f)
            default: break
            }
        }
        var line = "\(n) \(n == 1 ? "hand" : "hands")"
        guard !files.isEmpty else { return line }
        let f = files.count
        let a = files.reduce(0) { $0 + $1.additions }
        let d = files.reduce(0) { $0 + $1.deletions }
        line += " · \(f) \(f == 1 ? "file" : "files") · +\(a) −\(d)"
        return line
    }

    /// The sentence holding 'still at work', from its start to the intro's end.
    private static func stillAtWorkSentence(in intro: String) -> String? {
        guard let match = intro.range(of: "still at work") else { return nil }
        let start: String.Index
        if let stop = intro[..<match.lowerBound].lastIndex(of: ".") {
            start = intro.index(after: stop)
        } else {
            start = intro.startIndex
        }
        let sentence = intro[start...].trimmingCharacters(in: .whitespacesAndNewlines)
        return sentence.isEmpty ? nil : sentence
    }

    // MARK: Cards

    private func persona(for hand: CrewReportDigest.Hand) -> CrewPersona {
        if let match = personas.first(where: { $0.name == hand.name }) { return match }
        if let persona = CrewRoster.persona(named: hand.name) { return persona }
        return CrewRoster.persona(at: 0)
    }

    private static func status(_ outcome: CrewReportDigest.Outcome) -> CrewHandInfo.Status {
        switch outcome {
        case .done: return .completed
        case .failed: return .failed
        case .stopped: return .stopped
        }
    }

    /// One hand: its name over its outcome, its task, its files, then its report. With
    /// several hands the name row is the toggle that opens and closes the report.
    private func handCard(_ hand: CrewReportDigest.Hand, index: Int, collapsible: Bool) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if collapsible {
                if hasReport(hand) {
                    // No animation, like the diff inspector's file cards: the popover takes
                    // its height from the content, and an animated toggle feeds it a new
                    // height every frame, which AppKit animates again on top.
                    Button {
                        if expanded.contains(hand.name) {
                            expanded.remove(hand.name)
                        } else {
                            expanded.insert(hand.name)
                        }
                    } label: {
                        HStack(alignment: .center, spacing: 6) {
                            nameRow(hand)
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.right")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(Chrome.secondaryText)
                                .rotationEffect(.degrees(expanded.contains(hand.name) ? 90 : 0))
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                } else {
                    nameRow(hand)
                }
            }
            if collapsible {
                // The name is on the row above, so the task drops any lead of its own.
                let task = Self.withoutNamePrefix(Self.cleanedTitle(hand.task, names: [hand.name]), name: hand.name)
                if !task.isEmpty {
                    Text(task)
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.primaryText)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            landingRows(hand.landing)
            reportFileRow(hand.reportPath)
            if !collapsible || expanded.contains(hand.name) {
                reportBody(hand)
            }
        }
        .padding(12)
        .background {
            if collapsible {
                RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Chrome.overlay(0.05))
            }
        }
    }

    /// The hand's name and, beside it, its outcome as a seal or caption plus its effort.
    private func nameRow(_ hand: CrewReportDigest.Hand) -> some View {
        HStack(alignment: .center, spacing: 8) {
            CrewGlyph(persona: persona(for: hand), size: 20, status: Self.status(hand.outcome))
            Text(hand.name)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(persona(for: hand).color)
            switch hand.outcome {
            case .done:
                EmptyView()
            case .failed:
                Text("Failed")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.danger)
            case .stopped:
                Text("Stopped")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.warning)
            }
            Spacer(minLength: 8)
            if let effort = hand.effort, !effort.isEmpty {
                Text(effort)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .monospacedDigit()
                    .lineLimit(1)
            }
        }
    }

    private func hasReport(_ hand: CrewReportDigest.Hand) -> Bool {
        !(hand.body.isEmpty || hand.body == "No report.")
    }

    /// Where the hand's work went: its files as monospaced rows, or one line saying otherwise.
    @ViewBuilder
    private func landingRows(_ landing: CrewReportDigest.Landing) -> some View {
        switch landing {
        case .landed(let files):
            fileRows(files)
        case .notLanded(let files, let error):
            VStack(alignment: .leading, spacing: 4) {
                Text(error.map { "Did not land: \($0)" } ?? "Did not land")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.warning)
                    .fixedSize(horizontal: false, vertical: true)
                fileRows(files)
            }
        case .nothing:
            Text("Changed nothing.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        case .unfinished(let copyPath):
            VStack(alignment: .leading, spacing: 2) {
                Text("Nothing landed")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.warning)
                if let copyPath {
                    Text(copyPath)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
        case .none:
            EmptyView()
        }
    }

    /// Where the hand's report was kept on disk, one click from opening; nothing for a
    /// report from before the files were kept.
    @ViewBuilder
    private func reportFileRow(_ path: String?) -> some View {
        if let path {
            Button {
                NSWorkspace.shared.open(URL(fileURLWithPath: path))
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "doc.text")
                    Text(verbatim: path)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Chrome.secondaryText)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Open the report file")
        }
    }

    /// Each file on a row of its own: the path, trimmed in the middle when long, and
    /// its additions and deletions at the end. A file still carrying conflict markers
    /// says so in amber.
    @ViewBuilder
    private func fileRows(_ files: [CrewReportDigest.LandedFile]) -> some View {
        if !files.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(files) { file in
                    HStack(spacing: 8) {
                        filePathText(file)
                            .lineLimit(1)
                            .truncationMode(.head)
                        Spacer(minLength: 8)
                        HStack(spacing: 6) {
                            Text("+\(file.additions)").foregroundStyle(Chrome.success)
                            Text("−\(file.deletions)").foregroundStyle(Chrome.danger)
                        }
                        .monospacedDigit()
                        .layoutPriority(1)
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .help(file.hasConflicts ? "\(file.path) (with conflicts)" : file.path)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Chrome.overlay(0.05)))
        }
    }

    private func filePathText(_ file: CrewReportDigest.LandedFile) -> Text {
        let path = file.path
        let slash = path.lastIndex(of: "/")
        let directory = slash.map { String(path[...$0]) } ?? ""
        let filename = slash.map { String(path[path.index(after: $0)...]) } ?? path
        let head = Text(directory).foregroundStyle(Chrome.secondaryText)
        let tail = Text(filename).foregroundStyle(file.hasConflicts ? Chrome.warning : Chrome.primaryText)
        return Text("\(head)\(tail)")
    }

    /// The hand's own report at the popover's reading size; a missing report says so.
    @ViewBuilder
    private func reportBody(_ hand: CrewReportDigest.Hand) -> some View {
        if !hasReport(hand) {
            Text("No report.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        } else if let handBlocks = blocks[hand.name] {
            // Eager: a lazy stack reports an estimated height until its rows exist, and
            // the popover would resize a second time once they do.
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(handBlocks.enumerated()), id: \.offset) { _, block in
                    MarkdownBlockView(block: block)
                        .equatable()
                }
            }
            .padding(.top, 2)
            .font(.system(size: 12))
            .environment(\.markdownPointSize, 12)
            .environment(\.markdownDimmed, false)
            .textSelection(.enabled)
        } else {
            ProgressView()
                .controlSize(.small)
                .padding(.vertical, 8)
        }
    }
}
