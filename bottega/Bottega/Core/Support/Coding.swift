// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import os

extension KeyedDecodingContainer {
    /// Decodes a value that older saves may not contain.
    func value<T: Decodable>(_ key: Key, default fallback: @autoclosure () -> T) -> T {
        ((try? decodeIfPresent(T.self, forKey: key)) ?? nil) ?? fallback()
    }

    /// Decodes a saved list one element at a time. Read whole, a list stands or falls with
    /// its worst element: one this build cannot read (written by a newer build, say, or
    /// naming a provider this build no longer has) empties it, and the next save writes the
    /// empty list over everything the user had. Read here, that element is dropped on its
    /// own (see `Lenient`) and the rest are kept. A list that is missing, or is not a list
    /// at all, reads as empty, as `value` reads it.
    func values<Element: Decodable>(_ key: Key) -> [Element] {
        value(key, default: [Lenient<Element>]()).compactMap(\.value)
    }

    /// `values` for a list whose absence means something of its own, such as nothing
    /// recorded yet: nil when the key is missing or holds no list.
    func valuesIfPresent<Element: Decodable>(_ key: Key) -> [Element]? {
        value(key, default: [Lenient<Element>]?.none)?.compactMap(\.value)
    }

    /// `values` for the lists a whole stored document is made of (a library's projects, a
    /// thread's items). An element this build cannot read still goes alone; a key that is
    /// missing or null still reads as empty. But a list that is there and is not a list at all
    /// throws: the document then reads as unreadable and is kept aside whole (see `StoredFile`),
    /// rather than read as empty and saved over with nothing.
    func documentValues<Element: Decodable>(_ key: Key) throws -> [Element] {
        try decodeIfPresent([Lenient<Element>].self, forKey: key)?.compactMap(\.value) ?? []
    }
}

/// Where `Lenient` reports an element it dropped.
private let decodingLog = Logger(subsystem: AppInfo.logSubsystem, category: "decoding")

/// Decodes an element without failing the surrounding collection. An element that cannot
/// be read comes back as nil for the collection to drop, and is logged: the next save
/// leaves it out, so the log is the one trace of what was dropped and why.
struct Lenient<Element: Decodable>: Decodable {
    var value: Element?

    init(from decoder: Decoder) throws {
        do {
            value = try Element(from: decoder)
        } catch {
            value = nil
            decodingLog.error("Dropped an unreadable \(String(describing: Element.self), privacy: .public) at \(readablePath(decoder.codingPath), privacy: .public): \(String(describing: error), privacy: .public)")
        }
    }
}

/// A coding path as it reads in the saved JSON, `projects[2].scripts[0]`, so a dropped
/// element, or where a whole file stopped reading (see `StoredFile`), can be found in its file.
func readablePath(_ path: [any CodingKey]) -> String {
    var text = ""
    for key in path {
        if let index = key.intValue {
            text += "[\(index)]"
        } else {
            text += text.isEmpty ? key.stringValue : "." + key.stringValue
        }
    }
    return text.isEmpty ? "the top level" : text
}

extension JSONEncoder {
    static let storage: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys]
        return encoder
    }()
}

extension JSONDecoder {
    static let storage: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}
