// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// `ui.sock`: one JSON object per line. Protocol version 1 (LMAS `docs/bottega-protocol.md`).
public enum Wire {
    public static let version = 1
    /// The longest line LMAS takes, newline included.
    public static let maxLineBytes = 1024 * 1024
    public static let maxEventsPerPage = 500
    /// Nothing is counted further than this, nor is any text longer. A number past it is a mistake; refusing it where
    /// it is read keeps the arithmetic on what LMAS says from overflowing, which would take the app down.
    static let farthest: UInt64 = 1 << 53

    /// Names on the wire are snake_case; unknown fields are ignored, which is how the protocol grows.
    static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return encoder
    }()
}

extension KeyedDecodingContainer {
    /// A number that is counted on from.
    func decodeNumber(from first: UInt64 = 0, forKey key: Key) throws -> UInt64 {
        let number = try decode(UInt64.self, forKey: key)
        guard (first...Wire.farthest).contains(number) else {
            throw DecodingError.dataCorruptedError(forKey: key, in: self, debugDescription: "\(number) is out of range")
        }
        return number
    }

    /// The place of one thing in a row of them, from 1.
    func decodePlace(forKey key: Key) throws -> Int { Int(try decodeNumber(from: 1, forKey: key)) }
}

/// A machine word to switch on, and a sentence for people (LMAS writes it in Chinese).
public struct Why: Decodable, Equatable, Sendable {
    public var code: String
    public var text: String

    public init(code: String, text: String) {
        self.code = code
        self.text = text
    }
}
