import AppKit
import SwiftUI

/// A merge note's body read into fields, so the popover can draw the card instead of
/// the bare text. The files line carries the numbers; every other line is a status
/// remark worth one quiet row, and the up-to-date line becomes a check rather than text.
struct HydraMergeNote {
    let link: MergeRequestLink
    let files: Int
    let additions: Int
    let deletions: Int
    let target: String
    let branch: String
    let notes: [String]
    let heads: [Head]

    struct Head: Hashable, Identifiable {
        let name: String
        let index: Int
        let task: String
        let files: [String]
        var id: String { "\(index)-\(name)" }
    }

    /// The files line, which is what makes a body a merge note at all: without it the
    /// caller keeps the generic popover.
    private static let filesPattern = try! NSRegularExpression(
        pattern: #"^(\d+) files? \(\+(\d+) −(\d+)\) landed on `([^`]+)` from `([^`]+)`\.$"#
    )

    static let headPattern = try! NSRegularExpression(pattern: "^- (.+?) \\((\\d+)\\): (.*?) — (?:no files|(\\d+) files?: (.*))$")

    static func parse(_ body: String, link: MergeRequestLink) -> HydraMergeNote? {
        var match: (files: Int, additions: Int, deletions: Int, target: String, branch: String)?
        var notes: [String] = []
        var heads: [Head] = []
        var inHeads = false
        for rawLine in body.components(separatedBy: "\n") {
            let line = rawLine.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !line.isEmpty else {
                inHeads = false
                continue
            }
            if line == "Heads" {
                inHeads = true
                continue
            }
            if inHeads {
                if line.hasPrefix("- ") {
                    let range = NSRange(line.startIndex..., in: line)
                    if let found = headPattern.firstMatch(in: line, range: range) {
                        func group(_ i: Int) -> String {
                            let ns = found.range(at: i)
                            guard ns.location != NSNotFound, let swift = Range(ns, in: line) else { return "" }
                            return String(line[swift])
                        }
                        let filesText = group(5)
                        heads.append(Head(
                            name: group(1), index: Int(group(2)) ?? 0, task: group(3),
                            files: found.range(at: 4).location != NSNotFound && !filesText.isEmpty ? filesText.components(separatedBy: ", ") : []
                        ))
                    }
                    continue
                }
                inHeads = false
            }
            let range = NSRange(line.startIndex..., in: line)
            if let found = filesPattern.firstMatch(in: line, range: range), match == nil {
                func group(_ i: Int) -> String {
                    let ns = found.range(at: i)
                    guard ns.location != NSNotFound, let swift = Range(ns, in: line) else { return "" }
                    return String(line[swift])
                }
                guard let files = Int(group(1)), let additions = Int(group(2)), let deletions = Int(group(3)) else { continue }
                match = (files, additions, deletions, group(4), group(5))
                continue
            }
            // The checkout line goes without saying once the merge is in; it is not a note.
            if line.hasSuffix("is up to date.") { continue }
            notes.append(line.replacingOccurrences(of: "`", with: ""))
        }
        guard let match else { return nil }
        return HydraMergeNote(
            link: link, files: match.files, additions: match.additions, deletions: match.deletions,
            target: match.target, branch: match.branch, notes: notes, heads: heads
        )
    }
}

/// A merge note as a small card: what landed and where, in numbers first, then the
/// checkout's own standing. Sized to its content, so no empty box hangs off the pill.
struct HydraMergePopover: View {
    let note: HydraMergeNote

    /// The forge's mark beside the title, loaded on appear so the header reads as the
    /// merge request's home rather than a bare label.
    @State private var favicon: NSImage?
    /// The link was just copied: the button says so for a beat.
    @State private var didCopyLink = false
    @State private var hoveredHead: HydraMergeNote.Head?

    /// GitLab calls it a merge request; the other forges call it a pull request.
    private var openTitle: String {
        switch note.link.forge {
        case .gitlab: "Open merge request"
        case .github, .gitea: "Open pull request"
        }
    }

    var body: some View {
        let host = note.link.url.host ?? ""
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Group {
                    if let favicon {
                        Image(nsImage: favicon)
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                    } else {
                        Image(systemName: "arrow.triangle.merge")
                            .font(.system(size: 18))
                            .foregroundStyle(Chrome.accent)
                    }
                }
                .frame(width: 22, height: 22)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                VStack(alignment: .leading, spacing: 2) {
                    Text("Merged")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(Chrome.primaryText)
                    Text(verbatim: "\(note.link.label) on \(host)")
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                }
                Spacer()
                Image(systemName: "checkmark.seal.fill")
                    .font(.system(size: 20))
                    .foregroundStyle(Chrome.success)
            }
            HStack(spacing: 8) {
                stat(value: "\(note.files)", label: note.files == 1 ? "file" : "files")
                stat(value: "+\(note.additions)", label: "added", tint: Chrome.success)
                stat(value: "−\(note.deletions)", label: "removed", tint: Chrome.danger)
            }
            // The branch chip spans whatever the arrow and the target leave, so the row
            // runs the card's full width and the name only shortens once it truly cannot fit.
            if !note.heads.isEmpty {
                HStack(spacing: 6) {
                    ForEach(note.heads) { head in
                        HydraGlyph(persona: HydraRoster.persona(at: head.index), size: 26)
                            .help("\(head.name): \(head.task)")
                            .onHover { inside in
                                if inside { hoveredHead = head } else if hoveredHead == head { hoveredHead = nil }
                            }
                            // Each glyph carries its own popover, so the card hangs from the
                            // face under the cursor; one popover on the row hung from the
                            // row's middle, between the faces and the count.
                            .popover(isPresented: Binding(
                                get: { hoveredHead == head },
                                set: { shown in if !shown, hoveredHead == head { hoveredHead = nil } }
                            ), arrowEdge: .bottom) {
                                headCard(head)
                            }
                    }
                    Spacer(minLength: 8)
                    Text(verbatim: note.heads.count == 1 ? "1 head landed" : "\(note.heads.count) heads landed")
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            HStack(spacing: 6) {
                chip(note.branch, fills: true)
                    .help(note.branch)
                Image(systemName: "arrow.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(Chrome.secondaryText)
                chip(note.target)
                    .fixedSize()
            }
            ForEach(Array(note.notes.enumerated()), id: \.offset) { _, entry in
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "info.circle")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                    Text(verbatim: entry)
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            HStack(spacing: 8) {
                Spacer()
                // Answers in place like a popover row: the words say the link is copied, a
                // checkmark comes for the beat, then the button is itself again.
                Button {
                    guard !didCopyLink else { return }
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(note.link.url.absoluteString, forType: .string)
                    withAnimation(.snappy(duration: 0.22)) { didCopyLink = true }
                    Task { [copied = $didCopyLink] in
                        try? await Task.sleep(for: .seconds(1.4))
                        withAnimation(.snappy(duration: 0.22)) { copied.wrappedValue = false }
                    }
                } label: {
                    HStack(spacing: 5) {
                        if didCopyLink {
                            Image(systemName: "checkmark")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(Chrome.success)
                                .transition(.scale(scale: 0.6).combined(with: .opacity))
                        }
                        Text(verbatim: didCopyLink ? "Copied" : "Copy link")
                            .contentTransition(.numericText())
                    }
                }
                .buttonStyle(.glass)
                Button {
                    NSWorkspace.shared.open(note.link.url)
                } label: {
                    Label(openTitle, systemImage: "arrow.up.right")
                }
                .buttonStyle(.glassProminent)
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(18)
        .frame(width: 380)
        .accessibilityElement(children: .contain)
        .task {
            if let known = FaviconCache.cached(host: host) {
                favicon = known
            } else {
                favicon = await FaviconCache.image(for: host)
            }
        }
    }

    /// One number tile: the count large, what it counts small beneath it.
    private func headCard(_ head: HydraMergeNote.Head) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(verbatim: head.name)
                .font(.system(size: 13, weight: .semibold))
            Text(verbatim: head.task)
                .font(.system(size: 12))
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(3)
                .fixedSize(horizontal: false, vertical: true)
            Divider()
            if head.files.isEmpty {
                Text("No files of its own in this merge")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            } else {
                ForEach(head.files, id: \.self) { path in
                    Text(verbatim: path)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
        }
        .padding(12)
        .frame(width: 320)
    }

    private func stat(value: String, label: String, tint: Color? = nil) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: value)
                .font(.system(size: 17, weight: .semibold).monospacedDigit())
                .foregroundStyle(tint ?? Chrome.primaryText)
            Text(verbatim: label)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(Chrome.secondaryText)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Chrome.overlay(0.08)))
    }

    /// A branch name as a capsule, since branch names read as names rather than prose.
    /// A filling chip stretches its capsule across the width it is given.
    private func chip(_ name: String, fills: Bool = false) -> some View {
        Text(verbatim: name)
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(Chrome.primaryText.opacity(0.9))
            .lineLimit(1)
            .truncationMode(.middle)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(maxWidth: fills ? .infinity : nil, alignment: .leading)
            .background(Capsule(style: .continuous).fill(Chrome.overlay(0.1)))
    }
}
