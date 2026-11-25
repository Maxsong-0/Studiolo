// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// What goes on inside a turn (LMAS `docs/bottega-protocol.md`, section 10; its `live.rs`). All of the text in here was
// written by agents and by the programs they ran: shown as plain text, never as if LMAS or the user had said it.

/// The head of a turn.
public struct TurnHead: Decodable, Equatable, Sendable {
    public var turn, session, vendor: String
    public var model: String?
    public var startedAtMs: UInt64

    public init(turn: String, session: String, vendor: String, model: String? = nil, startedAtMs: UInt64) {
        self.turn = turn
        self.session = session
        self.vendor = vendor
        self.model = model
        self.startedAtMs = startedAtMs
    }
}

/// Text that may have its middle left out.
public struct CutText: Decodable, Equatable, Sendable {
    public var text: String
    public var cut: Cut?

    /// From byte `at` of the original on, `bytes` of them are not in `text`.
    public struct Cut: Decodable, Equatable, Sendable {
        public var at, bytes: UInt64

        public init(at: UInt64, bytes: UInt64) {
            self.at = at
            self.bytes = bytes
        }
    }

    public init(text: String, cut: Cut? = nil) {
        self.text = text
        self.cut = cut
    }
}

/// One thing in a turn. `kind`, `status`, `code` and the like stay strings: an unknown word is shown, not refused.
public struct TurnItem: Equatable, Sendable {
    /// Which one in its turn, from 1.
    public var n: Int
    /// `text`, `reasoning`, `command`, `edit`, `read`, `search`, `coordinator`, `tool`, `plan`, `notice`.
    public var kind: String
    /// `running`, `done`, `failed`, `denied`, `interrupted`.
    public var status: String
    public var startedAt: UInt64?
    public var endedAt: UInt64?
    /// The vendor's own name for its tool.
    public var tool: String?
    public var title: String?
    public var cwd: String?
    public var paths: [String] = []
    /// The command, or the tool's arguments as JSON.
    public var input: CutText?
    /// What was said, thought, printed or returned.
    public var text: CutText?
    public var exitCode: Int?
    public var files: [FileChange] = []
    public var steps: [Step] = []
    /// Of a `notice`: `error`, `denied`, `too_much`.
    public var code: String?

    public struct FileChange: Decodable, Equatable, Sendable {
        /// `add`, `modify`, `delete`, `move`.
        public var path, change: String
        public var to: String?
        public var added, removed: Int?
        public var diff: CutText?
    }

    public struct Step: Decodable, Equatable, Sendable {
        /// `pending`, `running`, `done`.
        public var text, status: String
    }

    public var isOver: Bool { status != "running" }

    /// The command without the shell codex puts around every one. For a few words about it, not for showing what ran.
    public var commandInShort: String? {
        guard let command = input?.text else { return nil }
        for shell in ["zsh", "bash", "sh"] {
            for flag in ["-lc", "-c"] {
                let start = "/bin/\(shell) \(flag) "
                guard command.hasPrefix(start) else { continue }
                let inner = command.dropFirst(start.count)
                for quote in ["'", "\""] where inner.count > 2 && inner.hasPrefix(quote) && inner.hasSuffix(quote) {
                    return String(inner.dropFirst().dropLast())
                }
                return inner.isEmpty ? command : String(inner)
            }
        }
        return command
    }

    public init(n: Int, kind: String, status: String, text: CutText? = nil) {
        self.n = n
        self.kind = kind
        self.status = status
        self.text = text
    }
}

extension TurnItem: Decodable {
    private enum Key: CodingKey {
        case n, kind, status, startedAtMs, endedAtMs, tool, title, cwd, paths, input, text, exitCode, files, steps, code
    }

    /// Only its number is required: a part that cannot be read costs only itself.
    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: Key.self)
        func read<Value: Decodable>(_ key: Key, _: Value.Type = Value.self) -> Value? {
            (try? fields.decodeIfPresent(Value.self, forKey: key)) ?? nil
        }
        n = try fields.decodePlace(forKey: .n)
        kind = read(.kind) ?? ""
        status = read(.status) ?? ""
        startedAt = read(.startedAtMs)
        endedAt = read(.endedAtMs)
        tool = read(.tool)
        title = read(.title)
        cwd = read(.cwd)
        paths = read(.paths) ?? []
        input = read(.input)
        text = read(.text)
        exitCode = read(.exitCode)
        files = read(.files) ?? []
        steps = read(.steps) ?? []
        code = read(.code)
    }
}

/// One pushed line about one turn.
public struct Live: Equatable, Sendable {
    public var turn: String
    public var what: What

    public enum What: Equatable, Sendable {
        /// The first thing this connection says about a turn.
        case began(TurnHead)
        /// What this one is now, all of it.
        case item(TurnItem)
        /// More of the `n`th one's text, from byte `at` of the original on.
        case append(n: Int, at: UInt64, text: String)
        /// Nothing more about this turn comes on this connection. Not `whole`: the last of some is only had by asking.
        case ended(whole: Bool)
        /// Something this Bottega does not know, or cannot read.
        case unknown
    }

    public init(turn: String, what: What) {
        self.turn = turn
        self.what = what
    }
}

extension Live: Decodable {
    private enum Key: CodingKey { case turn, began, item, append, ended }
    private struct Append: Decodable {
        var n: Int
        var at: UInt64
        var text: String

        private enum Key: CodingKey { case n, at, text }
        init(from decoder: any Decoder) throws {
            let fields = try decoder.container(keyedBy: Key.self)
            n = try fields.decodePlace(forKey: .n)
            at = try fields.decodeNumber(forKey: .at)
            text = try fields.decode(String.self, forKey: .text)
        }
    }
    private struct Ended: Decodable { var whole: Bool }

    /// Never throws: these lines are a preview, and one that cannot be read must not cost the connection.
    public init(from decoder: any Decoder) {
        let fields = try? decoder.container(keyedBy: Key.self)
        func read<Value: Decodable>(_ key: Key, _: Value.Type = Value.self) -> Value? {
            (try? fields?.decodeIfPresent(Value.self, forKey: key)) ?? nil
        }
        turn = read(.turn) ?? ""
        if let head = read(.began, TurnHead.self) {
            what = .began(head)
        } else if let item = read(.item, TurnItem.self) {
            what = .item(item)
        } else if let append = read(.append, Append.self) {
            what = .append(n: append.n, at: append.at, text: append.text)
        } else if let ended = read(.ended, Ended.self) {
            what = .ended(whole: ended.whole)
        } else {
            what = .unknown
        }
    }
}

/// `watch` accepted: the turns that are running now.
public struct Watching: Decodable, Equatable, Sendable { public var turns: [String] }

/// A page of `turn`.
public struct TurnPage: Decodable, Equatable, Sendable {
    public var head: TurnHead
    /// `running`, `whole`, `partial`.
    public var state: String
    public var items: [TurnItem]
    /// Up to which one this page goes, and up to which one there are.
    public var through, last: Int

    private enum Key: CodingKey { case state, items, through, last }
    /// One nobody can read costs only itself.
    private struct Lenient: Decodable {
        var item: TurnItem?
        init(from decoder: any Decoder) { item = try? TurnItem(from: decoder) }
    }

    public init(from decoder: any Decoder) throws {
        head = try TurnHead(from: decoder)
        let fields = try decoder.container(keyedBy: Key.self)
        state = try fields.decode(String.self, forKey: .state)
        items = try fields.decode([Lenient].self, forKey: .items).compactMap(\.item)
        through = Int(try fields.decodeNumber(forKey: .through))
        last = Int(try fields.decodeNumber(forKey: .last))
    }
}
