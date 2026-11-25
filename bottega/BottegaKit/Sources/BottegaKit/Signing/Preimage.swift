// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation
import CryptoKit

/// The bytes a signature covers: fields in a fixed order, each with its length (4 bytes, big endian) in front.
/// The first field names the purpose, so a signature made for one thing is never valid for another.
/// Protocol section 4; `vectors.json` holds the reference bytes.
public struct Preimage: Equatable, Sendable {
    public private(set) var bytes: Data

    private init(_ purpose: String) {
        bytes = Data()
        append(purpose)
    }

    /// The SHA-256 of these bytes. An intent is recognised by this, never by its signature: the same bytes have
    /// more than one valid signature.
    public var digest: String { Hex.encode(SHA256.hash(data: bytes)) }

    public static let nonceBytes = 32

    /// Proves that the other end of this connection holds the device's daily key.
    public static func handshake(nonce: Data, connection: UInt64) -> Preimage {
        var preimage = Preimage("bottega-handshake-v1")
        preimage.append(nonce)
        preimage.append(connection)
        return preimage
    }

    /// A registration, signed once by each of the two keys.
    public static func register(
        kind: String, name: String, dailyKey: Data, approvalKey: Data, nonce: Data, connection: UInt64
    ) -> Preimage {
        var preimage = Preimage("bottega-register-v1")
        preimage.append(kind)
        preimage.append(name)
        preimage.append(dailyKey)
        preimage.append(approvalKey)
        preimage.append(nonce)
        preimage.append(connection)
        return preimage
    }

    /// An intent: the head in the clear, then the body exactly as sent.
    public static func intent(
        device: String, number: UInt64, createdAt: UInt64, epoch: UInt64, kind: String, body: String
    ) -> Preimage {
        var preimage = Preimage("bottega-intent-v1")
        preimage.append(device)
        preimage.append(number)
        preimage.append(createdAt)
        preimage.append(epoch)
        preimage.append(kind)
        preimage.append(body)
        return preimage
    }

    private mutating func append(_ field: Data) {
        // Nothing that gets signed comes near 4 GiB: a line to LMAS is at most 1 MiB.
        withUnsafeBytes(of: UInt32(clamping: field.count).bigEndian) { bytes.append(contentsOf: $0) }
        bytes.append(field)
    }

    private mutating func append(_ text: String) { append(Data(text.utf8)) }

    private mutating func append(_ number: UInt64) {
        append(withUnsafeBytes(of: number.bigEndian) { Data($0) })
    }
}

public enum Digest {
    /// SHA-256 of a text's UTF-8 bytes: what an approval says it read.
    public static func sha256(_ text: String) -> String { Hex.encode(SHA256.hash(data: Data(text.utf8))) }
}
