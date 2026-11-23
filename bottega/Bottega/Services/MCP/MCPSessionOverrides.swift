// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

enum MCPSessionOverrides {
    static func copilotUserServerNames() -> [String] {
        let url = URL(fileURLWithPath: LoginEnvironment.homeDirectory)
            .appendingPathComponent(".copilot/mcp-config.json")
        guard let data = try? Data(contentsOf: url),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let servers = root["mcpServers"] as? [String: Any]
        else { return [] }
        return servers.keys.sorted()
    }

    static func copilotArguments() -> [String] {
        guard let json = MCPProviderConfig.copilotConfigJSON() else { return [] }
        var ids = Set<String>()
        if let data = json.data(using: .utf8),
           let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let servers = root["mcpServers"] as? [String: Any] {
            ids = Set(servers.keys)
        }
        var arguments = ["--additional-mcp-config", json]
        for name in copilotUserServerNames() where !ids.contains(name) {
            arguments += ["--disable-mcp-server", name]
        }
        if ids.contains("github") {
            arguments.append("--disable-builtin-mcps")
        }
        return arguments
    }

    static func opencodeEnvironment() -> [String: String] {
        guard let data = try? Data(contentsOf: MCPPaths.claudeConfigURL),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let servers = root["mcpServers"] as? [String: Any], !servers.isEmpty
        else { return [:] }
        var mcp: [String: Any] = [:]
        for (id, value) in servers {
            guard let entry = value as? [String: Any] else { continue }
            if (entry["type"] as? String) == "http" {
                var remote: [String: Any] = [
                    "type": "remote",
                    "url": entry["url"] as? String ?? "",
                    "enabled": true,
                ]
                let headers = stringMap(entry["headers"])
                if !headers.isEmpty { remote["headers"] = headers }
                mcp[id] = remote
            } else {
                guard let command = entry["command"] as? String, !command.isEmpty else { continue }
                let args = stringArray(entry["args"])
                var local: [String: Any] = [
                    "type": "local",
                    "command": [command] + args,
                    "enabled": true,
                ]
                let environment = stringMap(entry["env"])
                if !environment.isEmpty { local["environment"] = environment }
                mcp[id] = local
            }
        }
        guard !mcp.isEmpty else { return [:] }
        let rootObject: [String: Any] = [
            "$schema": "https://opencode.ai/config.json",
            "mcp": mcp,
        ]
        guard let jsonData = try? JSONSerialization.data(withJSONObject: rootObject, options: [.sortedKeys]),
              let json = String(data: jsonData, encoding: .utf8)
        else { return [:] }
        return ["OPENCODE_CONFIG_CONTENT": json]
    }

    private static func stringArray(_ value: Any?) -> [String] {
        (value as? [Any] ?? []).compactMap { $0 as? String }
    }

    private static func stringMap(_ value: Any?) -> [String: String] {
        var result: [String: String] = [:]
        for (key, item) in (value as? [String: Any] ?? [:]) {
            if let text = item as? String { result[key] = text }
        }
        return result
    }
}
