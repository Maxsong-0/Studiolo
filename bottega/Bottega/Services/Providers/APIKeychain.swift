// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Security

/// One Keychain implementation for every native provider's API key. Each
/// `*Keychain` enum below used to carry its own identical copy of the SecItem
/// calls, so a fix (say, to the accessibility level) had to be applied four
/// times; the accounts live in one Keychain service (`AppInfo.keychainService`), one item
/// per provider.
enum APIKeychain {
    /// The stored key for an account, or `fallback` when nothing is stored.
    static func key(for account: String, fallback: String) -> String {
        if let keychain = read(account), !keychain.isEmpty { return keychain }
        return fallback
    }

    /// Stores the key, or removes it for an empty one. Returns whether the Keychain
    /// took it, so the caller knows whether a fallback copy is still needed.
    @discardableResult
    static func setKey(_ key: String, account: String) -> Bool {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            delete(account)
            return true
        }
        let data = Data(trimmed.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AppInfo.keychainService,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
        let attributes = query.merging([
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlocked,
        ]) { _, new in new }
        return SecItemAdd(attributes as CFDictionary, nil) == errSecSuccess
    }

    private static func read(_ account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AppInfo.keychainService,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let key = String(data: data, encoding: .utf8),
              !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return key.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func delete(_ account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AppInfo.keychainService,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
