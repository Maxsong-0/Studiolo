// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation

/// LMAS writes bytes as lowercase hex with no prefix and no spaces. One spelling per byte string, so comparing
/// text and comparing bytes always agree; anything else is refused, uppercase included.
public enum Hex {
    public static func encode(_ bytes: some Sequence<UInt8>) -> String {
        var text = ""
        for byte in bytes {
            text.unicodeScalars.append(digits[Int(byte >> 4)])
            text.unicodeScalars.append(digits[Int(byte & 0x0f)])
        }
        return text
    }

    /// Exactly `count` bytes, or nil.
    public static func decode(_ text: String, count: Int) -> Data? {
        let characters = Array(text.utf8)
        guard characters.count == count * 2 else { return nil }
        var bytes = Data(capacity: count)
        for index in stride(from: 0, to: characters.count, by: 2) {
            guard let high = value(characters[index]), let low = value(characters[index + 1]) else { return nil }
            bytes.append(high << 4 | low)
        }
        return bytes
    }

    private static let digits = Array("0123456789abcdef".unicodeScalars)

    private static func value(_ character: UInt8) -> UInt8? {
        switch character {
        case UInt8(ascii: "0")...UInt8(ascii: "9"): character - UInt8(ascii: "0")
        case UInt8(ascii: "a")...UInt8(ascii: "f"): character - UInt8(ascii: "a") + 10
        default: nil
        }
    }
}
