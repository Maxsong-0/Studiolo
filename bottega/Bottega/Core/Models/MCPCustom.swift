// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A server the user defined themselves (Settings › MCP › Custom), kept lean: the
/// values of its variables never land here. Once the server is connected they are
/// fields of its catalog entry, and the secret ones live in the Keychain like any
/// catalog server's (see `MCPCatalogEntry.fields`).
struct MCPCustomServer: Codable, Sendable, Hashable, Identifiable {
    enum Kind: String, Codable, Sendable {
        case stdio
        case http
    }

    /// One environment variable (stdio) or header (http) the server needs. `isSecret`
    /// decides where its value is kept once filled in: the Keychain, or `mcp.json`.
    struct Variable: Codable, Sendable, Hashable, Identifiable {
        var key: String
        var isSecret = false
        var id: String { key }
    }

    /// `custom-` plus the slugged name, so a custom never collides with a catalog id.
    var id: String
    var name: String
    var kind: Kind
    /// stdio: the executable, its arguments, and the environment it needs.
    var command = ""
    var args: [String] = []
    var env: [Variable] = []
    /// http: the streamable-HTTP address and the headers it needs.
    var url = ""
    var headers: [Variable] = []

    private enum CodingKeys: String, CodingKey {
        case id, name, kind, command, args, env, url, headers
    }

    /// Decoded tolerantly: a definition saved before a field existed (or hand-edited)
    /// loads with the missing pieces empty rather than dropping the whole list, and a
    /// variable that cannot be read goes alone rather than taking the server with it.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        kind = try container.decode(Kind.self, forKey: .kind)
        command = try container.decodeIfPresent(String.self, forKey: .command) ?? ""
        args = try container.decodeIfPresent([String].self, forKey: .args) ?? []
        env = container.values(.env)
        url = try container.decodeIfPresent(String.self, forKey: .url) ?? ""
        headers = container.values(.headers)
    }

    init(id: String, name: String, kind: Kind) {
        self.id = id
        self.name = name
        self.kind = kind
    }

    /// The variables of the server's kind: `env` for stdio, `headers` for http.
    var variables: [Variable] { kind == .stdio ? env : headers }

    /// The id a name gets: lowercase, hyphens for anything else, `custom-` in front.
    /// A name with nothing sluggable ("!!!") still gets a usable id.
    static func makeID(_ name: String) -> String {
        var slug = ""
        var lastWasDash = true
        for character in name.lowercased() {
            if character.isLetter || character.isNumber {
                slug.append(character)
                lastWasDash = false
            } else if !lastWasDash {
                slug.append("-")
                lastWasDash = true
            }
        }
        if slug.hasSuffix("-") { slug.removeLast() }
        return "custom-" + (slug.isEmpty ? "server" : slug)
    }

    /// What the server is, in one card line.
    var summary: String {
        switch kind {
        case .stdio:
            let line = ([command] + args).joined(separator: " ")
            return "Runs on this Mac: \(line)."
        case .http:
            return "Remote server at \(URL(string: url)?.host() ?? url)."
        }
    }

    /// The server as a catalog entry, so connecting, probing, the Keychain, the
    /// provider configs and the key panel treat it exactly like a built-in one: its
    /// variables become fields, and the env or headers hold `{key}` placeholders the
    /// resolve fills from the values the user typed.
    func catalogEntry() -> MCPCatalogEntry {
        let fields = variables.map {
            MCPField(key: $0.key, label: $0.key, placeholder: "", kind: $0.isSecret ? .secret : .text)
        }
        // Drafts reach the editor's header before duplicate keys are validated.
        let placeholders = Dictionary(variables.map { ($0.key, "{\($0.key)}") }, uniquingKeysWith: { first, _ in first })
        var entry = MCPCatalogEntry(
            id: id,
            name: name,
            vendor: "Custom",
            summary: summary,
            category: .developer,
            asset: "",
            colorValue: 0,
            isMonochrome: true,
            transport: kind == .stdio
                ? .stdio(command: command, args: args)
                : .http(url: url, headers: placeholders),
            fields: fields,
            docsURL: ""
        )
        if kind == .stdio {
            entry.env = placeholders
        }
        return entry
    }
}

/// What the editor reports when a custom server cannot be saved.
enum MCPCustomError: Error, Equatable {
    case emptyName
    case takenName(String)
    case emptyCommand
    case badURL
    case badVariableKey(String)
    case duplicateVariableKey(String)

    var message: String {
        switch self {
        case .emptyName: "The server needs a name."
        case .takenName(let name): "A server named “\(name)” already exists."
        case .emptyCommand: "A stdio server needs a command."
        case .badURL: "The URL must start with https:// (or http:// for a local server)."
        case .badVariableKey(let key): "“\(key)” is not a valid variable name: letters, digits, `_`, `-` and `.`, starting with a letter or `_`."
        case .duplicateVariableKey(let key): "“\(key)” is listed twice."
        }
    }

    /// Whether a variable key is usable as an env var or header name and as a `{key}`
    /// placeholder: a letter or `_`, then letters, digits, `_`, `-` or `.`.
    static func validVariableKey(_ key: String) -> Bool {
        guard let first = key.first, first.isLetter || first == "_" else { return false }
        return key.allSatisfy { $0.isLetter || $0.isNumber || $0 == "_" || $0 == "-" || $0 == "." }
    }

    /// The server checked for saving: name and kind-specific fields present, variable
    /// keys valid and unique, and the name not taken by the catalog or another custom.
    static func validate(_ server: MCPCustomServer, existingIDs: Set<String>) -> MCPCustomError? {
        let name = server.name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return .emptyName }
        let id = MCPCustomServer.makeID(name)
        guard !existingIDs.contains(id) else { return .takenName(name) }
        switch server.kind {
        case .stdio:
            guard !server.command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return .emptyCommand }
        case .http:
            guard validRemoteURL(server.url) else { return .badURL }
        }
        var seen: Set<String> = []
        for variable in server.variables {
            guard validVariableKey(variable.key) else { return .badVariableKey(variable.key) }
            let key = server.kind == .http ? variable.key.lowercased() : variable.key
            guard seen.insert(key).inserted else { return .duplicateVariableKey(variable.key) }
        }
        return nil
    }

    /// A remote address a streamable-HTTP server may use: https anywhere, http only on
    /// this Mac (localhost, 127.0.0.1, ::1), and never empty or with no host.
    static func validRemoteURL(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = URLComponents(string: trimmed),
              let scheme = parsed.scheme?.lowercased(),
              let host = parsed.host, !host.isEmpty else { return false }
        guard scheme == "https" || scheme == "http" else { return false }
        if scheme == "http" {
            return ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host.lowercased())
        }
        return true
    }
}
