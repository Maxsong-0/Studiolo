// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Stores the Z.ai API key in the macOS Keychain so it never sits
/// in plaintext. UserDefaults keeps a copy only where the Keychain refuses it.
enum ZaiKeychain {
    private static let account = "Z.ai API Key"

    static func apiKey(fallback: String) -> String {
        APIKeychain.key(for: account, fallback: fallback)
    }

    @discardableResult
    static func setAPIKey(_ key: String) -> Bool {
        APIKeychain.setKey(key, account: account)
    }
}
