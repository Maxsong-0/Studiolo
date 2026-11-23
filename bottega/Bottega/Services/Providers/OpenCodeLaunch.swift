// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// How OpenCode is launched so that the runtime mode means something.
///
/// OpenCode's own rules start from `"*": "allow"`, so left alone it never asks: Supervised,
/// Auto-accept edits and Auto would all run as Full access. The rules below are handed to it
/// through `OPENCODE_CONFIG_CONTENT`, the channel the MCP servers already ride in on, and the
/// questions come back as `session/request_permission` like any other ACP agent's.
///
/// Four things about OpenCode 1.3.0 shape the rules (measured against a live `opencode acp`;
/// see `03-记忆/opencode权限与ACP的实测事实.md`):
/// - A global rule is overridden by an agent's own, and the last matching rule wins, so the
///   rules are written per agent as well as globally. A repo's own `opencode.json` can still
///   define agents of its own that these do not name.
/// - A global `edit: ask` would turn the plan agent's outright refusal of edits into a
///   question, so the plan agent gets its refusal back.
/// - A question raised inside a subagent never reaches the client (its ACP server drops
///   requests from sessions it did not open) and the whole turn hangs. Subagents therefore
///   refuse what the mode asks about instead of asking.
/// - The rules are read when the process starts, so a mode change has to restart the session.
///   `ThreadRuntime` keeps the mode in the session signature for that.
enum OpenCodeLaunch {
    private static let contentKey = "OPENCODE_CONFIG_CONTENT"
    /// Merged over every other source of global rules, so a value inherited from the login
    /// shell would quietly undo the rules below.
    private static let permissionKey = "OPENCODE_PERMISSION"

    /// What `mode` makes OpenCode ask about, in OpenCode's names for its permissions; nil for
    /// Full access, which leaves OpenCode as it is.
    ///
    /// Auto asks like Supervised: its summary promises a reviewer on the provider's side, and
    /// OpenCode has none, so it falls back to the stricter reading.
    static func askedTools(for mode: RuntimeMode) -> [String]? {
        switch mode {
        case .supervised, .auto: ["edit", "bash"]
        case .autoAcceptEdits: ["bash"]
        case .fullAccess: nil
        }
    }

    /// The config that makes OpenCode ask about `tools`.
    static func permissionConfig(asking tools: [String]) -> [String: Any] {
        var global: [String: Any] = [:]
        var build: [String: Any] = [:]
        var plan: [String: Any] = [:]
        var subagent: [String: Any] = [:]
        for tool in tools {
            global[tool] = "ask"
            build[tool] = "ask"
            subagent[tool] = "deny"
            plan[tool] = tool == "edit" ? ["*": "deny"] : "ask"
        }
        return [
            "permission": global,
            "agent": [
                "build": ["permission": build],
                "plan": ["permission": plan],
                "general": ["permission": subagent],
                "explore": ["permission": subagent],
            ],
        ]
    }

    /// `environment` as OpenCode should be launched with for `mode`: the permission config
    /// merged into whatever `OPENCODE_CONFIG_CONTENT` already carries (the MCP servers), the
    /// rules winning where they meet. Throws when the rules cannot be applied, and the launch
    /// stops there: starting anyway would run OpenCode without the questions the mode promises.
    static func environment(_ environment: [String: String], mode: RuntimeMode) throws -> [String: String] {
        guard let tools = askedTools(for: mode) else { return environment }
        var result = environment
        result[permissionKey] = nil

        var config: [String: Any] = [:]
        if let existing = environment[contentKey], !existing.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            guard let data = existing.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { throw refusal(mode, "\(contentKey) is not a JSON object") }
            config = object
        }
        let merged = merged(config, with: permissionConfig(asking: tools))
        guard let data = try? JSONSerialization.data(withJSONObject: merged, options: [.sortedKeys]),
              let json = String(data: data, encoding: .utf8)
        else { throw refusal(mode, "the rules could not be encoded") }
        result[contentKey] = json
        return result
    }

    /// `overlay` laid over `base`, key by key down through nested objects; `overlay` wins.
    private static func merged(_ base: [String: Any], with overlay: [String: Any]) -> [String: Any] {
        var result = base
        for (key, value) in overlay {
            if let new = value as? [String: Any], let old = result[key] as? [String: Any] {
                result[key] = merged(old, with: new)
            } else {
                result[key] = value
            }
        }
        return result
    }

    private static func refusal(_ mode: RuntimeMode, _ reason: String) -> ProviderError {
        .failed("OpenCode was not started: \(mode.title) mode needs permission rules that could not be applied (\(reason)).")
    }
}
