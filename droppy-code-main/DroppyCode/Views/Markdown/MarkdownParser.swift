import Foundation

/// Block-level Markdown for agent output. Inline styling is handled by `AttributedString`.
enum MarkdownBlock: Hashable, Sendable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case code(language: String?, code: String)
    case list(ordered: Bool, start: Int, items: [MarkdownListItem])
    case quote([MarkdownBlock])
    case table(header: [String], rows: [[String]])
    case rule
}

struct MarkdownListItem: Hashable, Sendable {
    var text: String
    var checked: Bool?
    var children: [MarkdownBlock]
}

enum MarkdownParser {
    /// A parse, with how much of it survives the text growing by appending, as a streaming
    /// reply does. The parser decides each line from that line and the next, so with `m` the
    /// index of the last line (the one still being written), every decision up to a block
    /// that starts at line `m - 2` or earlier is final: the blocks before it are `stable`,
    /// and a longer text is parsed again from `restart`, that block's UTF-8 offset (see
    /// `parse(_:extending:)`). With no such block, nothing is reused.
    struct Parse: Equatable, Sendable {
        var blocks: [MarkdownBlock]
        var stable: Int
        var restart: Int
    }

    static func parse(_ text: String) -> [MarkdownBlock] {
        parse(text[...]).blocks
    }

    /// The parse of `text` given `previous`, the parse of an earlier text. When `text` merely
    /// extends it, the stable blocks are reused and only the tail is parsed.
    static func parse(_ text: String, extending previous: (text: String, parse: Parse)?) -> Parse {
        guard let previous, previous.parse.stable > 0,
              text.utf8.count > previous.text.utf8.count, text.utf8.starts(with: previous.text.utf8) else {
            return parse(text[...])
        }
        let restart = previous.parse.restart
        let tail = parse(text[text.utf8.index(text.utf8.startIndex, offsetBy: restart)...])
        return Parse(
            blocks: Array(previous.parse.blocks[..<previous.parse.stable]) + tail.blocks,
            stable: previous.parse.stable + tail.stable,
            restart: restart + tail.restart
        )
    }

    /// Splits on `\n`, dropping a `\r` before it, without a whole-text copy first.
    private static func parse(_ text: Substring) -> Parse {
        let utf8 = text.utf8
        let pieces = utf8.split(separator: 0x0A, omittingEmptySubsequences: false)
        var lines: [String] = []
        lines.reserveCapacity(pieces.count)
        var starts: [Int] = []
        starts.reserveCapacity(pieces.count)
        for piece in pieces {
            starts.append(utf8.distance(from: utf8.startIndex, to: piece.startIndex))
            lines.append(String(Substring(piece.last == 0x0D ? piece.dropLast() : piece)))
        }
        let parsed = parse(lines[...])
        // The last block that starts at least two lines above the last line: the decisions
        // that end everything before it never read the line still being written.
        guard let stable = parsed.blockLines.lastIndex(where: { $0 <= lines.count - 3 }) else {
            return Parse(blocks: parsed.blocks, stable: 0, restart: 0)
        }
        return Parse(blocks: parsed.blocks, stable: stable, restart: starts[parsed.blockLines[stable]])
    }

    /// Quotes and list items nest by recursion, one level per `>` or indent step, which model
    /// output decides. Past this many levels the inner lines are paragraph text.
    private static let maxDepth = 8

    /// The blocks of `lines`, and the line each top-level block starts on.
    private static func parse(_ lines: ArraySlice<String>, depth: Int = 0) -> (blocks: [MarkdownBlock], blockLines: [Int]) {
        var blocks: [MarkdownBlock] = []
        var blockLines: [Int] = []
        var index = lines.startIndex
        var paragraph: [String] = []
        var paragraphStart = index

        func flushParagraph() {
            let text = paragraph.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                blocks.append(.paragraph(text))
                blockLines.append(paragraphStart)
            }
            paragraph.removeAll()
        }

        while index < lines.endIndex {
            let line = lines[index]
            let trimmed = self.trimmed(line)

            if trimmed.isEmpty {
                flushParagraph()
                index += 1
                continue
            }

            if let fence = fenceMarker(trimmed) {
                flushParagraph()
                blockLines.append(index)
                let language = String(trimmed.dropFirst(fence.count)).trimmingCharacters(in: .whitespaces)
                var code: [String] = []
                index += 1
                while index < lines.endIndex {
                    let candidate = self.trimmed(lines[index])
                    if candidate.hasPrefix(fence), candidate.allSatisfy({ $0 == fence.first }) { index += 1; break }
                    code.append(lines[index])
                    index += 1
                }
                blocks.append(.code(language: language.isEmpty ? nil : language, code: code.joined(separator: "\n")))
                continue
            }

            if let heading = headingLevel(trimmed) {
                flushParagraph()
                blockLines.append(index)
                let content = trimmed.drop(while: { $0 == "#" }).trimmingCharacters(in: .whitespaces)
                blocks.append(.heading(level: heading, text: content.trimmingCharacters(in: CharacterSet(charactersIn: "# "))))
                index += 1
                continue
            }

            if isRule(trimmed) {
                flushParagraph()
                blockLines.append(index)
                blocks.append(.rule)
                index += 1
                continue
            }

            if trimmed.hasPrefix(">"), depth < maxDepth {
                flushParagraph()
                blockLines.append(index)
                var quoted: [String] = []
                while index < lines.endIndex {
                    let candidate = self.trimmed(lines[index])
                    guard candidate.hasPrefix(">") else { break }
                    var content = candidate.dropFirst()
                    if content.hasPrefix(" ") { content = content.dropFirst() }
                    quoted.append(String(content))
                    index += 1
                }
                blocks.append(.quote(parse(quoted[...], depth: depth + 1).blocks))
                continue
            }

            if index + 1 < lines.endIndex, trimmed.contains("|"), isTableDelimiter(lines[index + 1]) {
                flushParagraph()
                blockLines.append(index)
                let header = tableCells(trimmed)
                var rows: [[String]] = []
                index += 2
                while index < lines.endIndex {
                    let candidate = self.trimmed(lines[index])
                    guard !candidate.isEmpty, candidate.contains("|") else { break }
                    rows.append(tableCells(candidate))
                    index += 1
                }
                blocks.append(.table(header: header, rows: rows))
                continue
            }

            if depth < maxDepth, let marker = listMarker(line), paragraph.isEmpty || !marker.ordered || marker.start == 1 {
                flushParagraph()
                blockLines.append(index)
                let (list, next) = parseList(lines, from: index, depth: depth)
                blocks.append(list)
                index = next
                continue
            }

            if paragraph.isEmpty { paragraphStart = index }
            paragraph.append(line)
            index += 1
        }
        flushParagraph()
        return (blocks, blockLines)
    }

    private struct ListMarker {
        var indent: Int
        var ordered: Bool
        var start: Int
        var contentOffset: Int
        var content: String
    }

    private static func parseList(_ lines: ArraySlice<String>, from start: Int, depth: Int) -> (MarkdownBlock, Int) {
        guard let first = listMarker(lines[start]) else { return (.paragraph(lines[start]), start + 1) }
        var items: [MarkdownListItem] = []
        var index = start
        var currentText: [String] = []
        var childLines: [String] = []
        var hasItem = false

        func flushItem() {
            guard hasItem else { return }
            var text = currentText.joined(separator: "\n").trimmingCharacters(in: .whitespaces)
            var checked: Bool?
            if text.hasPrefix("[ ] ") {
                checked = false
                text.removeFirst(4)
            } else if text.lowercased().hasPrefix("[x] ") {
                checked = true
                text.removeFirst(4)
            }
            items.append(MarkdownListItem(text: text, checked: checked, children: parse(childLines[...], depth: depth + 1).blocks))
            currentText.removeAll()
            childLines.removeAll()
        }

        while index < lines.endIndex {
            let line = lines[index]
            let trimmed = self.trimmed(line)
            if trimmed.isEmpty {
                let next = index + 1
                if next < lines.endIndex, indentation(lines[next]) > first.indent, !self.trimmed(lines[next]).isEmpty {
                    childLines.append("")
                    index += 1
                    continue
                }
                if next < lines.endIndex, let marker = listMarker(lines[next]), marker.indent == first.indent, marker.ordered == first.ordered {
                    index += 1
                    continue
                }
                break
            }
            if let marker = listMarker(line), marker.indent <= first.indent {
                guard marker.ordered == first.ordered else { break }
                flushItem()
                hasItem = true
                currentText = [marker.content]
                index += 1
                continue
            }
            if indentation(line) > first.indent {
                let dropped = String(line.dropFirst(min(indentation(line), first.contentOffset)))
                if childLines.isEmpty, listMarker(line) == nil, fenceMarker(trimmed) == nil {
                    currentText.append(trimmed)
                } else {
                    childLines.append(dropped)
                }
                index += 1
                continue
            }
            if childLines.isEmpty, headingLevel(trimmed) == nil, fenceMarker(trimmed) == nil, !trimmed.hasPrefix(">") {
                currentText.append(trimmed)
                index += 1
                continue
            }
            break
        }
        flushItem()
        return (.list(ordered: first.ordered, start: first.start, items: items), index)
    }

    /// `trimmingCharacters(in: .whitespaces)`, skipped for the usual line that starts and ends
    /// with printable ASCII: no space or tab there, and no lead byte of a Unicode space either.
    private static func trimmed(_ line: String) -> String {
        if let first = line.utf8.first, let last = line.utf8.last,
           first > 0x20, first < 0x7F, last > 0x20, last < 0x7F { return line }
        return line.trimmingCharacters(in: .whitespaces)
    }

    private static func listMarker(_ line: String) -> ListMarker? {
        let indent = indentation(line)
        let body = line.dropFirst(indent)
        if let first = body.first, "-*+".contains(first) {
            let rest = body.dropFirst()
            guard rest.hasPrefix(" ") || rest.hasPrefix("\t") else { return nil }
            let content = rest.drop(while: { $0 == " " || $0 == "\t" })
            return ListMarker(indent: indent, ordered: false, start: 1, contentOffset: indent + 2, content: String(content))
        }
        let digits = body.prefix(while: \.isNumber)
        guard !digits.isEmpty, digits.count <= 9 else { return nil }
        let rest = body.dropFirst(digits.count)
        guard let delimiter = rest.first, delimiter == "." || delimiter == ")" else { return nil }
        let afterDelimiter = rest.dropFirst()
        guard afterDelimiter.hasPrefix(" ") || afterDelimiter.isEmpty else { return nil }
        let content = afterDelimiter.drop(while: { $0 == " " })
        return ListMarker(indent: indent, ordered: true, start: Int(digits) ?? 1, contentOffset: indent + digits.count + 2, content: String(content))
    }

    private static func indentation(_ line: String) -> Int {
        var count = 0
        for character in line {
            if character == " " { count += 1 } else if character == "\t" { count += 4 } else { break }
        }
        return count
    }

    private static func fenceMarker(_ trimmed: String) -> String? {
        for marker in ["```", "~~~"] where trimmed.hasPrefix(marker) {
            guard let fenceCharacter = marker.first else { continue }
            return String(trimmed.prefix(while: { $0 == fenceCharacter }))
        }
        return nil
    }

    private static func headingLevel(_ trimmed: String) -> Int? {
        let hashes = trimmed.prefix(while: { $0 == "#" }).count
        guard (1...6).contains(hashes) else { return nil }
        let rest = trimmed.dropFirst(hashes)
        return rest.isEmpty || rest.hasPrefix(" ") ? hashes : nil
    }

    /// Three or more of one of `-`, `*`, `_`, spaces between them allowed. Every bullet line
    /// starts with one of those, so this checks bytes rather than building a string.
    private static func isRule(_ trimmed: String) -> Bool {
        guard let first = trimmed.utf8.first, first == 0x2D || first == 0x2A || first == 0x5F else { return false }
        var count = 0
        for byte in trimmed.utf8 {
            if byte == first { count += 1 } else if byte != 0x20 { return false }
        }
        return count >= 3
    }

    private static func isTableDelimiter(_ line: String) -> Bool {
        let trimmed = self.trimmed(line)
        guard trimmed.contains("-") else { return false }
        return trimmed.allSatisfy { "|:- ".contains($0) }
    }

    private static func tableCells(_ line: String) -> [String] {
        var content = trimmed(line)
        if content.hasPrefix("|") { content.removeFirst() }
        if content.hasSuffix("|") { content.removeLast() }
        return content.split(separator: "|", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces) }
    }
}
