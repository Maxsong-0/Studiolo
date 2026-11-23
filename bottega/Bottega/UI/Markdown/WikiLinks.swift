// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Obsidian's `[[wiki links]]`, which Markdown knows nothing of: rewritten into ordinary
/// links of a scheme of the app's own before a note is rendered, so the archive page can
/// take them (through its `openURL` action) and open the note they name. Only the page
/// that shows a vault's notes rewrites; chat text is never touched.
///
/// `[[Note]]`, `[[Note|Shown as]]`, `[[Note#Heading]]`, `[[folder/Note]]` and an embed
/// `![[Note]]` (shown as a link, since the page does not inline notes) are all taken; an
/// embedded image, `![[picture.png]]`, becomes Markdown's own image (see `MarkdownImage`).
/// Nothing inside a fenced code block or an inline code span is.
enum WikiLinks {
    static let scheme = "bottega-wiki"

    /// What a rewritten link names: the note (as written, `.md` and heading aside) and
    /// the heading after `#`, when one was given.
    struct Target: Equatable, Sendable {
        let note: String
        let heading: String?
    }

    static func url(for note: String, heading: String?) -> URL? {
        var components = URLComponents()
        components.scheme = scheme
        // A path, never a host: a host would read as a domain to look a favicon up for.
        components.path = "/" + note
        components.fragment = heading
        return components.url
    }

    /// The target of a link this rewrote; nil for any other URL.
    static func target(of url: URL) -> Target? {
        guard url.scheme == scheme else { return nil }
        let note = url.path.hasPrefix("/") ? String(url.path.dropFirst()) : url.path
        guard !note.isEmpty else { return nil }
        return Target(note: note, heading: url.fragment)
    }

    static func rewrite(_ markdown: String) -> String {
        guard markdown.contains("[[") else { return markdown }
        var out = ""
        out.reserveCapacity(markdown.utf8.count)
        var inFence = false
        for line in markdown.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.drop(while: { $0 == " " })
            if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                inFence.toggle()
                out += line
                out += "\n"
                continue
            }
            out += inFence ? String(line) : rewriteLine(String(line))
            out += "\n"
        }
        // The split put a newline after the last line too.
        if !markdown.hasSuffix("\n") { out.removeLast() }
        return out
    }

    /// One line outside a fence: every `[[…]]` outside a code span becomes a link.
    private static func rewriteLine(_ line: String) -> String {
        guard line.contains("[[") else { return line }
        var out = ""
        var index = line.startIndex
        var inCode = false
        while index < line.endIndex {
            let character = line[index]
            if character == "`" {
                inCode.toggle()
                out.append(character)
                index = line.index(after: index)
                continue
            }
            if !inCode, line[index...].hasPrefix("[["), let close = line.range(of: "]]", range: line.index(index, offsetBy: 2)..<line.endIndex) {
                let inner = String(line[line.index(index, offsetBy: 2)..<close.lowerBound])
                let embedded = out.hasSuffix("!")
                if embedded, let image = image(for: inner) {
                    // An embedded picture: Markdown's own image, shown in place.
                    out.removeLast()
                    out += image
                    index = close.upperBound
                    continue
                }
                if !inner.isEmpty, !inner.contains("[["), let link = link(for: inner) {
                    // Any other embed's `!` goes: the note is linked, not shown in place.
                    if embedded { out.removeLast() }
                    out += link
                    index = close.upperBound
                    continue
                }
            }
            out.append(character)
            index = line.index(after: index)
        }
        return out
    }

    /// `picture.png|alt` → `![alt](picture.png)`, for an embed of a picture (see
    /// `MarkdownImage.extensions`); nil for anything else.
    private static func image(for inner: String) -> String? {
        let parts = inner.split(separator: "|", maxSplits: 1, omittingEmptySubsequences: false)
        let reference = parts[0].trimmingCharacters(in: .whitespaces)
        guard !reference.isEmpty, !reference.contains("]"), !reference.contains(")"),
              MarkdownImage.extensions.contains((reference as NSString).pathExtension.lowercased()) else { return nil }
        let alt = (parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespaces) : "").replacingOccurrences(of: "]", with: "\\]")
        // A space in the name would end the destination: percent-encoded, and decoded again
        // where the picture is looked up.
        let destination = reference.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? reference
        return "![\(alt)](\(destination))"
    }

    /// `Note#Heading|Shown as` → `[Shown as](bottega-wiki:/Note#Heading)`.
    private static func link(for inner: String) -> String? {
        let parts = inner.split(separator: "|", maxSplits: 1, omittingEmptySubsequences: false)
        let reference = parts[0].trimmingCharacters(in: .whitespaces)
        let alias = parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespaces) : nil
        let pieces = reference.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false)
        var note = pieces[0].trimmingCharacters(in: .whitespaces)
        let heading = pieces.count > 1 ? pieces[1].trimmingCharacters(in: .whitespaces).nilIfEmpty : nil
        if note.lowercased().hasSuffix(".md") { note = String(note.dropLast(3)) }
        // `[[#Heading]]` links within the note: shown as the heading, going nowhere else.
        guard !note.isEmpty else { return nil }
        guard let url = url(for: note, heading: heading) else { return nil }
        let shown = (alias?.nilIfEmpty ?? (heading.map { "\(note) › \($0)" } ?? note))
            .replacingOccurrences(of: "]", with: "\\]")
        return "[\(shown)](\(url.absoluteString))"
    }
}
