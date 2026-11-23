// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Security

/// Stores MCP secret field values in the macOS Keychain so they never sit
/// in plaintext. `mcp.json` keeps only the non-secret values.
enum MCPKeychain {
    private static func account(server: String, field: String) -> String {
        "MCP " + server + " " + field
    }

    static func value(server: String, field: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AppInfo.keychainService,
            kSecAttrAccount as String: account(server: server, field: field),
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let value = String(data: data, encoding: .utf8),
              !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Stores the value, or removes it for an empty one.
    @discardableResult
    static func set(_ value: String, server: String, field: String) -> Bool {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            delete(server: server, field: field)
            return true
        }
        let data = Data(trimmed.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AppInfo.keychainService,
            kSecAttrAccount as String: account(server: server, field: field),
        ]
        let updated = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if updated == errSecSuccess { return true }
        guard updated == errSecItemNotFound else { return false }
        let attributes = query.merging([
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlocked,
        ]) { _, new in new }
        return SecItemAdd(attributes as CFDictionary, nil) == errSecSuccess
    }

    static func delete(server: String, field: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: AppInfo.keychainService,
            kSecAttrAccount as String: account(server: server, field: field),
        ]
        SecItemDelete(query as CFDictionary)
    }

    /// Forgets every secret the entry declares. The entry, not its id: a custom server
    /// is not in the catalog, so looking it up there would silently delete nothing.
    static func deleteAll(server entry: MCPCatalogEntry) {
        for field in entry.fields {
            delete(server: entry.id, field: field.key)
        }
    }
}
