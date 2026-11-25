// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import CryptoKit
import Foundation
import Testing

@testable import BottegaLink

/// Real Secure Enclave keys, made for the test and thrown away with its folder: nothing goes into the keychain, and
/// nothing asks for a finger. What needs one (signing with the approval key) is tried by hand.
@Suite(.enabled(if: SecureEnclave.isAvailable))
struct SecureKeysTests {
    private let folder = FileManager.default.temporaryDirectory.appendingPathComponent("keys-\(UUID().uuidString)")
    private var chest: SecureChest { SecureChest(isDevelopment: true, folder: folder, service: "unused") }

    @Test func theDailyKeySignsWhatLMASCanCheck() async throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(try chest.keys() == nil)
        let made = try chest.makeKeys()
        #expect(made.daily != made.approval)

        // The next launch: same keys out of the files.
        let keys = try #require(try chest.keys())
        #expect(keys.daily == made.daily)
        #expect(keys.approval == made.approval)

        let preimage = Preimage.handshake(nonce: Data(repeating: 0xAB, count: 32), connection: 7)
        let signature = try await keys.sign(preimage, with: .daily, reason: "")
        #expect(signature.utf8.count == 128)
        #expect(signature == signature.lowercased())
        #expect(keys.daily.verifies(preimage, signature: signature))
        #expect(!keys.approval.verifies(preimage, signature: signature))
    }

    @Test func theFilesArePrivateAndReplaced() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        let first = try chest.makeKeys()
        let second = try chest.makeKeys()
        #expect(first.daily != second.daily)
        #expect(try chest.keys()?.daily == second.daily)
        for name in try FileManager.default.contentsOfDirectory(atPath: folder.path) {
            let mode = try FileManager.default.attributesOfItem(atPath: folder.appendingPathComponent(name).path)[.posixPermissions]
            #expect(mode as? Int == 0o600)
        }
    }

    /// Half a pair is no pair: it is made afresh, not mixed.
    @Test func oneKeyMissingIsNoKeys() throws {
        defer { try? FileManager.default.removeItem(at: folder) }
        _ = try chest.makeKeys()
        try FileManager.default.removeItem(at: folder.appendingPathComponent("approval-key"))
        #expect(try chest.keys() == nil)
    }
}
