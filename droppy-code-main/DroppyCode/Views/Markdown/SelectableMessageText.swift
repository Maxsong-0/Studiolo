import Foundation

/// The message as plain text for the clipboard: one walk over the app's own blocks,
/// no styling. Headings as their text, code blocks verbatim, links as `title (url)`,
/// the delegation block left out. Used by the row menu's Copy message (see
/// `AssistantMessageRow.messageActions`). The file keeps the name of the selectable text
/// view it used to hold, which left with the whole-reply drag mode; this enum is all that
/// stayed.
enum MessageText {
    static func plain(_ markdown: String) -> String {
        var lines: [String] = []
        var first = true
        for block in MarkdownParser.parse(markdown) {
            if !first { lines.append("") }
            first = false
            append(block, to: &lines)
        }
        return lines.joined(separator: "\n")
    }

    private static func append(_ block: MarkdownBlock, to lines: inout [String]) {
        switch block {
        case .heading(_, let text):
            lines.append(inline(text))
        case .paragraph(let text):
            lines.append(inline(text))
        case .code(let language, let code):
            guard language?.lowercased() != "hydra", !code.isEmpty else { return }
            lines.append(code)
        case .list(let ordered, let start, let items):
            for (index, item) in items.enumerated() {
                let marker: String
                if let checked = item.checked {
                    marker = checked ? "☑ " : "☐ "
                } else if ordered {
                    marker = "\(start + index).  "
                } else {
                    marker = "•  "
                }
                if !item.text.isEmpty { lines.append(marker + inline(item.text)) }
                for child in item.children { append(child, to: &lines) }
            }
        case .quote(let inner):
            for child in inner { append(child, to: &lines) }
        case .table(let header, let rows):
            lines.append(header.joined(separator: "  "))
            for row in rows { lines.append(row.joined(separator: "  ")) }
        case .rule:
            break
        }
    }

    /// Links as `title (url)` when the title differs, the bare URL otherwise.
    private static func inline(_ source: String) -> String {
        var text = replace(#"!\[([^\[\]]*)\]\(\S+?\)"#, in: source) { $0[0] }
        text = replace(#"\[([^\[\]]+)\]\((\S+?)(?:\s+"[^"]*")?\)"#, in: text) { groups in
            groups[0] == groups[1] ? groups[1] : "\(groups[0]) (\(groups[1]))"
        }
        text = replace(#"<((?:https?|mailto):[^<>\s]+)>"#, in: text) { $0[0] }
        return text
    }

    private static func replace(_ pattern: String, in text: String, with transform: ([String]) -> String) -> String {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return text }
        let matches = regex.matches(in: text, range: NSRange(text.startIndex..., in: text))
        guard !matches.isEmpty else { return text }
        var out = text
        for match in matches.reversed() {
            let ns = out as NSString
            var groups: [String] = []
            for group in 1..<match.numberOfRanges {
                let range = match.range(at: group)
                groups.append(range.location == NSNotFound ? "" : ns.substring(with: range))
            }
            guard let range = Range(match.range, in: out) else { continue }
            out.replaceSubrange(range, with: transform(groups))
        }
        return out
    }
}
