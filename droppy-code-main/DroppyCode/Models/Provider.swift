import Foundation

enum ProviderKind: String, Codable, CaseIterable, Identifiable, Sendable {
    case codex
    case claude
    case cursor
    case opencode
    case grok
    case deepseek
    case meta
    case zai
    case devin
    case antigravity
    case copilot
    case commandcode
    case pi

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .codex: "Codex"
        case .claude: "Claude"
        case .cursor: "Cursor"
        case .opencode: "OpenCode"
        case .grok: "Grok"
        case .deepseek: "DeepSeek"
        case .meta: "Meta"
        case .zai: "Z.ai"
        case .devin: "Devin"
        case .antigravity: "Antigravity"
        case .copilot: "Copilot"
        case .commandcode: "Command Code"
        case .pi: "Pi"
        }
    }

    var executableName: String {
        switch self {
        case .codex: "codex"
        case .claude: "claude"
        case .cursor: "cursor-agent"
        case .opencode: "opencode"
        case .grok: "grok"
        case .deepseek: ""
        case .meta: ""
        case .zai: ""
        case .devin: "devin"
        case .antigravity: "agy"
        case .copilot: "copilot"
        case .commandcode: "cmd"
        case .pi: "pi"
        }
    }

    var iconName: String { "provider-\(rawValue)" }

    var loginCommand: String {
        switch self {
        case .codex: "codex login"
        case .claude: "claude auth login"
        case .cursor: "cursor-agent login"
        case .opencode: "opencode auth login"
        case .grok: "grok login"
        case .deepseek: "DEEPSEEK_API_KEY=sk-..."
        case .meta: "MODEL_API_KEY=..."
        case .zai: "ZAI_API_KEY=..."
        case .devin: "devin auth login"
        case .antigravity: "agy"
        case .copilot: "copilot login"
        case .commandcode: "cmd login"
        case .pi: "pi"
        }
    }

    var installURL: URL {
        switch self {
        case .codex: URL(string: "https://developers.openai.com/codex/cli")!
        case .claude: URL(string: "https://claude.com/product/claude-code")!
        case .cursor: URL(string: "https://cursor.com/cli")!
        case .opencode: URL(string: "https://opencode.ai")!
        case .grok: URL(string: "https://x.ai/cli")!
        case .deepseek: URL(string: "https://platform.deepseek.com/api_keys")!
        case .meta: URL(string: "https://dev.meta.ai/")!
        case .zai: URL(string: "https://z.ai/manage-apikey/apikey-list")!
        case .devin: URL(string: "https://cli.devin.ai")!
        case .antigravity: URL(string: "https://antigravity.google/docs/cli/overview")!
        case .copilot: URL(string: "https://github.com/github/copilot-cli")!
        case .commandcode: URL(string: "https://commandcode.ai/docs/quickstart")!
        case .pi: URL(string: "https://pi.dev/docs/latest")!
        }
    }

    /// API-key providers talk to their cloud API directly instead of a local CLI.
    var isAPIKeyBased: Bool { self == .deepseek || self == .meta || self == .zai }

    /// Providers with an API key field in Settings: the API-key providers, which need one,
    /// and Command Code, whose CLI takes a Studio key through `COMMAND_CODE_API_KEY` as an
    /// alternative to `cmd login`.
    var acceptsAPIKey: Bool { isAPIKeyBased || self == .commandcode }

    /// Host shown in Settings for API-key providers, e.g. "api.meta.ai/v1".
    var apiHost: String? {
        switch self {
        case .deepseek: "api.deepseek.com"
        case .meta: "api.meta.ai/v1"
        case .zai: "api.z.ai/api/coding/paas/v4"
        default: nil
        }
    }

    /// Where the dashboard key lives, for the Settings detail line.
    var apiKeySource: String? {
        switch self {
        case .deepseek: "platform.deepseek.com"
        case .meta: "dev.meta.ai"
        case .zai: "z.ai/manage-apikey/apikey-list"
        case .commandcode: "commandcode.ai/studio"
        default: nil
        }
    }

    /// Env var fallback read when no key is stored in Settings.
    var apiKeyEnvVar: String? {
        switch self {
        case .deepseek: "DEEPSEEK_API_KEY"
        case .meta: "MODEL_API_KEY"
        case .zai: "ZAI_API_KEY"
        case .commandcode: "COMMAND_CODE_API_KEY"
        default: nil
        }
    }

    /// Whether the provider can drop later turns from its own conversation.
    var supportsRewind: Bool { self == .codex || self == .claude || self == .copilot }

    /// Headless stream-json only carries text: the TUI pastes images, but a
    /// streaming session rejects non-text blocks, so image attachments stay off.
    var supportsImages: Bool { self != .grok && self != .antigravity }

    var usesACP: Bool { self == .cursor || self == .opencode || self == .grok || self == .devin }
}

enum RuntimeMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case supervised
    case autoAcceptEdits
    case auto
    case fullAccess

    var id: String { rawValue }

    var title: String {
        switch self {
        case .supervised: "Supervised"
        case .autoAcceptEdits: "Auto-accept edits"
        case .auto: "Auto"
        case .fullAccess: "Full access"
        }
    }

    var summary: String {
        switch self {
        case .supervised: "Asks before commands and file changes"
        case .autoAcceptEdits: "Edits files freely, asks before other actions"
        case .auto: "The provider's reviewer approves routine actions"
        case .fullAccess: "Runs commands and edits without asking"
        }
    }

    var symbol: String {
        switch self {
        case .supervised: "hand.raised"
        case .autoAcceptEdits: "pencil.line"
        case .auto: "wand.and.sparkles"
        case .fullAccess: "lock.open"
        }
    }
}

enum InteractionMode: String, Codable, Sendable {
    case build
    case plan
}

enum WorkspaceMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case local
    case worktree

    var id: String { rawValue }

    var title: String {
        switch self {
        case .local: "Local"
        case .worktree: "New worktree"
        }
    }
}

struct ModelOption: Codable, Hashable, Identifiable, Sendable {
    var id: String
    var name: String
    var detail: String?
    var efforts: [String]
    var defaultEffort: String?
    var isDefault: Bool
    /// The service tier that makes this model fast, when its provider offers one.
    var fastTier: String?

    init(
        id: String,
        name: String,
        detail: String? = nil,
        efforts: [String] = [],
        defaultEffort: String? = nil,
        isDefault: Bool = false,
        fastTier: String? = nil
    ) {
        self.id = id
        self.name = name
        self.detail = detail
        self.efforts = efforts
        self.defaultEffort = defaultEffort
        self.isDefault = isDefault
        self.fastTier = fastTier
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        detail = try container.decodeIfPresent(String.self, forKey: .detail)
        efforts = try container.decodeIfPresent([String].self, forKey: .efforts) ?? []
        defaultEffort = try container.decodeIfPresent(String.self, forKey: .defaultEffort)
        isDefault = try container.decodeIfPresent(Bool.self, forKey: .isDefault) ?? false
        fastTier = try container.decodeIfPresent(String.self, forKey: .fastTier)
    }

    var supportsFast: Bool { fastTier != nil }

    /// The name as the picker shows it, such as "5.6 Terra" for "GPT-5.6-Terra".
    var shortName: String {
        var trimmed = name
        if trimmed.lowercased().hasPrefix("gpt-") { trimmed = String(trimmed.dropFirst(4)) }
        return trimmed.replacingOccurrences(of: "-", with: " ")
    }

    /// The name at its shortest, for the composer chip, where the provider's icon already
    /// says whose model it is: "Opus" for "Opus (1M context)", "3.8 Flash" for "Gemini 3.8
    /// Flash", "V4.1 Flash" as is. Parentheticals and dashed or dotted descriptions go, then
    /// the vendor's own name when it leads. The picker keeps `shortName` and `detail`.
    var chipName: String {
        var text = shortName
        for separator in [" (", " · ", " — ", " – ", " - ", ": "] {
            if let range = text.range(of: separator) { text = String(text[..<range.lowerBound]) }
        }
        let lowered = text.lowercased()
        for vendor in ["claude ", "gpt ", "gemini ", "deepseek ", "openai ", "anthropic ", "google ", "meta ", "muse ", "glm "] {
            if lowered.hasPrefix(vendor), text.count > vendor.count {
                text = String(text.dropFirst(vendor.count))
                break
            }
        }
        let cleaned = text.trimmingCharacters(in: .whitespaces)
        return cleaned.isEmpty ? shortName : cleaned
    }

    static func effortTitle(_ effort: String) -> String {
        switch effort {
        case "xhigh", "extra-high": "Extra High"
        case "": "Default"
        default: effort.prefix(1).uppercased() + effort.dropFirst()
        }
    }
}

struct SlashCommand: Hashable, Identifiable, Sendable {
    var name: String
    var detail: String
    var isBuiltIn: Bool = false
    var skillPath: String?

    var id: String { name }

    static func name(in text: String) -> String? {
        guard text.first == "/" || text.first == "$" else { return nil }
        return text.dropFirst().split(whereSeparator: \.isWhitespace).first.map(String.init)
    }

    static func suggestions(_ commands: [Self], matching query: String, recent: [String]) -> [Self] {
        let needle = query.lowercased()
        let ranks = Dictionary(recent.enumerated().map { ($0.element, $0.offset) }, uniquingKeysWith: min)
        var seen = Set<String>()
        // Lowercased once per command, not once per comparison of the sort.
        var matched: [(command: Self, name: String, prefixed: Bool, rank: Int)] = []
        matched.reserveCapacity(commands.count)
        for command in commands where seen.insert(command.name).inserted {
            let name = command.name.lowercased()
            guard needle.isEmpty || name.contains(needle) else { continue }
            matched.append((command, name, name.hasPrefix(needle), ranks[command.name] ?? Int.max))
        }
        matched.sort { lhs, rhs in
            if lhs.prefixed != rhs.prefixed { return lhs.prefixed }
            return lhs.rank == rhs.rank ? lhs.name < rhs.name : lhs.rank < rhs.rank
        }
        return matched.map(\.command)
    }

    func codexInput(for text: String) -> [JSONValue] {
        guard let skillPath, Self.name(in: text) == name else {
            return [["type": "text", "text": .string(text)]]
        }
        return [
            ["type": "text", "text": .string("$" + text.dropFirst())],
            ["type": "skill", "name": .string(name), "path": .string(skillPath)],
        ]
    }

    static func claudeCommands(_ entries: [JSONValue]) -> [Self] {
        entries.compactMap { entry in
            guard let name = entry["name"]?.string ?? entry.string else { return nil }
            return Self(name: name, detail: entry["description"]?.string ?? "Claude Code command")
        }
    }

    static func codexSkills(_ result: JSONValue) -> [Self] {
        (result["data"]?.array ?? []).flatMap { entry in
            (entry["skills"]?.array ?? []).compactMap { skill in
                guard skill["enabled"]?.bool != false,
                      let name = skill["name"]?.string, let path = skill["path"]?.string else { return nil }
                return Self(name: name, detail: skill["interface"]?["shortDescription"]?.string ?? skill["description"]?.string ?? "", skillPath: path)
            }
        }
    }
}
