import SwiftUI

/// A heads' report batch read as a digest: the intro, then one entry per head with
/// its outcome, its files and its report. The lead's closing instructions ride in
/// the last head's body and are dropped, since they are for the lead, not the user.
struct HydraReportDigest {
    let intro: String?
    let heads: [Head]

    struct Head: Identifiable {
        let id: String
        let name: String
        let task: String
        let outcome: Outcome
        let effort: String?
        let landing: Landing
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

    /// The report off the lead's `reportMessage`: everything before the first head
    /// header is the intro, each head header starts a head, other `## ` headings
    /// stay in the report, and trailing `(...)` groups on the header are the
    /// outcome or the effort rather than the task.
    static func parse(_ text: String) -> HydraReportDigest {
        let lines = text.components(separatedBy: "\n")
        var introLines: [String] = []
        var sections: [(header: String, lines: [String])] = []
        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("## "), Self.isHeadHeader(String(trimmed.dropFirst(3))) {
                sections.append((String(trimmed.dropFirst(3)), []))
            } else if sections.isEmpty {
                introLines.append(line)
            } else {
                sections[sections.count - 1].lines.append(line)
            }
        }
        let intro = introLines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        var heads: [Head] = []
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
            var body = rest.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if index == sections.count - 1 { body = withoutLeadInstructions(body) }
            heads.append(Head(id: name, name: name, task: task, outcome: outcome, effort: effort, landing: landing, body: body))
        }
        return HydraReportDigest(intro: intro.isEmpty ? nil : intro, heads: heads)
    }

    /// A `## ` line starts a head only when the text before its first colon is a
    /// roster name; anything else (a head's own markdown headings) stays in the body.
    private static func isHeadHeader(_ header: String) -> Bool {
        guard let colon = header.firstIndex(of: ":") else { return false }
        let name = String(header[..<colon]).trimmingCharacters(in: .whitespaces)
        return HydraRoster.index(named: name) != nil
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
        while rest.hasSuffix(")"), let open = rest.lastIndex(of: "(") {
            let group = String(rest[rest.index(after: open)..<rest.index(before: rest.endIndex)])
            rest = String(rest[..<open]).trimmingCharacters(in: .whitespaces)
            let lower = group.lowercased()
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

    /// The closing paragraph the lead appends for itself is not the heads' report;
    /// it leaves with the last head's final paragraph when it reads like instructions.
    private static let leadPrefixes = [
        "The changes listed above", "The heads worked in your checkout", "A file listed with conflicts",
        "Do not check any of this", "Do not wait for", "A head that failed", "A head the user stopped",
        "The user queued", "The user sent",
    ]

    private static func withoutLeadInstructions(_ body: String) -> String {
        guard let range = body.range(of: "\n\n", options: .backwards) else { return body }
        let tail = String(body[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard leadPrefixes.contains(where: { tail.hasPrefix($0) }) else { return body }
        return String(body[..<range.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// The digest of a heads' report batch in the popover its pill opens, laid out like a
/// native popover: a header that stays put, then one plain card per head with its
/// outcome, its files and its report, scrolling under it. The blocks parse off the
/// main thread as the popover opens, so opening costs the blocks on screen.
struct HydraReportsPopover: View {
    let title: String
    let text: String
    let personas: [HydraPersona]

    @State private var digest: HydraReportDigest?
    @State private var expanded: Set<String> = []
    @State private var blocks: [String: [MarkdownBlock]] = [:]

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let digest {
                        ForEach(Array(digest.heads.enumerated()), id: \.element.id) { index, head in
                            if index > 0 {
                                Divider().opacity(0.5)
                            }
                            headCard(head, index: index, collapsible: digest.heads.count > 1)
                        }
                    } else {
                        ProgressView()
                            .controlSize(.small)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 40)
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(idealHeight: 360, maxHeight: 520)
        }
        .frame(width: 460)
        // The popover reads at its own size, not the conversation's zoom.
        .environment(\.chatZoom, 1)
        .task(id: text) {
            let parsed = HydraReportDigest.parse(text)
            digest = parsed
            try? await MarkdownView.warm(parsed.heads.map(\.body))
            guard !Task.isCancelled else { return }
            var warmed: [String: [MarkdownBlock]] = [:]
            for head in parsed.heads {
                warmed[head.name] = MarkdownView.blocks(for: head.body)
            }
            blocks = warmed
            // One head's report reads whole; several start collapsed to their files.
            expanded = parsed.heads.count == 1 ? Set(parsed.heads.map(\.name)) : []
        }
    }

    // MARK: Header

    /// Who reported, and the one sentence saying who is still at work, if anyone is.
    private var header: some View {
        HStack(alignment: .center, spacing: 10) {
            HStack(spacing: -6) {
                ForEach(personas, id: \.self) { persona in
                    HydraGlyph(persona: persona, size: 22)
                }
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(Self.cleanedTitle(title, names: personas.map(\.name)))
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(2)
                if let intro = digest?.intro, let sentence = Self.stillAtWorkSentence(in: intro) {
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

    /// The title with a head's name said once: a lead that writes "Juno: Juno: streaming
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

    /// One head: its name over its outcome, its task, its files, then its report. With
    /// several heads the name row is the toggle that opens and closes the report.
    private func headCard(_ head: HydraReportDigest.Head, index: Int, collapsible: Bool) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if collapsible, hasReport(head) {
                Button {
                    withAnimation(Chrome.panelSlide) {
                        if expanded.contains(head.name) {
                            expanded.remove(head.name)
                        } else {
                            expanded.insert(head.name)
                        }
                    }
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        nameRow(head)
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(Chrome.secondaryText)
                            .rotationEffect(.degrees(expanded.contains(head.name) ? 90 : 0))
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
            } else {
                nameRow(head)
            }
            // The name is on the row above, so the task drops any lead of its own.
            let task = Self.withoutNamePrefix(Self.cleanedTitle(head.task, names: [head.name]), name: head.name)
            if !task.isEmpty {
                Text(task)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            landingRows(head.landing)
            if !collapsible || expanded.contains(head.name) {
                reportBody(head)
            }
        }
    }

    /// The head's name and, beside it, its outcome as a seal or caption plus its effort.
    private func nameRow(_ head: HydraReportDigest.Head) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(head.name)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Chrome.primaryText)
            switch head.outcome {
            case .done:
                Image(systemName: "checkmark.seal.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.success)
                    .accessibilityLabel(Text("Done"))
            case .failed:
                Text("Failed")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.danger)
            case .stopped:
                Text("Stopped")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.warning)
            }
            if let effort = head.effort, !effort.isEmpty {
                Text(effort)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .monospacedDigit()
                    .lineLimit(1)
            }
        }
    }

    private func hasReport(_ head: HydraReportDigest.Head) -> Bool {
        !(head.body.isEmpty || head.body == "No report.")
    }

    /// Where the head's work went: its files as monospaced rows, or one line saying otherwise.
    @ViewBuilder
    private func landingRows(_ landing: HydraReportDigest.Landing) -> some View {
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

    /// Each file on a row of its own: the path, trimmed in the middle when long, and
    /// its additions and deletions at the end. A file still carrying conflict markers
    /// says so in amber.
    @ViewBuilder
    private func fileRows(_ files: [HydraReportDigest.LandedFile]) -> some View {
        if !files.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(files) { file in
                    HStack(spacing: 8) {
                        Text(file.path)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundStyle(file.hasConflicts ? Chrome.warning : Chrome.secondaryText)
                        Spacer(minLength: 8)
                        Text("+\(file.additions) −\(file.deletions)")
                            .foregroundStyle(Chrome.secondaryText)
                            .lineLimit(1)
                            .layoutPriority(1)
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .help(file.hasConflicts ? "\(file.path) (with conflicts)" : file.path)
                }
            }
        }
    }

    /// The head's own report at the popover's reading size; a missing report says so.
    @ViewBuilder
    private func reportBody(_ head: HydraReportDigest.Head) -> some View {
        if !hasReport(head) {
            Text("No report.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
        } else if let headBlocks = blocks[head.name] {
            LazyVStack(alignment: .leading, spacing: 8) {
                ForEach(Array(headBlocks.enumerated()), id: \.offset) { _, block in
                    MarkdownBlockView(block: block)
                        .equatable()
                }
            }
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
