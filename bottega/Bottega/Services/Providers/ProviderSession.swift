// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// What a thread asks a provider to do for one turn.
struct TurnInput: Sendable {
    var text: String
    var images: [Attachment]
    var model: String?
    var effort: String?
    /// A Codex service tier for this turn: the fast tier, "default", or nil when the model has none.
    var serviceTier: String? = nil
    /// Cursor ACP's Fast variant (`model_config` / `fast`); other providers ignore it.
    var fastMode = false
    var runtimeMode: RuntimeMode
    var interactionMode: InteractionMode
    /// A Crew hand past its budget answering for the last time: the turn is one reply
    /// with no tools, where the session can arrange that.
    var isFinalReport = false
    var command: SlashCommand?
}

/// Everything a provider session needs to launch.
struct SessionConfiguration: Sendable {
    var provider: ProviderKind
    var executable: URL?
    var workingDirectory: URL
    var environment: [String: String]
    var resumeID: String?
    var resumeAt: String?
    var model: String?
    var effort: String?
    var fastMode = false
    var runtimeMode: RuntimeMode
    var interactionMode: InteractionMode
    /// Native API providers (DeepSeek, Meta, Z.ai) authenticate with this instead of a CLI.
    var apiKey: String?
    /// Set while Crew is on for the thread: providers that run hands of their own define
    /// them at launch, on the pair's model and effort, and steer the lead towards them.
    var crew: CrewLaunch?
    /// The conversation so far, for a provider that keeps nothing between launches (the
    /// API providers hold their history in memory only): the exchanges go back in as
    /// context, so a session that starts over still knows what was said.
    var transcript: [TranscriptMessage] = []
    /// The thread is a Crew hand that Bottega runs: the session paces it, so a hand
    /// that keeps reading without ever changing anything is told to act.
    var isCrewHand = false
    /// What such a hand's turn may take (Settings › Crew › Limits): the tool and time
    /// budget a session that paces its hands from inside the turn goes by. Read only
    /// while `isCrewHand`.
    var crewLimits = CrewLimits.default
    var antiSlopEnabled = true
}

/// One exchange of a conversation, for replaying it into a session that starts over.
struct TranscriptMessage: Sendable {
    enum Role: Sendable {
        case user
        case assistant
    }

    var role: Role
    var text: String
}

/// A hand a provider started inside the lead's session.
struct AgentSpawn: Sendable {
    /// The provider's id for the hand, which its later events carry.
    var id: String
    /// Claude's task id, which is what stops the hand; nil where the id above does.
    var taskID: String?
    /// The tool call in the lead's transcript that spawned the hand.
    var toolUseID: String?
    var description: String
    /// The hand's brief, when the provider says.
    var prompt: String?
    var model: String?
    /// The hand profile the spawn was routed to, when the provider's report names one:
    /// Claude's and Copilot's agent definitions are one per profile, and Codex's spawn
    /// call carries the profile's model and effort, which the session matches back.
    var profile: String?
    /// Whether the spawning tool call returns before the hand is done. A foreground hand
    /// is finished by its tool result; a background one only by the provider saying so.
    var isBackground = true
}

/// Normalized events every provider adapter emits into a thread.
enum ProviderEvent: Sendable {
    case sessionReady(sessionID: String)
    case turnStarted(providerTurnID: String?)
    case messageDelta(id: String, text: String)
    case messageCompleted(id: String, text: String)
    case reasoningDelta(id: String, text: String)
    case reasoningCompleted(id: String, text: String)
    case toolStarted(id: String, call: ToolCall)
    case toolOutput(id: String, text: String)
    case toolUpdated(id: String, update: ToolUpdate)
    case planDelta(id: String, text: String)
    case planCompleted(id: String, markdown: String)
    case todos([TodoStep])
    case approval(ApprovalRequest)
    case question(QuestionRequest)
    case requestResolved(id: String)
    case usage(ContextUsage)
    /// Newly consumed tokens, as the provider's own ledger reported them for one
    /// request or one turn, never the occupancy of a context window, which `usage`
    /// carries. A report from inside a hand carries that hand's id.
    case tokenSpend(TokenSpend)
    case diff(String)
    case notice(Notice)
    /// The account's usage limit is spent; the turn stops until it resets, at the time given when known.
    case usageLimit(resetsAt: Date?)
    case modeChanged(InteractionMode)
    case models([ModelOption], current: String?)
    case commands([SlashCommand])
    case title(String)
    case assistantMessageID(String)
    case turnCompleted(status: TurnStatus, error: String?)
    case exited(error: String?)
    /// A hand started inside the session, or an existing one was described further.
    case agentStarted(AgentSpawn)
    /// An event from inside a hand's own transcript.
    indirect case agentEvent(agentID: String, ProviderEvent)
    /// The provider's word on a running hand: a progress note, its last tool, its spend.
    case agentProgress(agentID: String, summary: String?, lastTool: String?, tokens: Int?, toolCalls: Int?)
    /// A hand finished, with the provider's summary of its result when it has one.
    case agentFinished(agentID: String, status: TurnStatus, summary: String?)
}

struct ToolUpdate: Sendable {
    var title: String?
    var detail: String?
    var output: String?
    var status: ToolCall.Status?
    var exitCode: Int?
    var edits: [FileEdit]?
    var kind: ToolCall.Kind?
}

@MainActor
protocol ProviderSession: AnyObject {
    var onEvent: ((ProviderEvent) -> Void)? { get set }
    var isRunning: Bool { get }

    /// Launches the provider and returns its session id.
    func start() async throws -> String
    func send(_ input: TurnInput) async throws
    func interrupt() async
    func resolveApproval(_ requestID: String, optionID: String)
    func answerQuestion(_ requestID: String, answers: [String: [String]])
    func compact() async throws
    func stop()
    /// Stops a hand running inside the session. Returns whether the provider could.
    func stopAgent(_ id: String) async -> Bool
}

extension ProviderSession {
    func stopAgent(_ id: String) async -> Bool { false }
}

enum ProviderError: LocalizedError {
    case notInstalled(ProviderKind)
    case notRunning
    case failed(String)

    var errorDescription: String? {
        switch self {
        case .notInstalled(let provider):
            if provider.isAPIKeyBased {
                "\(provider.displayName) needs an API key. Add one in Settings → Providers → \(provider.displayName)."
            } else {
                "\(provider.displayName) is not installed. Install it and sign in with `\(provider.loginCommand)`."
            }
        case .notRunning:
            "The agent session is not running."
        case .failed(let message):
            message
        }
    }
}

enum ToolTitles {
    /// Removes the `/bin/zsh -lc '…'` wrapper providers put around shell commands.
    static func unwrapShell(_ command: String) -> String {
        let trimmed = command.trimmingCharacters(in: .whitespacesAndNewlines)
        for prefix in ["/bin/zsh -lc ", "/bin/bash -lc ", "bash -lc ", "zsh -lc ", "/bin/sh -c ", "sh -c "] {
            guard trimmed.hasPrefix(prefix) else { continue }
            var rest = String(trimmed.dropFirst(prefix.count))
            if rest.count >= 2, let first = rest.first, first == "'" || first == "\"", rest.last == first {
                rest = String(rest.dropFirst().dropLast())
                if first == "'" { rest = rest.replacingOccurrences(of: "'\\''", with: "'") }
            }
            return rest
        }
        return trimmed
    }

    static func relativePath(_ path: String, to root: String) -> String {
        let normalizedRoot = root.hasSuffix("/") ? root : root + "/"
        return path.hasPrefix(normalizedRoot) ? String(path.dropFirst(normalizedRoot.count)) : path
    }

    /// Counts additions and deletions in a unified diff.
    static func diffStats(_ diff: String) -> (additions: Int, deletions: Int) {
        var additions = 0
        var deletions = 0
        for line in diff.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.hasPrefix("+++") || line.hasPrefix("---") { continue }
            if line.hasPrefix("+") { additions += 1 } else if line.hasPrefix("-") { deletions += 1 }
        }
        return (additions, deletions)
    }
}
