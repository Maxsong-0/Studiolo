// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import BottegaKit
public import BottegaLink
public import CryptoKit
import Foundation

/// Keys in memory that sign without a finger. For the tests and for `bottega-probe` only.
///
/// A target of its own, which the app does not link: with these inside the app, an approval could be signed without
/// Touch ID by whoever got the app to use them. `project.yml` must never name this target (`TestKeysStayOutTests`).
public struct SoftwareKeys: DeviceKeys {
    private let dailyKey: P256.Signing.PrivateKey
    private let approvalKey: P256.Signing.PrivateKey

    public init(daily: P256.Signing.PrivateKey = .init(), approval: P256.Signing.PrivateKey = .init()) {
        dailyKey = daily
        approvalKey = approval
    }

    public var daily: PublicKey { Self.publicKey(dailyKey) }
    public var approval: PublicKey { Self.publicKey(approvalKey) }

    public func sign(_ preimage: Preimage, with use: KeyUse, reason: String) throws -> String {
        let key = use == .daily ? dailyKey : approvalKey
        return Hex.encode(try key.signature(for: preimage.bytes).rawRepresentation)
    }

    private static func publicKey(_ key: P256.Signing.PrivateKey) -> PublicKey {
        // A key CryptoKit just made is on the curve and 65 bytes long.
        PublicKey(x963: key.publicKey.x963Representation)!
    }
}
