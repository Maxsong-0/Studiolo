// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// `ui.sock`: one JSON object per line. Protocol version 1 (LMAS `docs/bottega-protocol.md`).
public enum Wire {
    public static let version = 1
    /// The longest line LMAS takes, newline included.
    public static let maxLineBytes = 1024 * 1024
    public static let maxEventsPerPage = 500

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

/// A machine word to switch on, and a sentence for people (LMAS writes it in Chinese).
public struct Why: Decodable, Equatable, Sendable {
    public var code: String
    public var text: String

    public init(code: String, text: String) {
        self.code = code
        self.text = text
    }
}
