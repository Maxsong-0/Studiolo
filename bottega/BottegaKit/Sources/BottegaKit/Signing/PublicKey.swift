// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation
import CryptoKit

/// A P-256 public key in uncompressed X9.63 form: `04`, X, Y.
public struct PublicKey: Equatable, Sendable {
    public static let byteCount = 65
    public static let signatureByteCount = 64

    public let bytes: Data
    private let key: P256.Signing.PublicKey

    public static func == (one: PublicKey, other: PublicKey) -> Bool { one.bytes == other.bytes }

    public init?(x963 bytes: Data) {
        guard bytes.count == Self.byteCount, let key = try? P256.Signing.PublicKey(x963Representation: bytes) else {
            return nil
        }
        self.bytes = bytes
        self.key = key
    }

    public init?(hex: String) {
        guard let bytes = Hex.decode(hex, count: Self.byteCount) else { return nil }
        self.init(x963: bytes)
    }

    public var hex: String { Hex.encode(bytes) }

    /// Whether `signature` (hex of r then s) is this key's over `preimage`. A signature spelled wrong is just wrong.
    public func verifies(_ preimage: Preimage, signature: String) -> Bool {
        guard
            let raw = Hex.decode(signature, count: Self.signatureByteCount),
            let signature = try? P256.Signing.ECDSASignature(rawRepresentation: raw)
        else { return false }
        return key.isValidSignature(signature, for: preimage.bytes)
    }
}
