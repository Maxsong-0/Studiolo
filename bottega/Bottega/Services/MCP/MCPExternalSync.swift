// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// One-time cleanup: the connected servers reach the CLIs at launch now
/// (Claude, Codex, Copilot, OpenCode), never through their config files;
/// this only takes back what an earlier build put there.
enum MCPExternalSync {
    static func sync(_ servers: [MCPResolvedServer]) {
        let manager = FileManager.default
        guard manager.fileExists(atPath: managedURL.path) else { return }
        // A record that cannot be read now is not taken for one naming nothing: removed, it
        // would take with it the only list of what to take back. It stays for a later launch.
        guard let managed = readManaged() else { return }
        let home = URL(fileURLWithPath: LoginEnvironment.homeDirectory, isDirectory: true)
        remove(
            ids: managed["cursor"] ?? [],
            at: home.appendingPathComponent(".cursor/mcp.json"),
            rootKey: "mcpServers"
        )
        remove(
            ids: managed["opencode"] ?? [],
            at: home.appendingPathComponent(".config/opencode/opencode.json"),
            rootKey: "mcp"
        )
        remove(
            ids: managed["gemini"] ?? [],
            at: home.appendingPathComponent(".gemini/settings.json"),
            rootKey: "mcpServers"
        )
        try? manager.removeItem(at: managedURL)
    }

    /// Removes exactly `ids` from the file's `rootKey` object, leaving every
    /// other key alone. A missing or invalid-JSON file is skipped, and the
    /// file is rewritten only when something was removed.
    private static func remove(ids: [String], at url: URL, rootKey: String) {
        guard !ids.isEmpty else { return }
        guard let data = try? Data(contentsOf: url),
              var root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              var entries = root[rootKey] as? [String: Any]
        else { return }
        var removed = false
        for id in ids where entries[id] != nil {
            entries.removeValue(forKey: id)
            removed = true
        }
        guard removed else { return }
        root[rootKey] = entries
        guard let out = try? JSONSerialization.data(withJSONObject: root, options: [.prettyPrinted, .sortedKeys]) else {
            return
        }
        try? out.write(to: url, options: .atomic)
    }

    private static let managedURL = MCPPaths.directory.appendingPathComponent("managed.json")

    /// What an earlier build put in each CLI's config, or nil when the record cannot be read.
    private static func readManaged() -> [String: [String]]? {
        guard let data = try? Data(contentsOf: managedURL),
              let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        var out: [String: [String]] = [:]
        for key in ["cursor", "opencode", "gemini"] {
            out[key] = parsed[key] as? [String] ?? []
        }
        return out
    }
}
