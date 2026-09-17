import Foundation

struct ProviderStatus: Equatable, Sendable {
    enum Auth: Equatable, Sendable {
        case unknown
        case signedIn(String?)
        case signedOut
    }

    var executable: URL?
    var version: String?
    var auth: Auth = .unknown
    var isChecking = false
    /// Native API providers (DeepSeek, Meta, Z.ai) have no CLI; they are installed once an API key exists.
    var apiKeyConfigured = false

    var isInstalled: Bool { executable != nil || apiKeyConfigured }

    var summary: String {
        guard isInstalled else { return "Not installed" }
        switch auth {
        case .signedIn(let account): return account.map { "Signed in as \($0)" } ?? "Signed in"
        case .signedOut: return "Not signed in"
        case .unknown: return version.map { "Installed · \($0)" } ?? "Installed"
        }
    }
}

/// Finds provider CLIs, checks their sign-in state and caches their model catalogs.
@MainActor
@Observable
final class ProviderRegistry {
    private(set) var statuses: [ProviderKind: ProviderStatus] = [:]
    private(set) var catalogs: [ProviderKind: [ModelOption]] = [:]
    private(set) var commands: [String: [SlashCommand]] = [:]
    private(set) var commandErrors: [String: String] = [:]
    private(set) var recentCommands: [String: [String]] = [:]
    @ObservationIgnored private var commandLoads: [String: Task<Void, Never>] = [:]
    @ObservationIgnored private var commandsFetchedAt: [String: Date] = [:]
    /// How long a provider's command list is served without asking again. Listing means
    /// launching the CLI once per (provider, directory), and the list only changes when
    /// the user edits their commands or skills, so a thread switch every minute should
    /// not cost a process launch every minute.
    private static let commandFreshness: TimeInterval = 600
    private(set) var loadingCatalogs: Set<ProviderKind> = []

    @ObservationIgnored private let settings: AppSettings
    @ObservationIgnored private let cacheKey = "providerModelCatalogs"
    private(set) var planLimits: [ProviderKind: PlanLimits] = [:]
    private(set) var loadingLimits: Set<ProviderKind> = []
    /// The banked reset being spent right now: its credit id, or "codex" for one the app-server listed only by count.
    private(set) var redeemingReset: String?
    @ObservationIgnored private var limitsFetchedAt: [ProviderKind: Date] = [:]

    /// Pay-as-you-go balances (DeepSeek's credit), shown where subscriptions show their windows.
    private(set) var credits: [ProviderKind: ProviderCredits] = [:]
    private(set) var loadingCredits: Set<ProviderKind> = []
    @ObservationIgnored private var creditsFetchedAt: [ProviderKind: Date] = [:]

    private static let claudeEfforts = ["low", "medium", "high", "xhigh", "max"]
    private static let claudeSeed = [
        ModelOption(id: "default", name: "Default", detail: "Claude Code's recommended model", efforts: claudeEfforts, isDefault: true, fastTier: "fast"),
        ModelOption(id: "opus", name: "Opus", efforts: claudeEfforts, fastTier: "fast"),
        ModelOption(id: "claude-fable-5-1[1m]", name: "Fable", detail: "Fable 5.1 · Most capable for your hardest and longest-running tasks", efforts: claudeEfforts),
        ModelOption(id: "sonnet", name: "Sonnet", efforts: claudeEfforts),
        ModelOption(id: "haiku", name: "Haiku"),
    ]

    /// A cached Claude list made only of the earlier seed's ids is that seed, persisted when another
    /// provider's live catalog arrived, not a list the CLI reported: a live list always names `opus[1m]`
    /// and the Fable row. Re-seeding it gives launches that cached the old seed the Fable row, while a
    /// live list without Fable stays as it is, since that account cannot use it.
    /// The seed with a live Claude list laid over it: a live row for a seed id lends its
    /// effort levels, description and fast tier; a row the seed lacks is appended; no seed
    /// row is ever dropped, since threads keep naming those ids.
    private static func mergedClaudeCatalog(_ live: [ModelOption]) -> [ModelOption] {
        var merged = claudeSeed
        for option in live {
            if let index = merged.firstIndex(where: { $0.id == option.id }) {
                var kept = merged[index]
                if !option.efforts.isEmpty { kept.efforts = option.efforts }
                if let detail = option.detail, !detail.isEmpty { kept.detail = detail }
                if option.fastTier != nil { kept.fastTier = option.fastTier }
                if option.defaultEffort != nil { kept.defaultEffort = option.defaultEffort }
                merged[index] = kept
            } else {
                merged.append(option)
            }
        }
        return merged
    }

    private static func isStaleClaudeSeed(_ list: [ModelOption]) -> Bool {
        let earlierSeedIDs: Set<String> = ["default", "opus", "sonnet", "haiku"]
        return list.isEmpty || list.allSatisfy { earlierSeedIDs.contains($0.id) }
    }

    // Seeds come from the same tables the live fetch uses, so the two can never disagree.
    static let deepseekSeed = ["deepseek-v4-pro", "deepseek-flash"].compactMap(DeepSeekAPI.option(for:))
    static let metaSeed = ["muse-spark-1.3", "muse-spark-1.3-contributor", "muse-spark-1.2", "muse-spark-1.2-contributor", "muse-spark-1.1"]
        .compactMap(MetaAPI.option(for:))
    static let zaiSeed = ["glm-5.3", "glm-5.3-flash"].compactMap(ZaiAPI.option(for:))

    /// From `agy models`: base models with their supported effort levels.
    /// Effort is adjusted via the effort slider, not separate duplicate rows.
    static let antigravitySeed: [ModelOption] = [
        ModelOption(id: "gemini-3.8-flash", name: "Gemini 3.8 Flash", efforts: ["low", "medium", "high"], defaultEffort: "high", isDefault: true),
        ModelOption(id: "gemini-3.7-flash", name: "Gemini 3.7 Flash", efforts: ["low", "medium", "high"], defaultEffort: "high"),
        ModelOption(id: "gemini-3.6-flash", name: "Gemini 3.6 Flash", efforts: ["low", "medium", "high"], defaultEffort: "high"),
        ModelOption(id: "gemini-3.1-pro", name: "Gemini 3.1 Pro", efforts: ["low", "high"], defaultEffort: "high"),
        ModelOption(id: "claude-sonnet-4-6", name: "Claude Sonnet 4.6 (Thinking)", efforts: []),
        ModelOption(id: "claude-opus-4-6-thinking", name: "Claude Opus 4.6 (Thinking)", efforts: []),
        ModelOption(id: "gpt-oss-120b", name: "GPT-OSS 120B", efforts: ["medium"], defaultEffort: "medium"),
    ]

    /// Copilot's own choice of model, which the CLI accepts as `auto`. The live catalog lists
    /// the account's models behind it; a fresh install starts with just this row.
    static let copilotAuto = ModelOption(id: "auto", name: "Auto", detail: "Copilot picks the model for each request", isDefault: true)

    /// Command Code's starting catalog; `cmd --list-models` replaces it with the account's
    /// seventy-odd models, led by the one the CLI itself is set to.
    static let commandcodeSeed = CommandCodeAPI.seed

    /// Pi's starting catalog; the live catalog from pi's RPC `get_available_models` replaces it.
    static let piSeed = [ModelOption(id: "default", name: "Default", detail: "The model pi itself is set to", efforts: ["off", "minimal", "low", "medium", "high", "xhigh", "max"], defaultEffort: "medium", isDefault: true)]

    /// Every provider tries its live catalog at most once per launch unless forced, success or not,
    /// so views that ask on appear never re-spawn a CLI or re-hit an API while scrolling.
    @ObservationIgnored private var attemptedCatalogs: Set<ProviderKind> = []

    init(settings: AppSettings) {
        self.settings = settings
        recentCommands = (WebsiteCaptures.defaults ?? .standard).dictionary(forKey: "recentSlashCommands") as? [String: [String]] ?? [:]
        if !WebsiteCaptures.isEnabled,
           let data = UserDefaults.standard.data(forKey: cacheKey),
           let cached = try? JSONDecoder().decode([String: [ModelOption]].self, from: data) {
            for (key, list) in cached {
                guard let provider = ProviderKind(rawValue: key) else { continue }
                // A list cached before the scales were ordered may still run high to low
                // (Grok's did); the slider reads them low to high, so they are put right here.
                catalogs[provider] = list.map { option in
                    var option = option
                    option.efforts = ACPSession.orderedEfforts(option.efforts)
                    return option
                }
            }
        }
        if Self.isStaleClaudeSeed(catalogs[.claude] ?? []) { catalogs[.claude] = Self.claudeSeed }
        if catalogs[.deepseek]?.isEmpty ?? true { catalogs[.deepseek] = Self.deepseekSeed }
        if catalogs[.meta]?.isEmpty ?? true { catalogs[.meta] = Self.metaSeed }
        if catalogs[.zai]?.isEmpty ?? true { catalogs[.zai] = Self.zaiSeed }
        if catalogs[.antigravity]?.isEmpty ?? true || catalogs[.antigravity]?.contains(where: { $0.id.hasSuffix("-high") || $0.id.hasSuffix("-medium") || $0.id.hasSuffix("-low") }) == true {
            catalogs[.antigravity] = Self.antigravitySeed
        }
        if catalogs[.copilot]?.isEmpty ?? true { catalogs[.copilot] = [Self.copilotAuto] }
        if catalogs[.commandcode]?.isEmpty ?? true { catalogs[.commandcode] = Self.commandcodeSeed }
        if catalogs[.pi]?.isEmpty ?? true { catalogs[.pi] = Self.piSeed }
    }

    var availableProviders: [ProviderKind] {
        ProviderKind.allCases.filter { settings.isEnabled($0) && (statuses[$0]?.isInstalled ?? true) }
    }

    func status(_ provider: ProviderKind) -> ProviderStatus {
        // A capture run never probes the CLIs (see `refresh`), and reads as a Mac with every
        // provider installed and signed in, so the pairs and pickers show what they offer.
        if WebsiteCaptures.isEnabled {
            return ProviderStatus(executable: URL(fileURLWithPath: "/usr/bin/true"), auth: .signedIn(nil), apiKeyConfigured: true)
        }
        return statuses[provider] ?? ProviderStatus()
    }

    func executable(for provider: ProviderKind) -> URL? {
        guard !provider.isAPIKeyBased else { return nil }
        let custom = settings.binaryPath(for: provider)
        guard !custom.isEmpty || !provider.executableName.isEmpty else { return nil }
        if let found = LoginEnvironment.which(custom.isEmpty ? provider.executableName : custom) { return found }
        guard custom.isEmpty else { return nil }
        return Self.fallbackExecutables(for: provider).first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    /// Where a provider lands when its installer keeps it off the PATH: `gh copilot`
    /// downloads the Copilot CLI into gh's data directory.
    private static func fallbackExecutables(for provider: ProviderKind) -> [URL] {
        switch provider {
        case .copilot:
            let environment = LoginEnvironment.current
            let dataDirectory = environment["GH_DATA_DIR"].flatMap { $0.isEmpty ? nil : $0 }
                ?? environment["XDG_DATA_HOME"].flatMap { $0.isEmpty ? nil : ($0 as NSString).appendingPathComponent("gh") }
                ?? (LoginEnvironment.homeDirectory as NSString).appendingPathComponent(".local/share/gh")
            return [URL(fileURLWithPath: dataDirectory).appendingPathComponent("copilot/copilot")]
        case .commandcode:
            // npm's global bin under a user prefix, plus the CLI's other names: `cmd` is
            // the documented alias, `command-code` and `commandcode` the package's own.
            let home = LoginEnvironment.homeDirectory
            var candidates: [URL] = []
            for name in ["command-code", "commandcode", "cmd"] {
                if let found = LoginEnvironment.which(name) { candidates.append(found) }
            }
            for directory in ["\(home)/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", "\(home)/.npm-global/bin", "\(home)/.volta/bin"] {
                for name in ["command-code", "cmd"] {
                    candidates.append(URL(fileURLWithPath: directory).appendingPathComponent(name))
                }
            }
            return candidates
        case .pi:
            let home = LoginEnvironment.homeDirectory
            return ["\(home)/.npm-global/bin/pi", "/opt/homebrew/bin/pi", "/usr/local/bin/pi", "\(home)/.local/bin/pi", "\(home)/.volta/bin/pi", "\(home)/.bun/bin/pi"].map { URL(fileURLWithPath: $0) }
        default:
            return []
        }
    }

    func environment(for provider: ProviderKind) -> [String: String] {
        var environment = LoginEnvironment.current
        if let directory = executable(for: provider)?.deletingLastPathComponent().path,
           let path = environment["PATH"],
           !path.split(separator: ":").contains(Substring(directory)) {
            environment["PATH"] = directory + ":" + path
        }
        // A Studio key pasted into Settings reaches the CLI the way its docs say, and takes
        // precedence over the `cmd login` in `~/.commandcode/auth.json`.
        if provider == .commandcode {
            let key = settings.apiKey(for: .commandcode)
            if !key.isEmpty { environment["COMMAND_CODE_API_KEY"] = key }
        }
        return environment
    }

    private(set) var isRefreshing = false

    /// The Providers page's refresh button: checks every provider again, re-reads the usage limits
    /// the page shows for each signed-in account and reloads the live catalogs. Beyond the limits,
    /// nothing on that page checks on its own, so this is the only way it hits the CLIs and APIs.
    func refreshEverything() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        await refreshAll()
        // Credits were already re-read when the key proved itself in `refreshAPIProvider`.
        for provider in ProviderKind.allCases where status(provider).auth != .signedOut {
            refreshPlanLimits(provider, force: true)
        }
        await withTaskGroup(of: Void.self) { group in
            for provider in [ProviderKind.claude, .codex, .antigravity, .copilot, .commandcode, .pi, .deepseek, .meta, .zai] where status(provider).isInstalled {
                group.addTask { await self.loadCatalog(provider, force: true) }
            }
        }
    }

    /// How many providers are checked at the same time. Every check spawns the provider's
    /// CLI, and launch checks them all: starting a dozen node processes at once is a
    /// visible stall in everything else the app is doing just then.
    private static let probeWidth = 4

    func refreshAll() async {
        let providers = ProviderKind.allCases
        var next = 0
        await withTaskGroup(of: Void.self) { group in
            while next < providers.count, next < Self.probeWidth {
                let provider = providers[next]
                group.addTask { await self.refresh(provider) }
                next += 1
            }
            for await _ in group {
                guard next < providers.count else { continue }
                let provider = providers[next]
                next += 1
                group.addTask { await self.refresh(provider) }
            }
        }
    }

    func refresh(_ provider: ProviderKind) async {
        var status = statuses[provider] ?? ProviderStatus()
        status.isChecking = true
        statuses[provider] = status
        if provider.isAPIKeyBased {
            await refreshAPIProvider(provider)
            return
        }
        await LoginEnvironment.load()
        guard let executable = executable(for: provider) else {
            statuses[provider] = ProviderStatus()
            return
        }
        let environment = environment(for: provider)
        // Both probes spawn the CLI. One after the other rather than together: the pair
        // was two processes per provider at the same moment, on top of every other
        // provider being checked alongside.
        let version = await Self.version(executable, environment)
        let auth = await Self.auth(provider, executable, environment)
        statuses[provider] = ProviderStatus(executable: executable, version: version, auth: auth)
    }

    /// API-key providers have no binary: presence (and validity) of the key is the install state.
    private func refreshAPIProvider(_ provider: ProviderKind) async {
        let apiKey = settings.apiKey(for: provider)
        guard !apiKey.isEmpty else {
            statuses[provider] = ProviderStatus(auth: .signedOut)
            return
        }
        if provider == .deepseek {
            let valid = await DeepSeekAPI.validate(apiKey: apiKey)
            statuses[provider] = ProviderStatus(
                version: "API",
                auth: valid ? .signedIn(nil) : .signedOut,
                apiKeyConfigured: true
            )
            // The key just proved itself, so read the balance behind it right away: the
            // usage panel has it from launch, and a rejected key must not leave a stale one.
            if valid {
                refreshCredits(.deepseek, force: true)
            } else {
                credits[.deepseek] = nil
                creditsFetchedAt[.deepseek] = nil
            }
        } else if provider == .meta {
            let valid = await MetaAPI.validate(apiKey: apiKey)
            statuses[provider] = ProviderStatus(
                version: "API",
                auth: valid ? .signedIn(nil) : .signedOut,
                apiKeyConfigured: true
            )
        } else if provider == .zai {
            let valid = await ZaiAPI.validate(apiKey: apiKey)
            statuses[provider] = ProviderStatus(
                version: "API",
                auth: valid ? .signedIn(nil) : .signedOut,
                apiKeyConfigured: true
            )
            if valid {
                refreshPlanLimits(.zai, force: true)
            } else {
                planLimits[.zai] = nil
                limitsFetchedAt[.zai] = nil
            }
        } else {
            statuses[provider] = ProviderStatus(auth: .signedIn(nil), apiKeyConfigured: true)
        }
    }

    // MARK: - Models

    func models(for provider: ProviderKind) -> [ModelOption] {
        catalogs[provider] ?? []
    }

    func model(_ id: String?, for provider: ProviderKind) -> ModelOption? {
        guard let id else { return nil }
        let list = models(for: provider)
        if let exact = list.first(where: { $0.id == id }) { return exact }
        // Cursor used to advertise parameterized ids (`model[effort=…]`); the catalog now
        // stores the base name once the parameterized picker is declared.
        guard id.contains("["), let bracket = id.firstIndex(of: "[") else { return nil }
        let base = String(id[..<bracket])
        return list.first { $0.id == base }
    }

    func defaultModel(for provider: ProviderKind) -> ModelOption? {
        let list = models(for: provider)
        if let last = settings.lastModel(for: provider), let match = model(last, for: provider) {
            return match
        }
        return list.first(where: \.isDefault) ?? list.first
    }

    func loadCatalog(_ provider: ProviderKind, force: Bool = false) async {
        guard !loadingCatalogs.contains(provider) else { return }
        guard force || !attemptedCatalogs.contains(provider) else { return }
        if provider.isAPIKeyBased {
            await loadAPICatalog(provider, force: force)
            return
        }
        // Copilot's seed is only its Auto row, and Command Code's a handful of its seventy
        // models, so the account's list is fetched on first use. Claude's seed is the four
        // names its CLI has always taken, so its handshake runs every launch: the CLI's own
        // list follows its version.
        let seeded = provider == .claude
            || (provider == .copilot && models(for: provider) == [Self.copilotAuto])
            || (provider == .commandcode && models(for: provider) == Self.commandcodeSeed)
            || (provider == .pi && models(for: provider) == Self.piSeed)
        guard force || seeded || models(for: provider).isEmpty else { return }
        loadingCatalogs.insert(provider)
        defer { loadingCatalogs.remove(provider) }
        await LoginEnvironment.load()
        guard let executable = executable(for: provider) else { return }
        attemptedCatalogs.insert(provider)
        let environment = environment(for: provider)
        let list: [ModelOption]? = switch provider {
        case .claude: await loadClaudeCatalog(executable: executable, environment: environment)
        case .codex: try? await CodexSession.listModels(executable: executable, environment: environment)
        case .antigravity: try? await AntigravitySession.listModels(executable: executable, environment: environment)
        case .copilot: try? await CopilotSession.listModels(executable: executable, environment: environment)
        case .commandcode: try? await CommandCodeAPI.listModels(executable: executable, environment: environment)
        case .pi: try? await PiCLI.listModels(executable: executable, environment: environment)
        case .cursor, .opencode, .grok, .devin: try? await ACPSession.probeModels(provider: provider, executable: executable, environment: environment)
        case .deepseek, .meta, .zai: nil
        }
        if let list, !list.isEmpty { updateCatalog(list, for: provider) }
    }

    private func loadClaudeCatalog(executable: URL, environment: [String: String]) async -> [ModelOption]? {
        // The handshake runs somewhere: the models are the same from any folder.
        let directory = FileManager.default.temporaryDirectory
        return try? await ClaudeSession.handshake(executable: executable, directory: directory, environment: environment).models
    }

    private func loadAPICatalog(_ provider: ProviderKind, force: Bool) async {
        // The seed keeps API providers usable offline; a live fetch only refines it.
        let seed: [ModelOption]? = switch provider {
        case .deepseek: Self.deepseekSeed
        case .meta: Self.metaSeed
        case .zai: Self.zaiSeed
        default: nil
        }
        guard let seed else { return }
        if models(for: provider).isEmpty { updateCatalog(seed, for: provider) }
        loadingCatalogs.insert(provider)
        defer { loadingCatalogs.remove(provider) }
        let apiKey = settings.apiKey(for: provider)
        guard !apiKey.isEmpty else { return }
        attemptedCatalogs.insert(provider)
        let list: [ModelOption]? = switch provider {
        case .deepseek: try? await DeepSeekAPI.listModels(apiKey: apiKey)
        case .meta: try? await MetaAPI.listModels(apiKey: apiKey)
        case .zai: try? await ZaiAPI.listModels(apiKey: apiKey)
        default: nil
        }
        if let list, !list.isEmpty { updateCatalog(list, for: provider) }
    }

    /// Reads the provider's plan limits, at most once a minute after a successful read unless forced.
    /// The read runs on its own task, so closing the popover that asked for it cannot cancel it.
    func refreshPlanLimits(_ provider: ProviderKind, force: Bool = false) {
        guard PlanLimitsReader.exposesLimits(provider), !loadingLimits.contains(provider) else { return }
        if !force, let fetched = limitsFetchedAt[provider], Date.now.timeIntervalSince(fetched) < 60 { return }
        if provider.isAPIKeyBased {
            let apiKey = settings.apiKey(for: provider)
            guard !apiKey.isEmpty else {
                planLimits[provider] = nil
                return
            }
            loadingLimits.insert(provider)
            Task {
                let limits = await PlanLimitsReader.read(provider, apiKey: apiKey)
                loadingLimits.remove(provider)
                guard let limits else { return }
                planLimits[provider] = limits
                limitsFetchedAt[provider] = .now
            }
            return
        }
        guard PlanLimitsReader.exposesLimits(provider), !loadingLimits.contains(provider) else { return }
        loadingLimits.insert(provider)
        Task {
            defer { loadingLimits.remove(provider) }
            // A popover opened right after launch waits for the login shell like everything
            // else; before that read the CLI can only be found on the fallback PATH.
            await LoginEnvironment.load()
            guard let executable = executable(for: provider) else { return }
            let limits = await PlanLimitsReader.read(provider, executable: executable, environment: environment(for: provider))
            guard let limits else { return }
            planLimits[provider] = limits
            limitsFetchedAt[provider] = .now
        }
    }

    /// Spends one of a provider's banked resets and re-reads its limits so the bars show the cleared window. Codex through its app-server, Z.ai through its reset-card endpoint.
    func consumeResetCredit(_ provider: ProviderKind, _ creditID: String?) async throws -> PlanLimits.ResetOutcome {
        redeemingReset = creditID ?? provider.rawValue
        defer { redeemingReset = nil }
        switch provider {
        case .codex:
            await LoginEnvironment.load()
            guard let executable = executable(for: .codex) else { throw ResetCreditError.codexUnavailable }
            let environment = environment(for: .codex)
            let outcome = try await CodexSession.consumeResetCredit(executable: executable, environment: environment, creditID: creditID)
            if let limits = try? await CodexSession.readPlanLimits(executable: executable, environment: environment) {
                planLimits[.codex] = limits
                limitsFetchedAt[.codex] = .now
            }
            return outcome
        case .zai:
            let apiKey = settings.apiKey(for: .zai)
            guard !apiKey.isEmpty, let creditID else { throw ResetCreditError.unknownOutcome }
            let outcome = try await ZaiAPI.consumeResetCredit(apiKey: apiKey, creditID: creditID)
            if let limits = await ZaiAPI.planLimits(apiKey: apiKey) {
                planLimits[.zai] = limits
                limitsFetchedAt[.zai] = .now
            }
            return outcome
        default:
            throw ResetCreditError.unknownOutcome
        }
    }

    /// Reads a pay-as-you-go provider's balance, at most once a minute after a successful
    /// read unless forced. Like the plan limits, the read runs on its own task, so closing
    /// the popover that asked for it cannot cancel it.
    func refreshCredits(_ provider: ProviderKind, force: Bool = false) {
        guard CreditsReader.exposesCredits(provider), !loadingCredits.contains(provider) else { return }
        // Command Code's key is the CLI's own login unless Settings holds one.
        let apiKey = provider == .commandcode
            ? (CommandCodeAPI.apiKey(environment: environment(for: .commandcode)) ?? "")
            : settings.apiKey(for: provider)
        guard !apiKey.isEmpty else {
            credits[provider] = nil
            return
        }
        if !force, let fetched = creditsFetchedAt[provider], Date.now.timeIntervalSince(fetched) < 60 { return }
        loadingCredits.insert(provider)
        Task {
            let balance = await CreditsReader.read(provider, apiKey: apiKey)
            loadingCredits.remove(provider)
            guard let balance else { return }
            credits[provider] = balance
            creditsFetchedAt[provider] = .now
        }
    }

    /// A turn just spent tokens, so the cached balance is older than the spend. The next
    /// look reads the API again instead of handing back the number from before the turn.
    func invalidateCredits(_ provider: ProviderKind) {
        guard CreditsReader.exposesCredits(provider) else { return }
        creditsFetchedAt[provider] = nil
    }

    /// A turn just spent credits, so the cached plan windows are older than the spend.
    /// The next look reads the API again instead of handing back the numbers from
    /// before the turn.
    func invalidatePlanLimits(_ provider: ProviderKind) {
        guard provider.isAPIKeyBased, PlanLimitsReader.exposesLimits(provider) else { return }
        limitsFetchedAt[provider] = nil
    }

    func updateCatalog(_ list: [ModelOption], for provider: ProviderKind) {
        // Claude's live list refines the seed rather than replacing it: threads, pairs and
        // the chip name the seed's ids (`opus`, the Fable row), and a handshake that lists
        // the models another way left them as raw ids with one reasoning level.
        let list = provider == .claude ? Self.mergedClaudeCatalog(list) : list
        guard !list.isEmpty, list != catalogs[provider] else { return }
        catalogs[provider] = list
        guard !WebsiteCaptures.isEnabled else { return }
        let encoded = Dictionary(uniqueKeysWithValues: catalogs.map { ($0.key.rawValue, $0.value) })
        if let data = try? JSONEncoder().encode(encoded) {
            UserDefaults.standard.set(data, forKey: cacheKey)
        }
    }

    static func commandKey(_ provider: ProviderKind, directory: String) -> String {
        provider.rawValue + "\n" + directory
    }

    func updateCommands(_ list: [SlashCommand], for provider: ProviderKind, directory: String) {
        let key = Self.commandKey(provider, directory: directory)
        if commands[key] == nil, commands.count >= 32,
           let oldest = commandsFetchedAt.min(by: { $0.value < $1.value })?.key {
            commands[oldest] = nil
            commandsFetchedAt[oldest] = nil
            commandErrors[oldest] = nil
        }
        commands[key] = list
        commandsFetchedAt[key] = .now
        commandErrors[key] = nil
    }

    func recordCommand(_ name: String, for provider: ProviderKind) {
        var recent = recentCommands[provider.rawValue] ?? []
        recent.removeAll { $0 == name }
        recent.insert(name, at: 0)
        recentCommands[provider.rawValue] = Array(recent.prefix(100))
        (WebsiteCaptures.defaults ?? .standard).set(recentCommands, forKey: "recentSlashCommands")
    }

    /// `force` asks again whatever the list's age: the retry after a failed load.
    func loadCommands(_ provider: ProviderKind, directory: String, force: Bool = false) async {
        guard !WebsiteCaptures.isEnabled, provider == .codex || provider == .claude else { return }
        let key = Self.commandKey(provider, directory: directory)
        if let task = commandLoads[key] { await task.value; return }
        if !force, let fetched = commandsFetchedAt[key], Date.now.timeIntervalSince(fetched) < Self.commandFreshness { return }
        guard let executable = executable(for: provider) else { return }
        let environment = environment(for: provider)
        let task = Task {
            do {
                let directoryURL = URL(fileURLWithPath: directory)
                let list = try await provider == .codex
                    ? CodexSession.listCommands(executable: executable, directory: directoryURL, environment: environment)
                    : ClaudeSession.handshake(executable: executable, directory: directoryURL, environment: environment).commands
                updateCommands(list, for: provider, directory: directory)
            } catch {
                if commandErrors.count >= 32 { commandErrors.removeAll(keepingCapacity: true) }
                commandErrors[key] = error.localizedDescription
            }
        }
        commandLoads[key] = task
        await task.value
        commandLoads[key] = nil
    }

    // MARK: - Probes

    private static func version(_ executable: URL, _ environment: [String: String]) async -> String? {
        guard let result = try? await Shell.run(executable, ["--version"], environment: environment, timeout: 20),
              result.succeeded else { return nil }
        let text = TextCleanup.stripANSI(result.output + result.errorOutput)
        if let match = text.firstMatch(of: #/\d+\.\d+(?:\.\d+)?(?:[-.][0-9A-Za-z]+)?/#) {
            return String(match.output)
        }
        return nil
    }

    private static func auth(_ provider: ProviderKind, _ executable: URL, _ environment: [String: String]) async -> ProviderStatus.Auth {
        switch provider {
        case .codex:
            guard let result = try? await Shell.run(executable, ["login", "status"], environment: environment, timeout: 20) else {
                return .unknown
            }
            let text = result.output + result.errorOutput
            return result.succeeded && text.localizedCaseInsensitiveContains("logged in") ? .signedIn(nil) : .signedOut
        case .claude:
            guard let result = try? await Shell.run(executable, ["auth", "status"], environment: environment, timeout: 20),
                  let json = JSONValue.parse(result.stdout) else { return .unknown }
            return json["loggedIn"]?.bool == true ? .signedIn(json["email"]?.string) : .signedOut
        case .cursor:
            guard let result = try? await Shell.run(executable, ["status"], environment: environment, timeout: 20) else {
                return .unknown
            }
            let text = TextCleanup.stripANSI(result.output + result.errorOutput)
            if let match = text.firstMatch(of: #/Logged in as (\S+)/#) { return .signedIn(String(match.output.1)) }
            return text.localizedCaseInsensitiveContains("not logged in") ? .signedOut : .unknown
        case .devin:
            guard let result = try? await Shell.run(executable, ["auth", "status"], environment: environment, timeout: 20) else {
                return .unknown
            }
            let text = TextCleanup.stripANSI(result.output + result.errorOutput)
            if text.localizedCaseInsensitiveContains("not logged in") { return .signedOut }
            guard text.localizedCaseInsensitiveContains("logged in") else { return .unknown }
            return .signedIn(Self.devinAccount(from: text))
        case .antigravity:
            return await AntigravitySession.authStatus(executable: executable, environment: environment)
        case .copilot:
            return await CopilotSession.authStatus(executable: executable, environment: environment)
        case .commandcode:
            return await CommandCodeAPI.authStatus(executable: executable, environment: environment)
        case .pi:
            return await PiCLI.authStatus(executable: executable, environment: environment)
        case .opencode, .grok, .deepseek, .meta, .zai:
            return .unknown
        }
    }

    /// The `Name:` line of `devin auth status`, so Settings reads "Signed in as …".
    private static func devinAccount(from text: String) -> String? {
        for line in text.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard trimmed.lowercased().hasPrefix("name:") else { continue }
            let name = trimmed.dropFirst("name:".count).trimmingCharacters(in: .whitespaces)
            return name.isEmpty ? nil : name
        }
        return nil
    }
}
