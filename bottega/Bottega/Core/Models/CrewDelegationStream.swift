// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

extension CrewPrompts {
    /// A delegation block read while the reply holding it is still streaming: the entries
    /// whose closing brace has arrived, in the block's order, and whether the block itself
    /// has closed (its array's `]`, a bare entry's `}`, or the closing fence). Hands go
    /// out on these as they close, instead of once the whole block is written: a lead
    /// that takes a minute per brief has its first hand at work while it writes the second.
    struct StreamedDelegations: Hashable, Sendable {
        var delegations: [CrewDelegation] = []
        var isComplete = false
    }

    /// The delegation block of a reply as far as it has streamed: nil until its opening
    /// fence is there. See `streamedDelegations(inBody:)`.
    static func streamedDelegations(in text: String) -> StreamedDelegations? {
        guard let opener = text.firstMatch(of: #/```[ \t]*crew(?:-sent)?[ \t]*\r?\n/#.ignoresCase())?.range else { return nil }
        return streamedDelegations(inBody: text[opener.upperBound...])
    }

    /// The entries closed so far in a block body (what follows the opening fence), read
    /// without waiting for the JSON to be whole. The walk is over bytes: JSON's structure
    /// is ASCII, and a multi-byte character never holds an ASCII byte, so strings (with
    /// their escapes) and braces are tracked exactly, and a fence quoted inside a prompt is
    /// text like any other. An entry is read the moment its outermost brace closes, by the
    /// same rules as `delegations(in:)` (see `delegation(fromEntry:)`), and one that does
    /// not read is skipped as the whole-block parser skips it, so the two agree on the
    /// order and count of what goes out.
    static func streamedDelegations(inBody body: Substring) -> StreamedDelegations {
        var result = StreamedDelegations()
        let bytes = body.utf8
        var index = bytes.startIndex
        while index < bytes.endIndex, isJSONWhitespace(bytes[index]) { index = bytes.index(after: index) }
        guard index < bytes.endIndex else { return result }
        // The body opens an array, or is one bare entry; anything else is not readable yet.
        let isArray: Bool
        switch bytes[index] {
        case UInt8(ascii: "["):
            isArray = true
            index = bytes.index(after: index)
        case UInt8(ascii: "{"):
            isArray = false
        default:
            return result
        }
        var depth = 0
        var inString = false
        var escaped = false
        var entryStart = index
        // The body starts right after the opener's newline, so its first byte opens a line.
        var atLineStart = true
        while index < bytes.endIndex {
            let byte = bytes[index]
            if inString {
                if escaped {
                    escaped = false
                } else if byte == UInt8(ascii: "\\") {
                    escaped = true
                } else if byte == UInt8(ascii: "\"") {
                    inString = false
                }
            } else {
                switch byte {
                case UInt8(ascii: "\""):
                    inString = true
                case UInt8(ascii: "{"):
                    if depth == 0 { entryStart = index }
                    depth += 1
                case UInt8(ascii: "}"):
                    depth -= 1
                    if depth == 0 {
                        // Braces are ASCII, so both ends sit on character boundaries.
                        if let json = JSONValue.parse(String(body[entryStart...index])), let delegation = delegation(fromEntry: json) {
                            result.delegations.append(delegation)
                        }
                        if !isArray {
                            result.isComplete = true
                            return result
                        }
                    } else if depth < 0 {
                        // A closer with nothing open: the block is broken past here, and
                        // nothing after it will read.
                        result.isComplete = true
                        return result
                    }
                case UInt8(ascii: "]") where depth == 0:
                    result.isComplete = true
                    return result
                case UInt8(ascii: "`") where depth == 0 && atLineStart:
                    // A fence opening a line outside any entry is the block's closer.
                    result.isComplete = true
                    return result
                default:
                    break
                }
            }
            atLineStart = byte == UInt8(ascii: "\n")
            index = bytes.index(after: index)
        }
        return result
    }

    /// One entry of a block read as a hand's task: kept only with a prompt, titled from
    /// the prompt when it has no task, and carrying the announced name ("name", or "hand")
    /// as-is when the lead gave one, for the spawner to resolve against the roster, and
    /// the project ("project", or "repo") the hand is sent to, for the spawner to resolve
    /// against the sidebar. An entry may also name one of the pair's hand profiles
    /// ("profile"), which the spawner resolves the same way; an unknown profile runs on
    /// the shared worker model.
    static func delegation(fromEntry entry: JSONValue) -> CrewDelegation? {
        guard let prompt = entry["prompt"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines), !prompt.isEmpty else { return nil }
        let task = entry["task"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let name = entry["name"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
            ?? entry["hand"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
        let announced = (name?.isEmpty == false) ? name : nil
        let project = entry["project"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
            ?? entry["repo"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
        let profile = entry["profile"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
        // The files the entry claims, and the coordination row it was claimed under. Both
        // are optional: an entry that carries neither still delegates, it just gets no
        // overlap check.
        let files = (entry["files"]?.array ?? [])
            .compactMap { $0.string?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        let claim = entry["claim"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
            ?? entry["claim_id"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines)
        return CrewDelegation(task: task.isEmpty ? TextCleanup.singleLine(prompt, limit: 60) : task, prompt: prompt, name: announced, project: (project?.isEmpty == false) ? project : nil, profile: (profile?.isEmpty == false) ? profile : nil, files: files, claimID: (claim?.isEmpty == false) ? claim : nil)
    }

    fileprivate static func isJSONWhitespace(_ byte: UInt8) -> Bool {
        byte == UInt8(ascii: " ") || byte == UInt8(ascii: "\n") || byte == UInt8(ascii: "\r") || byte == UInt8(ascii: "\t")
    }
}
