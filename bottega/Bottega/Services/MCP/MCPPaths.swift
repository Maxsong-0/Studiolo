// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Where the config files providers read at launch are written. Each holds every enabled
/// server in that provider's own format; a missing or empty file means no servers. The
/// files sit in the app's own folder on disk (`Storage.root`), so the paths live in this
/// layer, beside the code that writes and reads them, not with the MCP models in Core.
enum MCPPaths {
    static let directory: URL = {
        let url = Storage.root.appendingPathComponent("mcp", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    /// Kept for the Keychain-free settings: `[MCPConnection]` as JSON.
    static let connectionsURL = Storage.root.appendingPathComponent("mcp.json")
    /// The user's own servers: `[MCPCustomServer]` as JSON (secret values never land here).
    static let customsURL = Storage.root.appendingPathComponent("custom-mcp.json")
    /// `{"mcpServers": {...}}` for `claude --mcp-config`.
    static let claudeConfigURL = directory.appendingPathComponent("claude.json")
    /// `{"mcp_servers": {...}}` for Codex's app-server `config` overrides.
    static let codexConfigURL = directory.appendingPathComponent("codex.json")
    /// `{"mcpServers": {...}}` for `copilot --additional-mcp-config`.
    static let copilotConfigURL = directory.appendingPathComponent("copilot.json")
}
