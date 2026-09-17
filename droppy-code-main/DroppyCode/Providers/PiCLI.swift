import Foundation

/// The small surface Droppy Code needs of the Pi coding harness (`pi`, the npm
/// package `@earendil-works/pi-coding-agent`): where its login lives, its model
/// catalog over RPC, and the gate extension that routes tool approvals to the app.
///
/// Pi signs in per provider under `~/.pi/agent/auth.json` (or `PI_CODING_AGENT_DIR`
/// when set); a bare provider API key in the environment also counts as signed in.
enum PiCLI {
    /// Pi's `--thinking` levels, which Droppy's effort strings map to 1:1.
    static let thinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]

    /// Where the gate extension and each session's gate file live.
    static var supportDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: LoginEnvironment.homeDirectory).appendingPathComponent("Library/Application Support")
        return base.appendingPathComponent("Droppy Code/pi", isDirectory: true)
    }

    /// `PI_CODING_AGENT_DIR` when set, else `<home>/.pi/agent`.
    static func configDirectory(environment: [String: String]) -> URL {
        if let directory = environment["PI_CODING_AGENT_DIR"], !directory.isEmpty {
            return URL(fileURLWithPath: directory)
        }
        let home = environment["HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? LoginEnvironment.homeDirectory
        return URL(fileURLWithPath: home).appendingPathComponent(".pi/agent", isDirectory: true)
    }

    /// `auth.json` as an object with at least one key (the keys are provider names
    /// such as anthropic, openai), else any provider API key in the environment.
    static func authStatus(executable: URL, environment: [String: String]) async -> ProviderStatus.Auth {
        let file = configDirectory(environment: environment).appendingPathComponent("auth.json")
        if let data = try? Data(contentsOf: file),
           let json = JSONValue.parse(data),
           let object = json.object, !object.isEmpty {
            return .signedIn(object.keys.sorted().joined(separator: ", "))
        }
        for key in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY", "OPENROUTER_API_KEY", "GROQ_API_KEY", "MISTRAL_API_KEY", "DEEPSEEK_API_KEY", "XAI_API_KEY"] {
            if let value = environment[key], !value.isEmpty {
                return .signedIn(nil)
            }
        }
        return .signedOut
    }

    /// Asks a throwaway `pi --mode rpc` process for its state and model catalog.
    static func listModels(executable: URL, environment: [String: String]) async throws -> [ModelOption] {
        let home = environment["HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? LoginEnvironment.homeDirectory
        var launchEnvironment = environment
        launchEnvironment["PI_SKIP_VERSION_CHECK"] = "1"
        let process = StdioProcess(
            executable: executable,
            arguments: ["--mode", "rpc", "--no-session", "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes", "--no-context-files"],
            directory: URL(fileURLWithPath: home),
            environment: launchEnvironment,
            framing: .lines
        )
        do {
            try process.start()
        } catch {
            throw ProviderError.failed("Pi could not start: \(error.localizedDescription)")
        }
        process.send(["type": "get_state", "id": "state"])
        process.send(["type": "get_available_models", "id": "models"])
        var stateResponse: JSONValue?
        var modelsResponse: JSONValue?
        let timeout = Task {
            try? await Task.sleep(for: .seconds(30))
            process.terminate()
        }
        for await message in process.messages {
            switch message["id"]?.string {
            case "state":
                stateResponse = message
            case "models":
                modelsResponse = message
            default:
                break
            }
            if stateResponse != nil, modelsResponse != nil { break }
        }
        timeout.cancel()
        process.terminate()

        let stateData = stateResponse?["data"] ?? stateResponse ?? .null
        let currentProvider = stateData["model"]?["provider"]?.string
        let currentID = stateData["model"]?["id"]?.string
        let items = modelsResponse?["data"]?["models"]?.array ?? modelsResponse?["models"]?.array ?? []
        var options: [ModelOption] = []
        for item in items {
            guard let provider = item["provider"]?.string, let id = item["id"]?.string else { continue }
            let reasoning = item["reasoning"]?.bool == true
            let fullID = "\(provider)/\(id)"
            let isCurrent: Bool = {
                guard let currentProvider, let currentID else { return false }
                if currentID.contains("/") { return fullID == currentID }
                return provider == currentProvider && id == currentID
            }()
            options.append(ModelOption(
                id: fullID,
                name: item["name"]?.string ?? id,
                detail: provider,
                efforts: reasoning ? thinkingLevels : [],
                defaultEffort: reasoning ? "medium" : nil,
                isDefault: isCurrent
            ))
        }
        guard !options.isEmpty else {
            throw ProviderError.failed("Pi listed no models.")
        }
        return options.sorted { ($0.detail ?? "", $0.name) < ($1.detail ?? "", $1.name) }
    }

    /// Writes the gate extension where `pi --mode rpc` can load it and returns its
    /// path. Rewritten only when its text changed, so a new build's extension
    /// replaces the old one.
    static func installExtension() throws -> URL {
        let directory = supportDirectory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("droppy-gate.ts")
        let data = Data(extensionSource.utf8)
        if (try? Data(contentsOf: file)) != data {
            try data.write(to: file, options: .atomic)
        }
        return file
    }

    /// The gate Droppy Code loads into every RPC session with `-e`. Pi itself keeps
    /// approving everything (`--approve`) or nothing (`--no-approve`); this extension
    /// gates the tools instead: every call that could change something is asked over
    /// `ctx.ui.select`, which in RPC mode surfaces on stdout as an
    /// `extension_ui_request` the app answers. Read-only tools pass straight through.
    /// Supervised asks for all of them, Auto-accept edits waves file changes through,
    /// Auto also passes shell commands that only read, and Full access returns before
    /// asking anything. Plan mode and the final report block the tools outright.
    static let extensionSource: String = #"""
    import fs from "node:fs";

    const readOnly = new Set(["read", "grep", "find", "ls"]);

    const readOnlyBinaries = new Set([
      "ls", "cat", "head", "tail", "grep", "rg", "find", "wc", "pwd", "echo",
      "which", "stat", "file", "du", "tree", "sort", "uniq", "diff",
    ]);
    const readOnlyGit = new Set(["status", "diff", "log", "show", "branch", "blame"]);

    function readGate() {
      try {
        const file = process.env.DROPPY_CODE_PI_GATE || "";
        return JSON.parse(fs.readFileSync(file, "utf8"));
      } catch {
        return {};
      }
    }

    function isReadOnlyCommand(command) {
      const segments = String(command ?? "").split(/&&|\|\||;|\|/);
      return segments.every((segment) => {
        const words = segment.trim().split(/\s+/).filter(Boolean);
        if (words.length === 0) return true;
        if (words[0] === "git") return words.length > 1 && readOnlyGit.has(words[1]);
        return readOnlyBinaries.has(words[0]);
      });
    }

    export default function (pi) {
      pi.on("before_agent_start", async (event, ctx) => {
        const gate = readGate();
        const extra = [];
        if (gate.finalReport) extra.push("Reply with your final report now: no tools, one message.");
        if (gate.interactionMode === "plan") extra.push("You are in plan mode: read-only. Do not edit files or run commands that change anything; investigate, then end with the plan as markdown.");
        if (typeof gate.systemPrompt === "string" && gate.systemPrompt.length > 0) extra.push(gate.systemPrompt);
        if (extra.length === 0) return;
        return { systemPrompt: event.systemPrompt + "\n\n" + extra.join("\n\n") };
      });

      pi.on("tool_call", async (event, ctx) => {
        const gate = readGate();
        if (gate.finalReport) return { block: true, reason: "Final report: no tools" };
        if (readOnly.has(event.toolName)) return;
        if (gate.interactionMode === "plan") return { block: true, reason: "Plan mode is read-only" };
        if (gate.runtimeMode === "fullAccess") return;
        const kind = event.toolName === "bash" ? "command" : (event.toolName === "edit" || event.toolName === "write") ? "fileChange" : "tool";
        if (kind === "fileChange" && (gate.runtimeMode === "autoAcceptEdits" || gate.runtimeMode === "auto")) return;
        if (kind === "command" && gate.runtimeMode === "auto" && isReadOnlyCommand(event.input && event.input.command)) return;
        const input = {};
        for (const [key, value] of Object.entries(event.input || {})) {
          input[key] = typeof value === "string" && value.length > 20000 ? value.slice(0, 20000) : value;
        }
        const payload = { kind, toolName: event.toolName, toolCallId: event.toolCallId, input };
        const answer = await ctx.ui.select("droppy-gate " + JSON.stringify(payload), ["Allow", "Allow for session", "Deny"]);
        if (answer === "Allow" || answer === "Allow for session") return undefined;
        return { block: true, reason: "The user declined this " + (kind === "command" ? "command" : kind === "fileChange" ? "file change" : "tool call") };
      });
    }
    """#
}
