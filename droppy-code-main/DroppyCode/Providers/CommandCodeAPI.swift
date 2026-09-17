import Foundation

/// The lightweight side of Command Code: where its login lives, the account's plan and
/// credits from `api.commandcode.ai`, the model catalog from `cmd --list-models`, and the
/// tables the CLI itself uses for reasoning efforts and context windows.
///
/// The CLI signs in with `cmd login` (a browser OAuth flow) and keeps the resulting API
/// key in `~/.commandcode/auth.json`; `COMMAND_CODE_API_KEY` in the environment takes
/// precedence, which is how a key pasted into Settings reaches both the CLI and these
/// reads. The `/alpha/…` routes are the ones the CLI's own `/usage` calls (verified live
/// against `command-code 1.53.1`).
enum CommandCodeAPI {
    static let baseURL = URL(string: "https://api.commandcode.ai")!
    static let billingURL = URL(string: "https://commandcode.ai/billing")!

    // MARK: - Login

    /// The key the CLI would use: the environment's, else the stored login's.
    static func apiKey(environment: [String: String]) -> String? {
        if let key = environment["COMMAND_CODE_API_KEY"]?.trimmingCharacters(in: .whitespacesAndNewlines), !key.isEmpty {
            return key
        }
        return storedLogin(environment: environment)?.apiKey
    }

    struct StoredLogin {
        var apiKey: String
        var userName: String?
    }

    /// `~/.commandcode/auth.json`, as `cmd login` writes it: `apiKey`, `userId`, `userName`,
    /// `keyName`, `authenticatedAt`.
    static func storedLogin(environment: [String: String]) -> StoredLogin? {
        let home = environment["HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? LoginEnvironment.homeDirectory
        let file = URL(fileURLWithPath: home).appendingPathComponent(".commandcode/auth.json")
        guard let data = try? Data(contentsOf: file), let json = JSONValue.parse(data),
              let key = json["apiKey"]?.string?.trimmingCharacters(in: .whitespacesAndNewlines), !key.isEmpty else { return nil }
        return StoredLogin(apiKey: key, userName: json["userName"]?.string ?? json["name"]?.string)
    }

    /// `~/.commandcode/config.json`: the model and efforts the user picked in the CLI, so
    /// a fresh Droppy Code thread starts on the same model the terminal would.
    static func userConfig(environment: [String: String]) -> JSONValue? {
        let home = environment["HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? LoginEnvironment.homeDirectory
        let file = URL(fileURLWithPath: home).appendingPathComponent(".commandcode/config.json")
        guard let data = try? Data(contentsOf: file) else { return nil }
        return JSONValue.parse(data)
    }

    /// `cmd status` checks the stored login against the API: "Authenticated as <name>" or
    /// "Not authenticated" (exit code 3). With no login stored at all the CLI is not asked.
    static func authStatus(executable: URL, environment: [String: String]) async -> ProviderStatus.Auth {
        guard apiKey(environment: environment) != nil else { return .signedOut }
        guard let result = try? await Shell.run(executable, ["status", "--no-auto-update"], environment: environment, timeout: 25) else {
            return .unknown
        }
        let text = TextCleanup.stripANSI(result.output + result.errorOutput)
        if let match = text.firstMatch(of: #/Authenticated as (\S+)/#) {
            return .signedIn(String(match.output.1))
        }
        if result.status == 3 || text.localizedCaseInsensitiveContains("not authenticated")
            || text.localizedCaseInsensitiveContains("not logged in") {
            return .signedOut
        }
        if text.localizedCaseInsensitiveContains("authentication verified") {
            return .signedIn(storedLogin(environment: environment)?.userName)
        }
        return .unknown
    }

    // MARK: - Plan and credits

    /// The account's rolling windows and monthly plan, from `/alpha/billing/credits` and
    /// `/alpha/billing/subscriptions`. The windows cap dollars rather than requests: a Go
    /// plan allows $3 per five hours, a Pro plan $16, and on-demand credits are never
    /// throttled. `used` and `cap` come as dollars; `resetAt` is epoch milliseconds, 0 while
    /// the window has not started.
    static func planLimits(environment: [String: String]) async -> PlanLimits? {
        guard let key = apiKey(environment: environment) else { return nil }
        async let creditsFetch = fetch("/alpha/billing/credits", key: key)
        async let subscriptionFetch = fetch("/alpha/billing/subscriptions", key: key)
        let credits = await creditsFetch
        let subscription = await subscriptionFetch
        guard let credits else { return nil }
        var windows: [PlanLimits.Window] = []
        let limits = credits["windowLimits"] ?? .null
        for (key, title) in [("fiveHour", "5-hour limit"), ("weekly", "Weekly")] {
            guard let window = limits[key], !window.isNull, let cap = window["cap"]?.double, cap > 0 else { continue }
            let used = window["used"]?.double ?? 0
            let resetAt = window["resetAt"]?.double.flatMap { $0 > 0 ? Date(timeIntervalSince1970: $0 / 1_000) : nil }
            windows.append(PlanLimits.Window(
                id: key,
                title: "\(title) · \(ProviderCredits.money(cap, currency: "USD"))",
                percent: min(100, used / cap * 100),
                resetsAt: resetAt
            ))
        }
        let planName = planName(subscription?["data"]?["planId"]?.string)
        guard !windows.isEmpty || planName != nil else { return nil }
        return PlanLimits(planName: planName, windows: windows)
    }

    /// What is left to spend: the plan's monthly credits, credits bought on demand and
    /// promotional ones, all in dollars. `belowThreshold` is the API saying the account
    /// can no longer be charged.
    static func credits(environment: [String: String]) async -> ProviderCredits? {
        guard let key = apiKey(environment: environment) else { return nil }
        return await credits(apiKey: key)
    }

    static func credits(apiKey key: String) async -> ProviderCredits? {
        guard let json = await fetch("/alpha/billing/credits", key: key),
              let credits = json["credits"], !credits.isNull else { return nil }
        let monthly = credits["monthlyCredits"]?.double ?? 0
        let purchased = credits["purchasedCredits"]?.double ?? 0
        let free = credits["freeCredits"]?.double ?? 0
        return ProviderCredits(
            total: monthly + purchased + free,
            currency: "USD",
            toppedUp: purchased,
            granted: free,
            monthly: monthly,
            isUsable: credits["belowThreshold"]?.bool != true
        )
    }

    /// "GOAT" from `individual-goat`, "Max 20×" from `individual-max-20x`, "Teams Pro" from
    /// `teams-pro`: the plan names commandcode.ai/pricing uses.
    static func planName(_ planID: String?) -> String? {
        guard let planID, !planID.isEmpty else { return nil }
        var parts = planID.split(separator: "-").map(String.init)
        if parts.first == "individual" { parts.removeFirst() }
        guard !parts.isEmpty else { return nil }
        return parts.map { part -> String in
            switch part.lowercased() {
            case "goat": return "GOAT"
            case "pro": return "Pro"
            case "go": return "Go"
            case "max": return "Max"
            case "ultra": return "Ultra"
            case "teams", "team": return "Teams"
            default:
                if let multiplier = part.firstMatch(of: #/^(\d+)x$/#) { return "\(multiplier.output.1)×" }
                return part.prefix(1).uppercased() + part.dropFirst()
            }
        }.joined(separator: " ")
    }

    private static func fetch(_ path: String, key: String) async -> JSONValue? {
        var request = URLRequest(url: baseURL.appendingPathComponent(path), timeoutInterval: 15)
        request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200 else { return nil }
        return JSONValue.parse(data)
    }

    // MARK: - Models

    /// `cmd --list-models`: section headers ("Anthropic", "Open Source"), then one model per
    /// line as `<id>  <description>`, some tagged "(default)" or "(recommended)". The CLI's
    /// own chosen model from `config.json` leads the list, so a thread starts where the
    /// terminal would; otherwise the recommended row does.
    static func listModels(executable: URL, environment: [String: String]) async throws -> [ModelOption] {
        let result = try await Shell.run(executable, ["--list-models", "--no-auto-update"], environment: environment, timeout: 30)
        let text = TextCleanup.stripANSI(result.output)
        let configured = userConfig(environment: environment)
        let list = parseModels(text, configuredModel: configured?["model"]?.string, configuredEfforts: configured?["reasoningEffort"])
        guard !list.isEmpty else {
            throw ProviderError.failed(TextCleanup.lastLines(result.errorOutput + result.output) ?? "Command Code listed no models.")
        }
        return list
    }

    static func parseModels(_ text: String, configuredModel: String?, configuredEfforts: JSONValue?) -> [ModelOption] {
        var options: [ModelOption] = []
        var section: String?
        for rawLine in text.components(separatedBy: .newlines) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty else { continue }
            if line.hasPrefix("Available models") || line.hasPrefix("Pass the full id") || line.hasPrefix("cmd ")
                || line.hasPrefix("Docs:") || line.hasPrefix("Usage") { continue }
            guard let match = line.firstMatch(of: #/^(\S+)\s{2,}(.+)$/#) else {
                // A line without a two-space gap is a section header, such as "Anthropic".
                if !line.contains(" "), line.first?.isUppercase == true { section = line } else if line.count < 24 { section = line }
                continue
            }
            let id = String(match.output.1)
            guard id.contains("-") || id.contains("/") else { continue }
            var detail = String(match.output.2).trimmingCharacters(in: .whitespaces)
            var isRecommended = false
            var isCLIDefault = false
            if let range = detail.range(of: " (recommended)") { detail.removeSubrange(range); isRecommended = true }
            if let range = detail.range(of: " (default)") { detail.removeSubrange(range); isCLIDefault = true }
            var option = ModelOption(
                id: id,
                name: displayName(for: id),
                detail: [section, detail.isEmpty ? nil : detail].compactMap { $0 }.joined(separator: " · "),
                efforts: efforts(for: id),
                defaultEffort: configuredEfforts?[id]?.string,
                isDefault: isRecommended || isCLIDefault
            )
            if let configuredModel, matches(id, configuredModel) { option.isDefault = true }
            options.append(option)
        }
        // One default only: the CLI's configured model wins, then the recommended row.
        if let configuredModel, let index = options.firstIndex(where: { matches($0.id, configuredModel) }) {
            for i in options.indices { options[i].isDefault = i == index }
            let chosen = options.remove(at: index)
            options.insert(chosen, at: 0)
        } else if let index = options.firstIndex(where: \.isDefault) {
            for i in options.indices { options[i].isDefault = i == index }
        }
        return options
    }

    /// The CLI matches models case-insensitively by full id or by the name after the slash.
    static func matches(_ id: String, _ query: String) -> Bool {
        let lhs = id.lowercased()
        let rhs = query.lowercased()
        guard lhs != rhs else { return true }
        let shortLHS = lhs.split(separator: "/").last.map(String.init) ?? lhs
        let shortRHS = rhs.split(separator: "/").last.map(String.init) ?? rhs
        return shortLHS == shortRHS
    }

    /// "Claude Sonnet 5" from `claude-sonnet-5`, "Kimi K2.7 Code" from `moonshotai/kimi-k2.7-code`,
    /// "GPT-5.6 Terra" from `gpt-5.6-terra`. The vendor prefix goes; the chip strips vendor
    /// words of its own.
    static func displayName(for id: String) -> String {
        let short = id.split(separator: "/").last.map(String.init) ?? id
        let base = short.split(separator: ":").first.map(String.init) ?? short
        let suffix = short.contains(":free") ? " Free" : ""
        var tokens = base.split(separator: "-").map(String.init)
        // Trailing version pairs read as one number: `claude-fable-5-1` is Fable 5.1.
        var index = 0
        while index + 1 < tokens.count {
            if tokens[index].allSatisfy(\.isNumber), tokens[index + 1].allSatisfy(\.isNumber) {
                tokens[index] += "." + tokens[index + 1]
                tokens.remove(at: index + 1)
            } else {
                index += 1
            }
        }
        let words = tokens.map { token -> String in
            switch token.lowercased() {
            case "gpt": return "GPT"
            case "glm": return "GLM"
            case "hy3", "hy4": return token.uppercased()
            case "v4", "v4.1", "v2.5": return token.uppercased()
            default:
                if token.first?.isNumber == true { return token }
                return token.prefix(1).uppercased() + token.dropFirst()
            }
        }
        if words.first == "GPT", words.count > 1 {
            return "GPT-" + words.dropFirst().joined(separator: " ") + suffix
        }
        return words.joined(separator: " ") + suffix
    }

    /// The reasoning efforts each model takes, from the CLI's own table (`cmd 1.53.1`).
    /// Models not in it take none, so the slider stays out of the way.
    static func efforts(for id: String) -> [String] {
        let full = ["low", "medium", "high", "xhigh", "max"]
        let toXHigh = ["low", "medium", "high", "xhigh"]
        let toHigh = ["low", "medium", "high"]
        let highMax = ["high", "max"]
        let lowHighMax = ["low", "high", "max"]
        let table: [String: [String]] = [
            "claude-sonnet-5": full, "claude-sonnet-4-6": full, "claude-fable-5-1": full, "claude-fable-5": full,
            "claude-opus-5": full, "claude-opus-4-8": full, "claude-opus-4-7": full,
            "gpt-6-astra": full, "gpt-5.6-sol": full, "gpt-5.6-terra": full, "gpt-5.6-luna": full,
            "gpt-5.5": toXHigh, "gpt-5.4": toXHigh, "gpt-5.3-codex": toXHigh, "gpt-5.4-mini": toHigh,
            "deepseek/deepseek-v4-pro": highMax, "deepseek/deepseek-v4-flash": highMax,
            "deepseek/deepseek-v4-flash-vision-exp": highMax, "deepseek/deepseek-v4-flash-fast": lowHighMax,
            "deepseek/deepseek-v4.1-flash": lowHighMax,
            "moonshotai/kimi-k3": lowHighMax, "zai-org/glm-5.3": lowHighMax, "z-ai/glm-5.3-flash": lowHighMax,
            "zai-org/glm-5.2": highMax,
            "google/gemini-3.8-flash": toHigh, "google/gemini-3.7-flash": toHigh, "google/gemini-3.6-flash": toHigh,
            "google/gemini-3.5-flash": toHigh, "google/gemini-3.5-flash-lite": toHigh, "google/gemini-3.1-flash-lite": toHigh,
            "tencent/hy4-preview": toHigh, "sakana/fugu-ultra": ["high", "xhigh"],
            "xai/grok-4.5": toHigh, "xai/grok-4.6": toXHigh,
            "qwen/qwen3.8-max-0902": ["low", "medium", "xhigh"], "qwen/qwen3.8-max": ["low", "medium", "xhigh"],
            "qwen/qwen3.8-27b": ["low", "medium", "xhigh"], "qwen/qwen3.8-flash": ["low", "medium", "xhigh"],
            "meta/muse-spark-1.1": toXHigh, "meta/muse-spark-1.2": toXHigh, "meta/muse-spark-1.2-contributor": toXHigh,
            "meta/muse-spark-1.3": full, "meta/muse-spark-1.3-contributor": toXHigh,
            "minimaxai/minimax-m3": toHigh, "minimax/minimax-m3-free": toHigh,
        ]
        return table[id.lowercased()] ?? []
    }

    /// Context windows, from the CLI's own table, so the context meter has a ceiling.
    static func contextWindow(for id: String?) -> Int? {
        guard let id else { return nil }
        let table: [String: Int] = [
            "claude-sonnet-5": 1_000_000, "claude-sonnet-4-6": 1_000_000, "claude-fable-5-1": 1_000_000, "claude-fable-5": 1_000_000,
            "claude-opus-5": 1_000_000, "claude-opus-4-8": 1_000_000, "claude-opus-4-7": 1_000_000, "claude-haiku-4-5": 200_000,
            "claude-haiku-4-5-20251001": 200_000,
            "gpt-6-astra": 1_050_000, "gpt-5.6-sol": 1_050_000, "gpt-5.6-terra": 1_050_000, "gpt-5.6-luna": 1_050_000,
            "gpt-5.5": 400_000, "gpt-5.4": 400_000, "gpt-5.3-codex": 400_000, "gpt-5.4-mini": 400_000,
            "deepseek/deepseek-v4-pro": 1_000_000, "deepseek/deepseek-v4-flash": 1_000_000, "deepseek/deepseek-v4-flash-vision-exp": 1_000_000,
            "deepseek/deepseek-v4-flash-fast": 1_000_000, "deepseek/deepseek-v4.1-flash": 1_000_000,
            "moonshotai/kimi-k3": 1_000_000, "moonshotai/kimi-k2.7-code": 256_000, "moonshotai/kimi-k2.7-code-highspeed": 262_000,
            "moonshotai/kimi-k2.6": 256_000, "moonshotai/kimi-k2.5": 256_000,
            "zai-org/glm-5.3": 1_000_000, "z-ai/glm-5.3-flash": 1_048_576, "zai-org/glm-5.2": 1_000_000, "zai-org/glm-5.2-fast": 1_000_000,
            "zai-org/glm-5": 200_000, "zai-org/glm-5.1": 200_000,
            "minimaxai/minimax-m3": 1_000_000, "minimaxai/minimax-m2.7": 197_000, "minimaxai/minimax-m2.5": 200_000,
            "xiaomi/mimo-v2.5-pro": 1_000_000, "xiaomi/mimo-v2.5": 1_000_000,
            "qwen/qwen3.7-max": 1_000_000, "qwen/qwen3.7-plus": 1_000_000, "qwen/qwen3.8-max-0902": 1_000_000, "qwen/qwen3.8-max": 1_000_000,
            "qwen/qwen3.8-27b": 262_144, "qwen/qwen3.8-flash": 1_000_000, "qwen/qwen3.7-flash": 1_000_000,
            "meituan/longcat-2.0:free": 1_048_576, "stepfun/step-3.7-flash": 256_000, "stepfun/step-3.5-flash": 1_000_000,
            "tencent/hy4-preview": 1_048_576, "tencent/hy3-paid": 262_144,
            "google/gemini-3.5-flash": 1_000_000, "google/gemini-3.8-flash": 1_000_000, "google/gemini-3.7-flash": 1_048_576,
            "google/gemini-3.6-flash": 1_000_000, "google/gemini-3.5-flash-lite": 1_000_000, "google/gemini-3.1-flash-lite": 1_000_000,
            "thinkingmachines/inkling": 256_000, "thinkingmachines/inkling-small": 1_000_000,
            "poolside/laguna-s-2.1-free": 256_000, "inclusionai/ling-3.0-flash-sante:free": 262_144,
            "sakana/fugu-ultra": 1_000_000, "xai/grok-4.5": 500_000, "xai/grok-4.6": 500_000,
            "meta/muse-spark-1.1": 1_048_576, "meta/muse-spark-1.2": 1_048_576, "meta/muse-spark-1.2-contributor": 1_048_576,
            "meta/muse-spark-1.3": 1_048_576, "meta/muse-spark-1.3-contributor": 1_048_576,
            "nvidia/nemotron-3-ultra-550b-a55b": 1_000_000,
        ]
        let lowered = id.lowercased()
        if let window = table[lowered] { return window }
        return table.first { matches($0.key, lowered) }?.value
    }

    /// A starting catalog for a Mac that has not listed models yet: the recommended
    /// Anthropic row, the CLI's own default, and a spread of what the account can run.
    static let seed: [ModelOption] = [
        ModelOption(id: "claude-sonnet-5", name: "Claude Sonnet 5", detail: "Anthropic · best combo of speed & intelligence", efforts: efforts(for: "claude-sonnet-5"), isDefault: true),
        ModelOption(id: "claude-fable-5-1", name: "Claude Fable 5.1", detail: "Anthropic · most capable for demanding reasoning & long-horizon agents", efforts: efforts(for: "claude-fable-5-1")),
        ModelOption(id: "claude-opus-5", name: "Claude Opus 5", detail: "Anthropic · most intelligent Opus for agents and coding", efforts: efforts(for: "claude-opus-5")),
        ModelOption(id: "gpt-6-astra", name: "GPT-6 Astra", detail: "OpenAI · most capable OpenAI model for demanding reasoning & agents", efforts: efforts(for: "gpt-6-astra")),
        ModelOption(id: "gpt-5.6-sol", name: "GPT-5.6 Sol", detail: "OpenAI · latest frontier model for general complex work", efforts: efforts(for: "gpt-5.6-sol")),
        ModelOption(id: "google/gemini-3.8-flash", name: "Gemini 3.8 Flash", detail: "Google · newest Gemini Flash, improved core reasoning", efforts: efforts(for: "google/gemini-3.8-flash")),
        ModelOption(id: "deepseek/deepseek-v4-flash", name: "DeepSeek V4 Flash", detail: "Open Source · fast hybrid-attention reasoning", efforts: efforts(for: "deepseek/deepseek-v4-flash")),
        ModelOption(id: "moonshotai/kimi-k3", name: "Kimi K3", detail: "Open Source · long-horizon coding & knowledge work with 1M context", efforts: efforts(for: "moonshotai/kimi-k3")),
        ModelOption(id: "zai-org/glm-5.3", name: "GLM 5.3", detail: "Open Source · frontier coding with emergent cyber capabilities", efforts: efforts(for: "zai-org/glm-5.3")),
    ]

    // MARK: - The approval mod

    /// The session mod Droppy Code loads into every headless run with `--mod`. Headless
    /// runs cannot prompt, so the CLI gets `--yolo` and this mod gates the tools instead:
    /// every call that could change something goes to Droppy Code as a request file in
    /// the run's approval directory, and the mod waits for the reply file the app writes
    /// back. Read-only tools pass straight through. With no directory in the environment
    /// (Full access) the mod registers nothing.
    ///
    /// Mods are TypeScript, compiled by the CLI at load; `cmd.hooks({beforeToolCall})` is
    /// the documented way to block a tool (commandcode.ai/docs/mods).
    static let approvalMod = #"""
    import fs from 'node:fs'
    import path from 'node:path'

    // Tools that only look: they never wait on the user.
    const READ_ONLY = new Set([
      'read_file', 'read_directory', 'glob', 'grep', 'shell_output', 'shell_tasks',
      'todo_write', 'task_create', 'task_update', 'task_list', 'task_get', 'sleep',
      'web_search', 'web_fetch', 'agent', 'agent_output', 'activate_skill', 'taste',
      'get_diagnostics', 'ask_user_question', 'plan_review', 'enter_plan_mode', 'exit_plan_mode',
      'cron_list', 'run_command',
    ])

    const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

    export default function (cmd: any) {
      const dir = process.env.DROPPY_CODE_APPROVALS
      if (!dir) return
      cmd.hooks({
        beforeToolCall: async ({toolCallId, toolName, input}: any, ctx: any) => {
          if (READ_ONLY.has(toolName)) return
          const id = String(toolCallId || `${Date.now()}-${Math.random().toString(36).slice(2)}`).replace(/[^A-Za-z0-9_.-]/g, '_')
          const request = path.join(dir, `${id}.request.json`)
          const reply = path.join(dir, `${id}.reply.json`)
          try { fs.unlinkSync(reply) } catch {}
          fs.writeFileSync(`${request}.tmp`, JSON.stringify({toolCallId: id, toolName, input}))
          fs.renameSync(`${request}.tmp`, request)
          const started = Date.now()
          for (;;) {
            let raw: string | undefined
            try { raw = fs.readFileSync(reply, 'utf8') } catch {}
            if (raw !== undefined) {
              try { fs.unlinkSync(reply) } catch {}
              let answer: any = {}
              try { answer = JSON.parse(raw) } catch {}
              if (answer.decision === 'allow') return
              return {block: true, additionalContext: answer.reason || 'The user declined this in Droppy Code.'}
            }
            if (ctx && ctx.signal && ctx.signal.aborted) {
              try { fs.unlinkSync(request) } catch {}
              return {block: true, additionalContext: 'The turn was stopped.'}
            }
            if (process.ppid === 1 || Date.now() - started > 6 * 60 * 60 * 1000) {
              try { fs.unlinkSync(request) } catch {}
              return {block: true, additionalContext: 'Droppy Code did not answer.'}
            }
            await sleep(40)
          }
        },
      })
    }
    """#

    /// Where the mod and each run's approval directory live.
    static var supportDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: LoginEnvironment.homeDirectory).appendingPathComponent("Library/Application Support")
        return base.appendingPathComponent("\(AppInfo.name)/CommandCode", isDirectory: true)
    }

    /// Writes the mod where the CLI can load it and returns its path. Rewritten only when
    /// its text changed, so a new build's mod replaces the old one.
    static func installMod() throws -> URL {
        let directory = supportDirectory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let file = directory.appendingPathComponent("droppy-code-approvals.ts")
        let data = Data(approvalMod.utf8)
        if (try? Data(contentsOf: file)) != data {
            try data.write(to: file, options: .atomic)
        }
        return file
    }
}
