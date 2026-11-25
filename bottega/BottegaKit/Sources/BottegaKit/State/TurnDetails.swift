// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// Text as far as it has come, in pieces by where they stood in the original. Between two pieces something is missing.
public struct GrownText: Equatable, Sendable {
    public struct Piece: Equatable, Sendable {
        /// The first byte of it in the original.
        public let at: UInt64
        public internal(set) var text: String

        public var end: UInt64 { at + UInt64(text.utf8.count) }
    }

    public private(set) var pieces: [Piece] = []

    /// Kept of a text that does not stop growing: its start, and its end.
    static let keptHead = 64 * 1024
    static let keptTail = 192 * 1024

    public init() {}

    public init(_ text: CutText) {
        guard let cut = text.cut, cut.bytes <= Wire.farthest, let (head, tail) = Self.split(text.text, at: cut.at) else {
            append(text.text, at: 0)
            return
        }
        append(head, at: 0)
        append(tail, at: cut.at + cut.bytes)
    }

    /// How long the original is, as far as is known.
    public var end: UInt64 { pieces.last?.end ?? 0 }

    /// What was left out before each piece, in bytes.
    public func missing(before index: Int) -> UInt64 {
        pieces[index].at - (index == 0 ? 0 : pieces[index - 1].end)
    }

    public var isWhole: Bool { pieces.count < 2 && (pieces.first?.at ?? 0) == 0 }

    /// Takes `text`, which starts at byte `at` of the original. What is already here is not taken twice.
    public mutating func append(_ text: String, at: UInt64) {
        var (text, at) = (text, at)
        if at < end {
            guard let (_, rest) = Self.split(text, at: end - at) else { return }
            (text, at) = (rest, end)
        }
        guard !text.isEmpty, at <= Wire.farthest else { return }
        if let last = pieces.indices.last, pieces[last].end == at {
            pieces[last].text += text
        } else {
            pieces.append(Piece(at: at, text: text))
        }
        trim()
    }

    /// The end of it, for showing while it grows: at most `bytes`, from the start of a line where there is one.
    public func tail(_ bytes: Int) -> String {
        guard let last = pieces.last, last.text.utf8.count > bytes else { return pieces.last?.text ?? "" }
        let end = Self.suffix(last.text, bytes)
        guard let newline = end.firstIndex(where: \.isNewline) else { return end }
        let line = end[end.index(after: newline)...]
        return line.isEmpty ? end : String(line)
    }

    /// Lets go of the middle, once there is clearly too much of it: not at every append, each time is a copy.
    private mutating func trim() {
        let kept = pieces.reduce(0) { $0 + $1.text.utf8.count }
        guard kept > (Self.keptHead + Self.keptTail) * 3 / 2 else { return }
        var head: [Piece] = [], tail: [Piece] = []
        var room = Self.keptHead
        for piece in pieces {
            let text = piece.text.utf8.count <= room ? piece.text : Self.prefix(piece.text, room)
            if !text.isEmpty { head.append(Piece(at: piece.at, text: text)) }
            room -= text.utf8.count
            if text.utf8.count < piece.text.utf8.count { break }
        }
        room = Self.keptTail
        for piece in pieces.reversed() {
            let text = piece.text.utf8.count <= room ? piece.text : Self.suffix(piece.text, room)
            if !text.isEmpty { tail.append(Piece(at: piece.end - UInt64(text.utf8.count), text: text)) }
            room -= text.utf8.count
            if text.utf8.count < piece.text.utf8.count { break }
        }
        pieces = head + tail.reversed()
    }

    /// nil when byte `at` is not where a character starts, or past the end.
    private static func split(_ text: String, at: UInt64) -> (String, String)? {
        guard
            let offset = Int(exactly: at), let index = text.utf8.index(text.startIndex, offsetBy: offset, limitedBy: text.endIndex),
            let place = index.samePosition(in: text.unicodeScalars)
        else { return nil }
        return (String(text.unicodeScalars[..<place]), String(text.unicodeScalars[place...]))
    }

    private static func prefix(_ text: String, _ bytes: Int) -> String {
        var index = text.utf8.index(text.startIndex, offsetBy: bytes)
        while index.samePosition(in: text.unicodeScalars) == nil { index = text.utf8.index(before: index) }
        return String(text.unicodeScalars[..<index])
    }

    private static func suffix(_ text: String, _ bytes: Int) -> String {
        var index = text.utf8.index(text.endIndex, offsetBy: -bytes)
        while index.samePosition(in: text.unicodeScalars) == nil { index = text.utf8.index(after: index) }
        return String(text.unicodeScalars[index...])
    }
}

/// What went on in one turn, as far as it has been told: pushed while it runs, or fetched afterwards.
public struct TurnDetails: Equatable, Sendable {
    public struct Item: Equatable, Sendable, Identifiable {
        /// As LMAS last gave it. Its `text` is in the other `text`, which also has what came after.
        public internal(set) var item: TurnItem
        public internal(set) var text: GrownText

        public var id: Int { item.n }

        init(_ item: TurnItem) {
            self.item = item
            text = item.text.map(GrownText.init) ?? GrownText()
            self.item.text = nil
        }
    }

    /// Whether what is here is all there is.
    public enum Completeness: Equatable, Sendable {
        /// More may come by itself.
        case growing
        /// The turn is over and some of it did not arrive. LMAS may have it.
        case behind
        /// All that LMAS has.
        case whole
        /// All that LMAS has, which is not all there was.
        case partial
    }

    public private(set) var head: TurnHead?
    /// In order.
    public private(set) var items: [Item] = []
    public private(set) var completeness: Completeness = .behind

    public init() {}

    /// The newest one that is not over: what the agent is doing now.
    public var current: Item? { items.last { !$0.item.isOver } }

    public mutating func take(_ what: Live.What) {
        switch what {
        case .began(let head):
            // A connection says it first, and then everything anew.
            self.head = head
            completeness = .growing
        case .item(let item): put(Item(item))
        case .append(let n, let at, let text):
            guard let index = index(of: n), !items[index].item.isOver else { return }
            items[index].text.append(text, at: at)
        case .ended(let whole): completeness = whole ? .whole : .behind
        case .unknown: break
        }
    }

    /// Fetched from the start: instead of what was here, unless that is still growing and further along.
    public mutating func take(_ pages: [TurnPage]) {
        guard let last = pages.last else { return }
        if completeness == .growing, last.state == "running" { return }
        head = last.head
        items = []
        for page in pages { for item in page.items { put(Item(item)) } }
        completeness = switch last.state {
        case "whole": .whole
        case "partial": .partial
        // Asked while it ran, with nothing pushed: asking again later gives more.
        default: .behind
        }
    }

    /// The connection that told about it is gone, or says it is not running.
    public mutating func fellBehind() {
        if completeness == .growing { completeness = .behind }
    }

    private mutating func put(_ item: Item) {
        if let index = index(of: item.id) {
            items[index] = item
        } else {
            let place = items.firstIndex { $0.id > item.id } ?? items.endIndex
            items.insert(item, at: place)
        }
    }

    private func index(of n: Int) -> Int? {
        // They count up from 1 without gaps, so the place is known unless something is missing.
        if n > 0, n <= items.count, items[n - 1].id == n { return n - 1 }
        return items.firstIndex { $0.id == n }
    }
}
