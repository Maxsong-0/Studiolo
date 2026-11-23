// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

enum MCPProviderConfig {
    static func claude(_ servers: [MCPResolvedServer]) -> JSONValue {
        var entries: [String: JSONValue] = [:]
        for server in servers {
            if server.isRemote {
                var entry: [String: JSONValue] = [
                    "type": "http",
                    "url": .string(server.url ?? ""),
                ]
                if !server.headers.isEmpty {
                    entry["headers"] = .object(server.headers.mapValues(JSONValue.string))
                }
                entries[server.id] = .object(entry)
            } else {
                var entry: [String: JSONValue] = [
                    "command": .string(server.command ?? ""),
                    "args": .array(server.args.map(JSONValue.string)),
                ]
                if !server.env.isEmpty {
                    entry["env"] = .object(server.env.mapValues(JSONValue.string))
                }
                entries[server.id] = .object(entry)
            }
        }
        return .object(["mcpServers": .object(entries)])
    }

    static func codex(_ servers: [MCPResolvedServer]) -> JSONValue {
        var entries: [String: JSONValue] = [:]
        for server in servers {
            if server.isRemote {
                var entry: [String: JSONValue] = ["url": .string(server.url ?? "")]
                if !server.headers.isEmpty {
                    entry["http_headers"] = .object(server.headers.mapValues(JSONValue.string))
                }
                entries[server.id] = .object(entry)
            } else {
                var entry: [String: JSONValue] = [
                    "command": .string(server.command ?? ""),
                    "args": .array(server.args.map(JSONValue.string)),
                ]
                if !server.env.isEmpty {
                    entry["env"] = .object(server.env.mapValues(JSONValue.string))
                }
                entries[server.id] = .object(entry)
            }
        }
        return .object(["mcp_servers": .object(entries)])
    }

    static func copilot(_ servers: [MCPResolvedServer]) -> JSONValue {
        var entries: [String: JSONValue] = [:]
        for server in servers {
            if server.isRemote {
                entries[server.id] = .object([
                    "type": "http",
                    "url": .string(server.url ?? ""),
                    "headers": .object(server.headers.mapValues(JSONValue.string)),
                    "tools": .array([.string("*")]),
                ])
            } else {
                let entry: [String: JSONValue] = [
                    "type": "local",
                    "command": .string(server.command ?? ""),
                    "args": .array(server.args.map(JSONValue.string)),
                    "env": .object(server.env.mapValues(JSONValue.string)),
                    "tools": .array([.string("*")]),
                ]
                entries[server.id] = .object(entry)
            }
        }
        return .object(["mcpServers": .object(entries)])
    }

    static func acpServers(allowsHTTP: Bool) -> JSONValue {
        guard let data = try? Data(contentsOf: MCPPaths.claudeConfigURL),
              let parsed = JSONValue.parse(data),
              let object = parsed["mcpServers"]?.object, !object.isEmpty
        else { return .array([]) }
        var servers: [JSONValue] = []
        for id in object.keys.sorted() {
            let entry = object[id] ?? .null
            if entry["type"]?.string == "http" || entry["url"]?.string != nil {
                let url = entry["url"]?.string ?? ""
                let headers = entry["headers"]?.object ?? [:]
                let headerList = headers.keys.sorted().map { key in
                    JSONValue.object(["name": .string(key), "value": .string(headers[key]?.string ?? "")])
                }
                if allowsHTTP {
                    servers.append(.object([
                        "type": .string("http"),
                        "name": .string(id),
                        "url": .string(url),
                        "headers": .array(headerList),
                    ]))
                } else {
                    var args: [JSONValue] = [.string("-y"), .string("mcp-remote"), .string(url)]
                    for key in headers.keys.sorted() {
                        args += [.string("--header"), .string("\(key): \(headers[key]?.string ?? "")")]
                    }
                    servers.append(.object([
                        "name": .string(id),
                        "command": .string("npx"),
                        "args": .array(args),
                        "env": .array([]),
                    ]))
                }
            } else {
                let env = entry["env"]?.object ?? [:]
                let envList = env.keys.sorted().map { key in
                    JSONValue.object(["name": .string(key), "value": .string(env[key]?.string ?? "")])
                }
                servers.append(.object([
                    "name": .string(id),
                    "command": .string(entry["command"]?.string ?? ""),
                    "args": .array(entry["args"]?.array ?? []),
                    "env": .array(envList),
                ]))
            }
        }
        return .array(servers)
    }

    static func gemini(_ servers: [MCPResolvedServer]) -> JSONValue {
        var entries: [String: JSONValue] = [:]
        for server in servers {
            if server.isRemote {
                entries[server.id] = .object([
                    "httpUrl": .string(server.url ?? ""),
                    "headers": .object(server.headers.mapValues(JSONValue.string)),
                ])
            } else {
                var entry: [String: JSONValue] = [
                    "command": .string(server.command ?? ""),
                    "args": .array(server.args.map(JSONValue.string)),
                ]
                if !server.env.isEmpty {
                    entry["env"] = .object(server.env.mapValues(JSONValue.string))
                }
                entries[server.id] = .object(entry)
            }
        }
        return .object(["mcpServers": .object(entries)])
    }

    static func writeAll(_ servers: [MCPResolvedServer]) {
        if servers.isEmpty {
            for url in [MCPPaths.claudeConfigURL, MCPPaths.codexConfigURL, MCPPaths.copilotConfigURL, geminiSettingsURL] {
                try? FileManager.default.removeItem(at: url)
            }
            return
        }
        write(claude(servers), to: MCPPaths.claudeConfigURL)
        write(codex(servers), to: MCPPaths.codexConfigURL)
        write(copilot(servers), to: MCPPaths.copilotConfigURL)
        write(gemini(servers), to: geminiSettingsURL)
    }

    // An entry here switches that server off for the session by name. Only the server's own name may be used,
    // because a nested table's path (`imap-email.env`) names an entry that exists nowhere in the user's config and makes Codex reject its whole configuration, which fails every turn at once.
    static func codexUserServerNames() -> [String] {
        let url = URL(fileURLWithPath: LoginEnvironment.homeDirectory).appendingPathComponent(".codex/config.toml")
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return [] }
        var names = Set<String>()
        for rawLine in text.components(separatedBy: .newlines) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix("[mcp_servers.") else { continue }
            var rest = String(line.dropFirst("[mcp_servers.".count))
            guard let end = rest.firstIndex(of: "]") else { continue }
            let tail = String(rest[rest.index(after: end)...]).trimmingCharacters(in: .whitespaces)
            guard tail.isEmpty || tail.hasPrefix("#") else { continue }
            rest = String(rest[..<end])
            // A quoted first segment is the name literally, in either TOML quote; an
            // unquoted one is the server's name only up to the first dot, which is what
            // a nested table (`[mcp_servers.imap-email.env]`) splits on.
            let quote: Character? = rest.hasPrefix("'") ? "'" : (rest.hasPrefix("\"") ? "\"" : nil)
            var name: String
            if let quote {
                let remainder = rest.dropFirst()
                guard let close = remainder.firstIndex(of: quote) else { continue }
                name = String(remainder[..<close])
            } else if let dot = rest.firstIndex(of: ".") {
                name = String(rest[..<dot])
            } else {
                name = rest
            }
            name = name.trimmingCharacters(in: .whitespaces)
            // Only a plain server name goes out: the provider applies this object as a
            // config patch, where a dot or a quote is read as a path, which mints the
            // very entry that made it reject its whole configuration.
            guard !name.isEmpty, !name.contains(where: { $0.isWhitespace || $0 == "[" || $0 == "]" || $0 == "'" || $0 == "\"" || $0 == "," || $0 == "." }) else { continue }
            names.insert(name)
        }
        return names.sorted()
    }

    static func codexOverrides() -> JSONValue? {
        guard let data = try? Data(contentsOf: MCPPaths.codexConfigURL),
              let parsed = JSONValue.parse(data),
              var object = parsed["mcp_servers"]?.object, !object.isEmpty
        else { return nil }
        // With servers connected in Settings they are the only ones the session sees, so the user's own Codex servers are switched off for this session by name.
        // (`enabled = false` is a config override, the file itself is never touched.)
        for name in codexUserServerNames() where object[name] == nil {
            object[name] = .object(["enabled": .bool(false)])
        }
        return .object(object)
    }

    static func claudeConfigPath() -> String? {
        guard serversPresent(in: MCPPaths.claudeConfigURL, key: "mcpServers") else { return nil }
        return MCPPaths.claudeConfigURL.path
    }

    static func copilotConfigJSON() -> String? {
        guard let data = try? Data(contentsOf: MCPPaths.copilotConfigURL),
              let parsed = JSONValue.parse(data),
              let object = parsed["mcpServers"]?.object, !object.isEmpty
        else { return nil }
        return parsed.compactString
    }

    static func geminiSettingsPath() -> String? {
        guard serversPresent(in: geminiSettingsURL, key: "mcpServers") else { return nil }
        return geminiSettingsURL.path
    }

    static func connectedIDs() -> [String] {
        guard let data = try? Data(contentsOf: MCPPaths.claudeConfigURL),
              let parsed = JSONValue.parse(data),
              let object = parsed["mcpServers"]?.object, !object.isEmpty
        else { return [] }
        return object.keys.sorted()
    }

    private static let geminiSettingsURL = MCPPaths.directory.appendingPathComponent("gemini-settings.json")

    private static func write(_ value: JSONValue, to url: URL) {
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? value.data(pretty: true).write(to: url, options: .atomic)
    }

    private static func serversPresent(in url: URL, key: String) -> Bool {
        guard let data = try? Data(contentsOf: url),
              let parsed = JSONValue.parse(data),
              let object = parsed[key]?.object, !object.isEmpty
        else { return false }
        return true
    }
}
