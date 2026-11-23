// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import SwiftUI

// Crew: one chat, many hands. With Crew on, the chat's agent leads a team of helper
// agents ("hands") that it sends out on the parts of a big job, in parallel. Claude, Codex
// and Copilot run the hands natively, inside their own session, with the model and effort
// from the pair the user set up; every other provider gets hosted hands, each a thread
// of its own, and hands their reports back to the lead as the next message. A pair may
// also send the hands out on another provider than the lead's (a Claude lead over Gemini
// hands, say): then they are hosted whatever the lead's provider, and the lead asks for
// them the way the other providers do.

/// A purpose-tuned hand configuration on a pair ("quick", "deep"): the lead routes a
/// delegation to it by name and that hand runs on the profile's model and effort instead
/// of the pair's shared worker fields. A nil field inherits the pair's: no provider keeps
/// the hand on the pair's hands provider, no model or effort takes the pair's own.
struct CrewHandProfile: Codable, Hashable, Identifiable, Sendable {
    var id: UUID
    /// The name the lead writes in a delegation's `profile` field; free-form ("quick",
    /// "visual", "docs"), with `purpose` saying what the profile is for.
    var name: String
    var provider: ProviderKind?
    var model: String?
    var effort: String?
    /// What the profile is for, in the user's words ("root causes, reviews, design"): the
    /// lead reads it beside the name, so it routes a task by the work rather than by what
    /// a name suggests. Nil or blank leaves the name to say it; saves from before it have none.
    var purpose: String?

    init(name: String) {
        id = UUID()
        self.name = name
    }

    /// The name without the whitespace a field in Settings can leave around it.
    var trimmedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The purpose on one line, as the lead's brief quotes it (the lines of one typed over
    /// several joined with commas, since profiles are told apart with semicolons), or nil
    /// when there is none.
    var trimmedPurpose: String? {
        let lines = (purpose ?? "").components(separatedBy: .newlines)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        return lines.isEmpty ? nil : lines.joined(separator: ", ")
    }

    /// The profile in words for the lead's brief: "deep (claude-opus-5 on Anthropic)", and
    /// with a purpose "deep (claude-opus-5 on Anthropic): root causes, reviews".
    var briefLabel: String {
        let on = [model, provider?.displayName].compactMap { $0 }.joined(separator: " on ")
        let label = on.isEmpty ? trimmedName : "\(trimmedName) (\(on))"
        return trimmedPurpose.map { "\(label): \($0)" } ?? label
    }

    /// The name as it appears in agent names ("Deep Work" gives "deep-work"), so the
    /// profile a native spawn names can be matched back whatever the spacing or case.
    /// Only ASCII letters and digits are kept and every other run of characters becomes
    /// one hyphen, since that is all the providers' agent names are sure to take; a name
    /// with none of them ("根因") falls back on the profile's id, so its agent still has
    /// a name of its own.
    var agentSlug: String {
        let slug = Self.slug(trimmedName)
        return slug.isEmpty ? "p-" + id.uuidString.prefix(8).lowercased() : slug
    }

    /// `text` the way agent names take it: lowercased, ASCII letters and digits kept,
    /// every other run of characters one hyphen, and none at either end. Empty when the
    /// text has no ASCII letter or digit at all.
    static func slug(_ text: String) -> String {
        var slug = ""
        for character in text.lowercased() {
            if character.isASCII, character.isLetter || character.isNumber {
                slug.append(character)
            } else if let last = slug.last, last != "-" {
                slug.append("-")
            }
        }
        while slug.hasSuffix("-") { slug.removeLast() }
        return slug
    }

    /// Whether a lead's own agent tools can send this profile's hands out, with the pair's
    /// hands on `handsProvider`: only on that provider (a profile that sends hands
    /// elsewhere is the app's business, the lead's agent tools cannot spawn it), and only
    /// when it tunes something (one with neither model nor effort is the shared hand under
    /// another name).
    func routesNatively(on handsProvider: ProviderKind) -> Bool {
        (provider == nil || provider == handsProvider) && (model != nil || effort != nil)
    }

    /// Whether this profile and `other` answer to the same delegation: the same name
    /// whatever the case, or the same agent name on the providers with hands of their own.
    func clashes(with other: CrewHandProfile) -> Bool {
        trimmedName.caseInsensitiveCompare(other.trimmedName) == .orderedSame || agentSlug == other.agentSlug
    }

    /// The profiles a lead can route to, in order: each with a name, since a delegation
    /// names its profile, and only the first of any that answer to the same name, since a
    /// provider registers one agent per name and a lead could only ever reach the first.
    static func usable(_ profiles: [CrewHandProfile]) -> [CrewHandProfile] {
        var kept: [CrewHandProfile] = []
        for profile in profiles where !profile.trimmedName.isEmpty && !kept.contains(where: { $0.clashes(with: profile) }) {
            kept.append(profile)
        }
        return kept
    }

    /// Whether a delegation's `profile` names this one: case-insensitive, and blind to
    /// the whitespace a name typed in Settings or written in a lead's JSON can carry. An
    /// agent slug ("deep-work" from a spawned agent's name) matches the same way, and so
    /// does any spelling that comes to the same slug: the name is put through `slug` first,
    /// so "Deep-Work" or "deep work" reaches the profile the agent name "deep-work" does.
    /// Compared as written, a slug only matched in lower case with hyphens exactly where
    /// they fall. A name with no ASCII letter or digit has no slug and goes by the name
    /// alone; the id-based slug such a profile's agent carries ("p-1a2b3c4d") is ASCII and
    /// still matches.
    func matches(_ name: String) -> Bool {
        let wanted = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmedName.caseInsensitiveCompare(wanted) == .orderedSame { return true }
        let wantedSlug = Self.slug(wanted)
        return !wantedSlug.isEmpty && agentSlug == wantedSlug
    }
}

/// A lead-and-hands pairing: which model runs the hands when a chat on this provider leads.
/// `orchestratorModel` nil means any model on the provider; a nil worker field means the
/// hands inherit the chat's own model or effort. `workerProvider` nil keeps the hands on
/// the lead's provider; set, they run there instead, on `workerModel` or that provider's
/// default model, with `workerEffort` from that provider's own efforts.
struct CrewPair: Codable, Hashable, Identifiable, Sendable {
    var id: UUID
    var provider: ProviderKind
    var orchestratorModel: String?
    var orchestratorEffort: String?
    var workerProvider: ProviderKind?
    var workerModel: String?
    var workerEffort: String?
    /// How many hands may work at once; nil puts no cap on them, as with no pair at all.
    var maxHands: Int?
    /// The name the user gave the pair, shown wherever the pair is named in place of its models; nil or blank is none.
    var name: String?
    /// The purpose-tuned hands of the pair's advanced mode; empty, every hand runs on the
    /// shared worker fields above, as before.
    var handProfiles: [CrewHandProfile]

    /// Every number of hands at once the app offers: a pair's cap, the Crew page's, and
    /// the hands Bottega runs with no cap at all (`CrewLimits.uncappedConcurrency`).
    /// Sixteen lets a big job go out wide in one go; the twenty-one at once that froze the
    /// app stay out of reach.
    static let maxHandsRange = 1...16

    init(provider: ProviderKind) {
        id = UUID()
        self.provider = provider
        handProfiles = []
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = container.value(.id, default: UUID())
        provider = try container.decode(ProviderKind.self, forKey: .provider)
        orchestratorModel = container.value(.orchestratorModel, default: nil)
        orchestratorEffort = container.value(.orchestratorEffort, default: nil)
        workerProvider = container.value(.workerProvider, default: nil)
        workerModel = container.value(.workerModel, default: nil)
        workerEffort = container.value(.workerEffort, default: nil)
        name = container.value(.name, default: nil)
        maxHands = Self.clampedCap(container.value(.maxHands, default: nil))
        handProfiles = container.values(.handProfiles)
    }

    /// The provider the hands run on: the lead's, unless the pair sends them elsewhere.
    var handsProvider: ProviderKind { workerProvider ?? provider }

    /// Whether the hands run on another provider than the lead.
    var sendsHandsElsewhere: Bool { workerProvider.map { $0 != provider } ?? false }

    /// The user's name for the pair, trimmed, or nil when there is none.
    var customName: String? {
        let trimmed = (name ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The hands have moved to another provider: every profile that rides on the hands'
    /// provider (no provider of its own) loses the model and effort it named there, as the
    /// pair's own worker fields do, since they were the old provider's words.
    mutating func clearInheritedProfileChoices() {
        for index in handProfiles.indices where handProfiles[index].provider == nil {
            handProfiles[index].model = nil
            handProfiles[index].effort = nil
        }
    }

    /// A cap kept inside the range, or none.
    static func clampedCap(_ cap: Int?) -> Int? {
        cap.map { min(max($0, maxHandsRange.lowerBound), maxHandsRange.upperBound) }
    }

    /// The pair a project's new chats start on. The rule applies only when all three hold:
    /// Crew is on, the project names a pair, and that pair is in `available` (the pairs
    /// the model picker can offer now). In every other case the result is nil and the
    /// caller uses its own fallback.
    static func projectPair(_ id: UUID?, crewOn: Bool, in available: [CrewPair]) -> CrewPair? {
        guard crewOn, let id else { return nil }
        return available.first { $0.id == id }
    }
}

/// What a session launches with while Crew is on: where the hands run and on what,
/// resolved from the pair, and how many may run at once.
struct CrewLaunch: Hashable, Sendable {
    /// The provider the hands run on: the lead's own, unless the pair sends them out on
    /// another one (see `CrewPair.workerProvider`).
    var handsProvider: ProviderKind
    /// Whether the lead's provider runs the hands inside its own session: only with the
    /// hands on the lead's provider, and only where it has hands of its own (Claude, Codex,
    /// Copilot). Otherwise the hands are hosted threads, whatever the lead runs on, and
    /// the lead asks for them with the delegation block.
    var runsNatively: Bool
    /// The hands' model and provider in words, for the lead's brief, when they run on
    /// another provider than the lead: "Gemini 3.8 Flash on Antigravity".
    var handsLabel: String?
    /// The hands' provider's own model id for the hands, or nil to inherit the lead's model
    /// (on the lead's provider) or take the hands' provider's default (elsewhere).
    var workerModel: String?
    var workerEffort: String?
    /// The cap on hands at work at once: the lower of the pair's and the Crew page's;
    /// nil, with neither set, lets as many out as the work asks for, the hands
    /// Bottega runs `limits.uncappedConcurrency` at a time.
    var maxHands: Int?
    /// The numbers the team runs to (Settings › Crew › Limits): rounds of hands per
    /// request, tasks per block, hands at once with no cap, the hands' budgets and the
    /// watchdog. They bound what Bottega does with the hands; no session launches
    /// with them (see `ThreadRuntime.ensureSession`).
    var limits = CrewLimits.default
    /// The pair's purpose-tuned hands (its advanced mode), the ones a lead can route to
    /// (`CrewHandProfile.usable`): a delegation naming one runs on that profile's model
    /// and effort. Empty, every hand takes the worker fields.
    var handProfiles: [CrewHandProfile] = []
    /// Whether hosted hands get copies of the checkout of their own.
    var isolatesHands = true
    /// Whether Bottega lands the team's finished work itself (see
    /// `AppModel.autoMergeCrewWork`); the lead is told so it never merges by hand.
    var autoMerges = false
    /// What that landing amounts to (the `crewMergeMode` setting): a merge request, a
    /// pushed branch, or a commit in the checkout alone. The lead is told which, so it
    /// never promises a link or a push that is not coming.
    var mergeMode: CrewMergeMode = .localCommit
    /// Whether the lead audits each hand's landed work before it finishes (the
    /// `crewReviewHands` setting): it reads the files the reports name and corrects what
    /// is wrong itself, rather than trusting the reports or sending out a hand to check.
    var reviewsHands = false
    /// Whether every check stays with the lead (the `crewStripsVerificationSteps`
    /// setting): a line of a hosted hand's brief asking it to run one is taken out before
    /// the hand sees it (see `CrewPrompts.verificationSteps(in:)`), and every hand is told
    /// to run none. Off, a hand runs the check its brief names when that check writes no
    /// build output; the build of the whole change stays the lead's either way.
    var stripsVerificationSteps = true
    /// Whether hands queue for the user's go-ahead instead of leaving the moment the lead
    /// asks for them (the `crewHoldsDelegations` setting). On, the app says how many are
    /// waiting and `/go` releases them; off, this is upstream's behaviour.
    var holdsDelegations = false
    /// The projects in the sidebar, the chat's own first: the lead may send a hand to any
    /// of them by name, and its work lands and merges there.
    var projects: [CrewProjectRef] = []
    /// The folders outside the sidebar the user opened to hands (Settings › Crew): a git
    /// repository inside one may take a hand, and joins the sidebar when it does. Empty,
    /// hands stay in the sidebar's projects.
    var allowedProjectRoots: [String] = []
    /// Whether the providers' native hand definitions carry the anti-slop instructions.
    var antiSlopEnabled = true
    /// The user's own standing rules for every lead, from Settings › Crew: the text typed
    /// there, then the contents of the rules file chosen there. The lead's policy ends with
    /// them, native or not (see `CrewPrompts.appendingLeadInstructions`); nil adds nothing.
    var leadInstructions: String?

    /// Whether one more hosted hand may go out with `running` already at work: under the
    /// cap, or with none, under `CrewLimits.uncappedConcurrency`.
    func hasRoom(running: Int) -> Bool {
        running < (maxHands ?? limits.uncappedConcurrency)
    }

    /// The profile a delegation asked for; nil when the name matches none, and the hand
    /// then runs on the worker fields like any unrouted one.
    func profile(named name: String) -> CrewHandProfile? {
        handProfiles.first { $0.matches(name) }
    }
}

/// What a hosted hand is told as its budget for one turn runs down, on any provider: a
/// nudge when it only reads, a word to wrap up, then a stop and one more turn for its
/// report, so the lead always hears back. How much it gets is the user's
/// (`CrewLimits`): the API sessions pace themselves by it from inside the turn, and every
/// other provider gets the stop from the hand's runtime.
enum CrewBudget {
    static func pacingNote(pacingTools: Int) -> String { "[Bottega] You have run \(pacingTools) tools without changing a file. If the task is research, reply with your findings now. Otherwise act on what you know: make the change, or reply with what blocks you." }
    static let wrapUpNote = "[Bottega] Your budget is nearly spent. Finish now: complete the smallest correct version of the task, then reply with your report."
    static let finalNote = "[Bottega] Your budget is spent and your tools are gone. Reply now with your report: what you changed, how far it got, and what is left."
}

/// What a hosted hand may do, worked out from the lead that sends it out. A hand never
/// outranks its lead: the delegation block is the model's own words, so nothing in it can
/// buy the hand more than the user gave the chat. Settings › Crew › Permissions is the
/// ceiling the user allows hands, and the lead's own mode lowers it further. A plan lead is
/// read-only by the user's choice, so its hands plan too, and run Supervised: where a
/// provider's plan mode is only the model's word, an approval nobody answers still holds,
/// and leaving plan mode inside the hand lands it on Supervised rather than the ceiling.
struct CrewHandAuthority: Equatable, Sendable {
    /// What held the hand below the Settings ceiling; nil when nothing did.
    enum Limit: Equatable, Sendable {
        case leadMode(RuntimeMode)
        case planLead
    }

    var runtimeMode: RuntimeMode
    var interactionMode: InteractionMode
    var limit: Limit?

    init(leadRuntime: RuntimeMode, leadInteraction: InteractionMode, settingsCeiling: RuntimeMode) {
        interactionMode = leadInteraction
        if leadInteraction == .plan {
            runtimeMode = RuntimeMode.supervised
            limit = settingsCeiling == .supervised ? nil : .planLead
        } else {
            runtimeMode = settingsCeiling.limited(to: leadRuntime)
            limit = runtimeMode == settingsCeiling ? nil : .leadMode(leadRuntime)
        }
    }

    /// The line the lead's timeline gets when it held a hand back, so a hand stopped on a
    /// card nobody answers is not a mystery. Nil when nothing was held back.
    func note(for hand: String) -> String? {
        switch limit {
        case nil:
            return nil
        case .leadMode(let mode):
            return "Crew: this chat runs \(mode.title), so \(hand) does too: when a hand goes out it never has more than the chat that sent it. What that mode does not allow is asked for in the hand's own card (or refused, on a provider that cannot ask), and a hand left waiting there sits until the watchdog or its time limit stops it. Raise the chat's permissions to give hands more (Settings › Crew › Permissions is the ceiling)."
        case .planLead:
            return "Crew: this chat is in plan mode, so \(hand) only plans, under Supervised: whatever would change a file or run a command waits on an approval nobody answers inside a hand, until the watchdog or its time limit stops it. Leave plan mode to send out hands that edit."
        }
    }

    /// What a plan hand is told at the end of its brief. The lead's task usually asks for a
    /// change, and a hand that tried one would only sit on an approval nobody answers.
    static let planHandNotice = "[Bottega] This chat is in plan mode, so you can only look and plan here. You run Supervised: anything that would change a file or run a command asks for an approval nobody will answer, and you are stopped if you wait on one. Put the changes you would make in your report as a plan, with the exact files and edits, and do not try them."

    /// The providers a plan hand may go out on: the ones where the 2026-10-03 audit (T-063,
    /// vault `04-协调/2026-10-03-T-063-Crew权限不越级计划与审计.md`) found plan and Supervised
    /// held below the prompt, by the provider's own mode or by Bottega's gate. Claude: plan
    /// is the CLI's own permission mode (that plan is not treated as a bypass still wants a live
    /// probe, which the CLI's expired login blocked on 2026-10-03). Codex: plan is only words,
    /// Supervised is its approval and read-only sandbox. Pi: plan blocks every tool but reads in
    /// the extension's tool_call hook. Command Code: the CLI's plan mode, and Bottega's
    /// approval mod, which lets `run_command` and `agent` through unasked (what those two do
    /// was not verified). The user's own allow rules (Claude settings, Codex execpolicy) can let a
    /// command or write through unasked, a plan lead's own session included. Every other provider is
    /// out until its own check is done; this is the one list to extend then.
    static let planHandProviders: Set<ProviderKind> = [.claude, .codex, .pi, .commandcode]

    /// Providers that do hold a plan hand below the prompt, and are still out of the list:
    /// Supervised makes them ask before a read (`OpenAICompatibleSession.requiresApproval`),
    /// so a plan hand there would stop at its first read.
    static let planStallProviders: Set<ProviderKind> = [.deepseek, .meta, .zai]

    /// How a refusal opens, so the report to the lead can tell it from a hand that failed.
    static let refusalPrefix = "Not started:"

    /// Why a hand set to plan cannot go out on `provider`, as the report the lead reads; nil
    /// where it can, and always for a hand that is not on plan. Failing closed: a plan lead is
    /// read-only by the user's choice, and a hand Bottega cannot hold to that does not go.
    static func planRefusal(provider: ProviderKind, interaction: InteractionMode) -> String? {
        guard interaction == .plan, !planHandProviders.contains(provider) else { return nil }
        let why = planStallProviders.contains(provider)
            ? "Supervised makes \(provider.displayName) ask before it reads a file, and nobody answers inside a hand, so a plan hand there would stop at its first read."
            : "nothing shows that \(provider.displayName)'s plan and permission modes stop it from writing."
        return "\(refusalPrefix) this chat is in plan mode, so a hand has to be held to reading, and \(why) Do the reading yourself, or ask the user to leave plan mode. Do not send this task out again while the chat is in plan mode."
    }
}

/// A hand's identity: its name, its colour and the signal flag that is its glyph.
struct CrewPersona: Hashable, Sendable {
    let name: String
    /// The asset with the hand's own flag: one of twenty-five, the code flag of its letter,
    /// drawn as a template so
    /// it takes the hand's colour.
    let asset: String
    /// The colour as 0xRRGGBB.
    let hex: UInt32

    init(_ name: String, _ asset: String, _ hex: UInt32) {
        self.name = name
        self.asset = asset
        self.hex = hex
    }

    var color: Color {
        Color(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }

}

/// The names hands are given, in the order they are sent out: the phonetic alphabet,
/// each with the signal flag of its letter. Past twenty-five the roster starts over with a number after the name.
enum CrewRoster {
    /// The roster in its own colours. Everything else reads `personas`: the same roster in
    /// the colours the theme gives the hands (see `setColors`).
    static let builtInPersonas: [CrewPersona] = [
        CrewPersona("Alfa", "crew-hand-01", 0xFF8A3D),
        CrewPersona("Bravo", "crew-hand-02", 0x3D8BFF),
        CrewPersona("Charlie", "crew-hand-03", 0x34C46A),
        CrewPersona("Delta", "crew-hand-04", 0xA35BE0),
        CrewPersona("Echo", "crew-hand-05", 0xF25C9A),
        CrewPersona("Foxtrot", "crew-hand-06", 0x2BB5B0),
        CrewPersona("Golf", "crew-hand-07", 0xE8B430),
        CrewPersona("Hotel", "crew-hand-08", 0xE84F4F),
        CrewPersona("India", "crew-hand-09", 0x6A5CFF),
        CrewPersona("Juliett", "crew-hand-10", 0x4FD1A1),
        CrewPersona("Kilo", "crew-hand-11", 0x2FB8E6),
        CrewPersona("Lima", "crew-hand-12", 0xB0784A),
        CrewPersona("Mike", "crew-hand-13", 0x8FD14F),
        CrewPersona("November", "crew-hand-14", 0xD946EF),
        CrewPersona("Oscar", "crew-hand-15", 0xF97316),
        CrewPersona("Papa", "crew-hand-16", 0xC084FC),
        CrewPersona("Quebec", "crew-hand-17", 0xB45309),
        CrewPersona("Romeo", "crew-hand-18", 0xF472B6),
        CrewPersona("Sierra", "crew-hand-19", 0x2563EB),
        CrewPersona("Tango", "crew-hand-20", 0x65A30D),
        CrewPersona("Uniform", "crew-hand-21", 0x94A3B8),
        CrewPersona("Victor", "crew-hand-22", 0x0D9488),
        CrewPersona("Whiskey", "crew-hand-23", 0xFACC15),
        CrewPersona("Xray", "crew-hand-24", 0x059669),
        CrewPersona("Yankee", "crew-hand-25", 0x7DD3FC),
    ]

    static func persona(at index: Int) -> CrewPersona {
        let count = personas.count
        let base = personas[((index % count) + count) % count]
        let round = max(0, index) / count
        guard round > 0 else { return base }
        return CrewPersona("\(base.name) \(round + 1)", base.asset, base.hex)
    }

    /// The roster index for an announced hand name, case-insensitive, without any
    /// round suffix ("otto", "Otto 2"). Nil when the name is not on the roster (a
    /// lead inventing names says "Ives", say, which is nobody: the nearest roster
    /// name is "Ivo"). Only exact roster names are authoritative; anything else
    /// falls back to the next sequential hand so an announcement never renames a
    /// stranger into the team.
    static func index(named name: String) -> Int? {
        var bare = name.trimmingCharacters(in: .whitespacesAndNewlines)
        // A trailing round number ("Otto 2") is the display suffix, not the name.
        if let space = bare.lastIndex(of: " "), Int(bare[bare.index(after: space)...]) != nil {
            bare = String(bare[..<space]).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return personas.firstIndex { $0.name.compare(bare, options: .caseInsensitive) == .orderedSame }
    }

    /// The roster persona for an announced hand name, its round kept: "Nova 3" is the
    /// third Nova, "otto" the first Otto. Nil when the name is not on the roster.
    static func persona(named name: String) -> CrewPersona? {
        guard let index = index(named: name) else { return nil }
        let bare = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if let space = bare.lastIndex(of: " "),
           let round = Int(bare[bare.index(after: space)...]), round > 1 {
            return persona(at: index + (round - 1) * personas.count)
        }
        return persona(at: index)
    }
}

/// A hand thread's place in its lead's team: who it is, what it was sent to do, how it
/// runs and how far it has got.
struct CrewHandInfo: Codable, Hashable, Sendable {
    /// Native hands run inside the lead's own provider session; hosted hands are
    /// sessions of their own that Bottega starts and reports back for.
    enum Kind: String, Codable, Sendable {
        case native
        case hosted
    }

    /// Delegated by the lead, or queued by the user while the lead worked. A record from a
    /// build that could also send a message straight to a hand reads as delegated.
    enum Origin: String, Codable, Sendable {
        case delegated
        case queued
    }

    enum Status: String, Codable, Sendable {
        case running
        case completed
        case failed
        case stopped

        var isFinished: Bool { self != .running }
    }

    /// The hand's place in the roster: its name, colour and shape.
    var index: Int
    var task: String
    var kind: Kind
    var origin: Origin
    var status: Status = .running
    /// The hand's report, once it has one: the provider's summary for a native hand, the
    /// final reply for a hosted one.
    var summary: String?
    /// The provider's one-line progress note, or the last tool the hand used.
    var activity: String?
    var toolCalls = 0
    /// What the hand spent, in tokens, as its provider reported it: a native hand's
    /// running total, a hosted hand's recorded turns. Lands on the record as the hand
    /// finishes (see `AppModel.finishCrewHand`); zero where nothing was reported.
    var tokens = 0
    var startedAt = Date.now
    var finishedAt: Date?
    /// When the task was put in the queue that holds work for a hand which cannot start
    /// yet, before anyone dispatched it; a task handed out at once was queued the moment
    /// it went out. Nil for native hands and older records.
    var queuedAt: Date?
    /// When the hand's brief was handed to its session, before its first turn ran. Kept
    /// apart from `queuedAt` so the time a task spent waiting for a slot is not counted as
    /// work, and apart from `startedAt`, which a native hand gets from its provider's
    /// events. Nil for a native hand, whose timing is its provider's to report.
    var executionStartedAt: Date?
    /// When the turn it last reported on finished, however it ended. Nil while the hand is
    /// at work, and for a native hand. A hand steered on after it reported is at work
    /// again, so this clears: its run then spans every turn from the brief to its latest
    /// report.
    var executionFinishedAt: Date?
    /// When the hand's work last started going into the lead's checkout: after the patch
    /// was prepared and after any wait behind another hand's landing, so it measures the
    /// write itself and not the queue. Nil when nothing was ever applied.
    var landingStartedAt: Date?
    /// When that ended, cleanly, in conflict markers or as a kept patch. Nil until it has,
    /// and for a landing that never began.
    var landingFinishedAt: Date?
    /// The provider's id for a native hand: Claude's spawning tool call, Codex's child
    /// thread, Copilot's agent id.
    var nativeID: String?
    /// Claude's task id, which is what stops a running hand.
    var nativeTaskID: String?
    /// The tool row in the lead's timeline that stands for this hand.
    var toolUseID: String?
    /// The delegation a hosted hand belongs to: its report waits for the others.
    var batchID: UUID?
    /// The hand profile the delegation routed this hand to, when the pair has profiles:
    /// shown with the hand and consulted again if the hand is steered on.
    var profile: String?
    /// Whether Bottega can stop this hand where it runs.
    var canStop = true
    /// A native hand whose spawning tool call returned at once: only the provider's own
    /// word ends it, never the tool result.
    var isBackground = true
    /// Set on a hosted hand with a copy of the checkout of its own: the tree the copy
    /// started from, which its work is measured against when it lands. Moves on with
    /// every landing, so a hand steered on afterwards lands only what is new.
    var baseTree: String?
    /// Where the hand's work went the last time it reported.
    var landing: CrewLanding?
    /// When the auto-merge took this hand's work out: the next merge leaves it be. The
    /// hand is what remembers, not the lead's turns: a hand can finish between two of the
    /// lead's turns, and a merge that only counted hands finished since its first
    /// unmerged turn began skipped exactly those, leaving their files in the checkout.
    var mergedAt: Date?
    /// The Markdown file the hand's last report was written to (see `CrewReportFile`), so
    /// the report can be read from disk whatever became of the chat or the run. Nil
    /// before the first report, and where writing it failed.
    var reportPath: String?

    var persona: CrewPersona { CrewRoster.persona(at: index) }
    /// The name the panel shows.
    var displayName: String { persona.name }
    var isFinished: Bool { status.isFinished }
    /// Whether the hand works in a copy of the checkout that Bottega made for it.
    var hasOwnCopy: Bool { baseTree != nil }

    init(index: Int, task: String, kind: Kind, origin: Origin) {
        self.index = index
        self.task = task
        self.kind = kind
        self.origin = origin
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        index = container.value(.index, default: 0)
        task = container.value(.task, default: "")
        kind = container.value(.kind, default: .hosted)
        origin = container.value(.origin, default: .delegated)
        // A status that cannot be read (a record from a newer build, say) reads as
        // stopped, never as completed: a lead told a hand is done when nobody knows what
        // became of it would answer the user for a team that never came back.
        status = container.value(.status, default: .stopped)
        summary = container.value(.summary, default: nil)
        activity = container.value(.activity, default: nil)
        toolCalls = container.value(.toolCalls, default: 0)
        tokens = container.value(.tokens, default: 0)
        startedAt = container.value(.startedAt, default: .now)
        finishedAt = container.value(.finishedAt, default: nil)
        // A hand from a build before these existed has none of them, and reads as one
        // whose timing nobody recorded rather than one that has not begun.
        queuedAt = container.value(.queuedAt, default: nil)
        executionStartedAt = container.value(.executionStartedAt, default: nil)
        executionFinishedAt = container.value(.executionFinishedAt, default: nil)
        landingStartedAt = container.value(.landingStartedAt, default: nil)
        landingFinishedAt = container.value(.landingFinishedAt, default: nil)
        nativeID = container.value(.nativeID, default: nil)
        nativeTaskID = container.value(.nativeTaskID, default: nil)
        toolUseID = container.value(.toolUseID, default: nil)
        batchID = container.value(.batchID, default: nil)
        profile = container.value(.profile, default: nil)
        // Bottega owns a hosted hand's turn, so it can always stop one; only a
        // native hand depends on the provider having said so.
        canStop = container.value(.canStop, default: kind == .hosted)
        isBackground = container.value(.isBackground, default: true)
        baseTree = container.value(.baseTree, default: nil)
        landing = container.value(.landing, default: nil)
        mergedAt = container.value(.mergedAt, default: nil)
        reportPath = container.value(.reportPath, default: nil)
    }
}

/// Where a hand named in a lead's prose leads: its own chat, and what it was sent to do,
/// so the name in the text can be hovered and opened. Keyed by persona name in the
/// timeline's environment.
struct CrewMentionTarget: Hashable, Sendable {
    var threadID: UUID
    var task: String
    var status: CrewHandInfo.Status
    /// The link a mention carries; the timeline opens it as the hand's chat.
    static let scheme = "bottega-hand"
    var url: URL { URL(string: "\(Self.scheme)://\(threadID.uuidString)") ?? URL(fileURLWithPath: "/") }
    /// The hand a mention link points at, or nil for any other URL.
    static func threadID(in url: URL) -> UUID? {
        guard url.scheme == scheme, let host = url.host() else { return nil }
        return UUID(uuidString: host)
    }
}

/// Where a hosted hand's work went when it reported: into the lead's checkout, into
/// it with conflicts left to settle, or into a patch file when it would not apply. A
/// hand with no copy of its own wrote into the checkout as it went, and its landing is
/// the files its tools changed there (see `AppModel.sharedCrewLanding`).
struct CrewLanding: Codable, Hashable, Sendable {
    struct File: Codable, Hashable, Sendable {
        var path: String
        var additions: Int
        var deletions: Int
    }

    /// The files the hand changed, with what the change amounted to.
    var files: [File] = []
    /// Files the three-way merge left conflict markers in.
    var conflicts: [String] = []
    /// Build-output diff sections left out before the patch was parsed.
    var droppedBuildOutputFiles = 0
    /// Where the patch went when none of it could be applied.
    var patchPath: String?
    var error: String?

    /// The hand changed nothing.
    var isEmpty: Bool { files.isEmpty && patchPath == nil && error == nil }
    /// The work is in the checkout, conflict markers or not.
    var landed: Bool { !files.isEmpty && patchPath == nil && error == nil }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        files = container.values(.files)
        conflicts = container.value(.conflicts, default: [])
        droppedBuildOutputFiles = container.value(.droppedBuildOutputFiles, default: 0)
        patchPath = container.value(.patchPath, default: nil)
        error = container.value(.error, default: nil)
    }

    init() {}
}

/// A hand the lead asked for through the delegation block, on a provider that runs no
/// hands of its own. `name` is the hand's announced name when the lead gave one: it is
/// authoritative, so the spawned hand carries exactly that roster name instead of the
/// next sequential one. Nil means the lead did not name it and the roster order decides.
struct CrewDelegation: Hashable, Sendable {
    var task: String
    var prompt: String
    var name: String?
    /// The project the hand works in when the lead named one ("project": a sidebar
    /// project's name or path, or a repository's absolute path); nil keeps it in the
    /// chat's own project.
    var project: String?
    /// The hand profile the lead routed the task to (`"profile"` in the block); nil runs
    /// the hand on the pair's shared worker model and effort.
    var profile: String?
    /// The files this hand is claiming (`"files"`). Hands run in parallel and never see
    /// each other's work, so two entries naming the same file means one silently
    /// overwrites the other. The runtime compares these as the entries are queued and
    /// says so before the hands go out, rather than leaving it to the reports afterwards.
    var files: [String] = []
    /// The coordination row the work was claimed under (`"claim"`), for projects that keep
    /// one. Carried through so a report can name the row it belongs to.
    var claimID: String?
}

/// What the auto-merge does with a team's finished work (Settings › Crew): open a merge
/// request and merge it, as upstream always did; commit and push a branch of its own,
/// leaving the merge request to the user; or commit in the checkout alone and push
/// nothing, for a repository with no remote or a user who pushes in batches.
enum CrewMergeMode: String, Codable, CaseIterable, Sendable {
    case mergeRequest
    case pushBranch
    case localCommit

    /// Whether the mode sends anything to the remote, and so needs an origin to push to.
    var pushes: Bool { self != .localCommit }
}

/// The stages a Crew merge walks through, in this order, one dot each on the merging
/// pill's track. With several projects the merge runs the stages after the first once per
/// project, so the track starts over with the project's name on the words. A merge that
/// only commits, or only pushes, ends early, and its outcome lights the dots it skipped.
/// The commit message is a fixed template, so no stage waits on a model to write it.
enum CrewMergeStage: Int, CaseIterable, Sendable {
    case gathering
    case committing
    case pushing
    case opening
    case merging
    case syncing

    /// The stage in words, as the pill and the sidebar show it.
    var words: String {
        switch self {
        case .gathering: "Gathering the team's files"
        case .committing: "Writing the commit"
        case .pushing: "Pushing the branch"
        case .opening: "Opening the merge request"
        case .merging: "Merging"
        case .syncing: "Bringing the checkout up to date"
        }
    }
}

/// A project the lead may send hands to: its name in the sidebar and its folder.
struct CrewProjectRef: Hashable, Sendable {
    var name: String
    var path: String
}

/// What a hand sends back to its lead.
struct CrewReport: Hashable, Sendable {
    var handIndex: Int
    var task: String
    var origin: CrewHandInfo.Origin
    var status: CrewHandInfo.Status
    var text: String
    /// Where a hosted hand's work went: what landed from its copy, or, for a hand with no
    /// copy of its own, the files its tools changed in the checkout itself; nil where
    /// nothing was recorded.
    var landing: CrewLanding?
    /// The hand's own copy of the checkout, for work that did not land.
    var copyPath: String?
    var elapsed: TimeInterval?
    var toolCalls = 0
    /// The Markdown file the report was also written to (see `CrewHandInfo.reportPath`).
    var reportPath: String?
}

/// The words Crew puts in front of the lead and the hands, per provider.
enum CrewPrompts {
    static let workerAgentName = "crew-worker"
    static let scoutAgentName = "crew-scout"

    /// The agent name a profile's worker variant is registered under on the native
    /// providers ("crew-worker-deep"): the lead routes a task to the profile by
    /// spawning exactly it.
    static func workerAgentName(for profile: CrewHandProfile) -> String {
        "\(workerAgentName)-\(profile.agentSlug)"
    }

    /// The profile an agent name routes to ("crew-worker-deep" gives "deep"), slug
    /// included; the base worker and scout names, and anything else, route nowhere.
    static func profileName(forAgentName agentName: String?) -> String? {
        guard let agentName, agentName.hasPrefix("\(workerAgentName)-") else { return nil }
        return String(agentName.dropFirst(workerAgentName.count + 1))
    }

    /// The profiles a native lead can route to: only ones running on the hands' own
    /// provider and tuning something (see `CrewHandProfile.routesNatively`).
    static func nativeRoutableProfiles(of launch: CrewLaunch) -> [CrewHandProfile] {
        launch.handProfiles.filter { $0.routesNatively(on: launch.handsProvider) }
    }

    /// Where a hosted hand works: a copy of the checkout made for it, or the checkout
    /// itself when the project cannot be copied (no git, no commits yet) or the user
    /// prefers it so.
    enum Workplace {
        case ownCopy(path: String)
        case shared(path: String)
    }

    /// What a hand is told about the checkout it works in and what it may do to it. The
    /// shared case is what every hand lives with on providers that run hands natively.
    /// `stripsChecks` says whether every check is the lead's (see
    /// `CrewLaunch.stripsVerificationSteps`).
    private static func workplaceRules(_ workplace: Workplace, stripsChecks: Bool) -> String {
        switch workplace {
        case .ownCopy(let path):
            let checks = stripsChecks
                ? "Never build, test or verify anything in your copy: the lead runs every check itself, in its own checkout."
                : "Run only the check your brief names in your copy, never a build of the whole project: the lead runs that one itself, in its own checkout."
            return """
            You have your own copy of the project at \(path): a git worktree Bottega made for you from the project's checkout as it was when you were sent out, uncommitted work included. Work in it directly, on the files as they are; your tools already run there. When you report, Bottega carries your changes into that checkout itself. So never commit, branch, stash, push, check out, reset, restore or clean anything, and never make or remove worktrees, whatever the project's own guidelines say about agents and worktrees: this copy already is yours. Do not use git to check your work either.
            \(checks) Anything under the project folder that git does not ignore lands in the lead's checkout, so leave no build output there.
            \(sharedServiceRule)
            """
        case .shared(let path):
            return """
            You work in the lead's checkout at \(path), alongside the lead and the other hands, who are changing other files at the same time. Changes you did not make are expected there: leave every one of them alone, never stash, check out, reset, restore or clean anything, and never move work into a branch or worktree, whatever the project's own guidelines say about agents and worktrees. Do not use git status or git diff to check your work: they show everyone's changes, and the lead does the verifying.
            \(sharedServiceRule)
            """
        }
    }

    /// The user's rule for shared services, which only the user restarts: a hand that
    /// finds one stuck says so rather than bringing it down under everyone else.
    private static let sharedServiceRule = "Never restart, kill or rebuild a shared service (a dev server, a simulator, a CI runner), even one that looks stuck: say in your report that it needs a restart instead."

    /// What a hand is told about checks: none at all while every check is the lead's (the
    /// default), else only the one its brief names, and only one that writes no build output.
    private static func checkRule(stripsChecks: Bool) -> String {
        stripsChecks
            ? "Never run a build, a test, a lint, a type check or any other check of your work, and never wait on one: the lead runs every check itself, in its own checkout. Make the edits your brief names and report."
            : "Run a check only when your brief names one, and only one that writes no build output (a type-check, a lint, a single targeted test): never a build of the whole project and never a check on your own initiative, since the lead runs that one build itself. Make the edits your brief names and report."
    }

    /// How a hand goes about its task, whatever provider runs it. Edits go in as targeted
    /// replacements, never as a whole file written over: the lead and the other hands may
    /// be changing other lines of the same file, and a rewrite takes their work with it.
    private static func howToWork(stripsChecks: Bool) -> String {
        """
        - Start on the task right away: read what you need and no more, then make the change.
        - A brief that names the file, the line and the change needs no second reading: make that edit first, then the next. Think briefly between tool calls; if you find yourself weighing designs for minutes, take the simplest reading of the brief and do it, and mention the choice in your report.
        - Edit files incrementally, with targeted replacements; never rewrite a whole file, since the lead or another hand may be changing other lines of it.
        - Do exactly this task and only this. Do not widen it, do not touch files it does not name unless it cannot be done otherwise (then keep that change minimal and mark the file "outside claim" in your report), and never revert, reformat or clean up work that is not yours.
        - \(checkRule(stripsChecks: stripsChecks))
        - If something blocks you, stop and say so instead of guessing or working around it.
        """
    }

    /// The report a hand ends on: four fixed sections, the last a row the lead can move
    /// straight into the project's coordination file, so closing the loop never means
    /// rewriting a hand's prose, and a hand that verified nothing says so in its own words.
    private static let howToReport = """
    Report in exactly four short sections, in this order, and nothing else: no preamble, no restating the brief, no logs.
    **Files changed**: one path per line, with a few words on what changed there; mark a file outside your claim "(outside claim)". Write "none" if you changed nothing.
    **Verification**: the check your brief named and its pass or fail line, if you ran one; otherwise what you confirmed by reading (the edit read back, its callers found), or "not verified: the lead builds". Never claim more than you saw.
    **Open / blocked**: what is unfinished, uncertain or blocked, said plainly; "none" if nothing is.
    **TASKS row**: when the task is done, one line `<your name>|<task>|<files>|已完成|<YYYY-MM-DD>` the lead can paste into the completed section of the project's TASKS file; otherwise "none", with what is left under Open / blocked.
    """

    /// What a lead is told about the shape of a brief: numbered mechanical steps with the
    /// file, the line, the symbol and the exact change, nothing left open. A brief that
    /// offers alternatives or asks the hand to read and reason first leaves a hand on a
    /// slow model thinking for minutes between tool calls, and some never make an edit.
    private static let briefRule = "Write each brief as numbered, executable steps naming the file, symbol or line anchor, and exact change. Resolve design choices first; ask no questions and offer no alternatives. Give enough context to begin editing within a few tool calls."

    /// What the lead does before it sends anyone out. The user's own working rules put
    /// reading the coordination state, checking for duplicates and claiming a row ahead
    /// of any parallel work; without this the lead delegates into work another chat has
    /// already claimed, which is the failure the rules exist to prevent.
    ///
    /// The state is read from the notes server when one is reachable rather than from the
    /// files: the same INDEX and TASKS, but derived server-side, carrying the standing
    /// rules with them, and recorded in an audit the user can query afterwards — so
    /// whether a lead actually looked before delegating stops being a matter of trust.
    /// Named as a preference, not a dependency: Core knows nothing about which servers are
    /// configured, and a lead with no such tool reads the files exactly as it did before.
    private static let coordinationRule = "Before sending hands out, read the coordination state. If a tool serving the user's notes is there — Studiolo's handshake, read_note and search, or anything like them — take the handshake first and treat what it returns as the authoritative state: it is derived on the server from the same files, it names what is already in progress and the standing rules that bind this work, and the read is recorded. Read the files yourself when no such tool is available (typically an INDEX under 00-总览/ and a TASKS file under 04-协调/, in the project folder or in the notes vault the user pointed you at). Either way, search the in-progress and completed sections for the task's own keywords — feature name, ticket number, path, table, entity — and never re-do something already claimed or finished; if it is claimed, continue it, pick an unclaimed piece, or agree a boundary. Claim one row per hand: line, task, the files it will touch, status, date. Each row must be small enough to verify on its own — \"remaining items\" or \"related polish\" is not a claim. Take any global sequence number (a migration version, a T-number) as max+1 and register it before using it. Then check the current state with grep or ls rather than trusting a plan or an audit written earlier."

    /// What the lead is told about its hands and the coordination state: they leave it to
    /// the lead, so what a hand needs from the notes travels in its brief. Every hand
    /// reading the same notes for itself is the spend the user's budget rules forbid, and
    /// several hands editing one TASKS file at once collide.
    private static let handCoordinationRule = "Your hands leave the coordination state to you: they neither claim nor close rows, and they read the notes only when a brief tells them to, so put what a hand needs from them (its claim, the work in flight beside it) in its brief. If a hand should update its own row, say so in its brief and give the file's absolute path."

    /// What the lead does after the hands are back, before it answers. Without it the
    /// coordination state the lead wrote on the way out is never closed on the way in. The
    /// completed row carries the verification's own pass or fail line, not a sentence
    /// about it, and starts from the row each hand's report proposes.
    private static let closeoutRule = "Close the loop before you answer: move each hand's row from in-progress to completed with the files it changed and the pass or fail line of your verification (the TASKS row its report proposes is the starting point), release any number, file or lock it registered, then write today's log entry and refresh the project's INDEX. If the project keeps no such files, skip this silently rather than inventing them."

    /// A lead may not put a check in a brief: every check is the lead's own. The rule
    /// says so in words the lead cannot mistake for its own. With the user letting hands
    /// run the check their brief names (`CrewLaunch.stripsVerificationSteps` off), the lead
    /// may name a read-only one, and the build of the whole change is still its own.
    private static func verificationRule(stripsChecks: Bool) -> String {
        stripsChecks
            ? "Hands edit or research and report; they never build, test, lint or verify. Do not assign checks or request pass reports. Run required validation yourself in the checkout after the hands return."
            : "Hands edit or research and report. A hand runs a check only when its brief names one, and only one that writes no build output (a type-check, a lint, a single targeted test): never assign a build of the whole project or a check of another hand's work. The build of the whole change is still yours, in the checkout, after the hands return."
    }

    /// The hand rules in one sentence, for a lead whose provider's own agents carry none
    /// of the ones Bottega gives its hands (Codex's worker and explorer): it ends
    /// every brief with them, so those hands keep the same discipline as the others.
    private static func briefedHandRules(stripsChecks: Bool) -> String {
        let checks = stripsChecks ? "run no build, test or other check" : "run only the check the brief names, one that writes no build output, never a build"
        return "edit incrementally and only the files named (mark any other in the report), \(checks), restart no shared service, leave the coordination files to the lead, and report in four sections: Files changed, Verification, Open / blocked, TASKS row."
    }

    /// Where hands work and where their work lands: this chat's project unless an entry
    /// names another — a project in the sidebar, or a git repository inside one of the
    /// folders the user opened to hands in Settings › Crew (`allowedRoots`; none by
    /// default) — with the merge going out per project. The lead's own tools run in this
    /// chat's checkout, but nothing keeps it to that project. What a hand may write in the
    /// user's notes vault follows (see `vaultRule`).
    private static func projectRule(_ projects: [CrewProjectRef], allowedRoots: [String]) -> String {
        let own = projects.first.map { "this chat's project, \($0.name) (`\($0.path)`)" } ?? "this chat's project"
        let others = projects.dropFirst()
        let named = others.isEmpty ? "" : " The other projects in the sidebar: " + others.map { "\($0.name) (`\($0.path)`)" }.joined(separator: ", ") + "."
        let reach: String
        if allowedRoots.isEmpty {
            reach = "Hands may work in the projects in the sidebar and nowhere else. A hand works in \(own) unless it is sent to another: an entry of the delegation block names it with \"project\" (a sidebar project's name or path), and a hand spawned with your own agent tool is told the project's folder in its prompt. A repository outside the sidebar is off limits to hands: an entry naming one sends its hand to this chat's project instead."
        } else {
            let folders = allowedRoots.map { "`\($0)`" }.joined(separator: ", ")
            reach = "Hands may work in the projects in the sidebar, and in git repositories inside the folders the user opened to them: \(folders). A hand works in \(own) unless it is sent to another: an entry of the delegation block names it with \"project\" (a sidebar project's name or path, or the absolute path of a git repository inside one of those folders, which Bottega then adds to the sidebar), and a hand spawned with your own agent tool is told the project's folder in its prompt. Any other repository is off limits to hands: an entry naming one sends its hand to this chat's project instead."
        }
        return "\(reach)\(named) Whatever a hand changes lands in the checkout of the project it worked in, and the team's work is merged per project, one merge request each. Your own tools run in this chat's checkout, and you may read and edit files in any project all the same: what you change elsewhere is merged along with that project's work. Run your own checks in the checkout itself: never make a worktree or a copy of the project for them. \(vaultRule)"
    }

    /// What a hand may write in the user's notes vault, whether it was sent into the vault
    /// or reaches it by path from another project: the vault syncs to the cloud and keeps
    /// every page, and the user's own rules keep agents to a project's logs, coordination
    /// and bug pages there, away from the rules and the framework notes, and never let a
    /// secret into it. The lead writes the rest of the close-out (the INDEX) itself.
    private static let vaultRule = "Any hand that writes in the user's notes vault (an Obsidian vault whose project folders are laid out 00-总览/ to 06-BUG/), whether it was sent into the vault or reaches it by path from another project, writes only under a project's \(CrewCoordination.handWritableFolders.map { $0 + "/" }.joined(separator: ", ")): never anything under _规则/ or 01-框架/, and never a secret (no key, token, password or credential) anywhere in the vault. Put this boundary in the brief of every hand whose task touches the vault."

    /// What a lead is told when the setting has Bottega land the work: the merge is
    /// the app's, not the lead's, whatever else it has been told about merging, and asking
    /// for one is a job it finishes by replying.
    private static let autoMergeRule = "Bottega handles branches, commits and merge requests after all hands return and you finish. Never run or delegate those operations. For merge/status questions, use only the current [Crew] note: if merged, give its link; if files remain, say they land when your answer ends; if merging failed or files are outside every project, give the stated reason and required user action. If nothing is pending, promise no merge. Never claim a merge without the note confirming it."

    /// What the merge above amounts to when the user chose other than a merge request, said
    /// after the rule itself; empty for a merge request, which the rule already describes.
    private static func mergeModeRule(_ mode: CrewMergeMode) -> String {
        switch mode {
        case .mergeRequest:
            ""
        case .pushBranch:
            " Here that merge stops at a pushed branch: Bottega commits the team's work on a branch and pushes it, and opening and merging the merge request is the user's. Nothing is merged, so claim no merge; the [Crew] note names the branch, with the forge's link for opening the merge request when there is one."
        case .localCommit:
            " Here that merge is a local commit: Bottega commits the team's work on the checkout's own branch and pushes nothing, so there is no branch to push, no merge request and no link; the [Crew] note names the commit."
        }
    }

    /// The line in a hand's section of a report message naming the Markdown file the
    /// report was kept in; `CrewReportDigest` reads it back by this prefix.
    static let reportFilePrefix = "Report file:"

    /// What a lead is told when the setting has it check the hands' work: a quick read of
    /// every file a report names, with the fixes made by the lead itself, so the check
    /// never turns into another round of hands checking hands. The build is not part of
    /// it: that is `finalCheckRule`, which holds with the review off too.
    private static let reviewRule = "Review the hands' work before you answer: read every changed file named in the reports and its affected callers, correct integration issues yourself, and disclose your corrections. Never delegate this review."

    /// The one verification of the whole change, the lead's own and never skipped: run
    /// once, with every hand back, never hand by hand (the user's rule: acceptance is done
    /// once, after all the code is in), and its pass or fail line is what the answer and
    /// the completed coordination rows stand on. Kept apart from `reviewRule`, so switching
    /// the review off no longer switches the build off with it.
    private static let finalCheckRule = "Verify once, at the end: with every hand back, run the narrowest build or type-check that proves the whole change, in the checkout, for all of the work together rather than hand by hand, and put its pass or fail line in your answer and in the completed rows. Never call the work done without that line (where nothing buildable changed, say so instead), and report a failure as it is."

    /// The language the lead answers the user in: the user's own rule, every reply in
    /// Simplified Chinese, with code, paths and commands left as they are. Repeated in the
    /// report message, since a model tends to answer in the language of the message in
    /// front of it, and those are Bottega's, in English.
    private static let languageRule = "Reply to the user in Simplified Chinese; keep code identifiers, paths and commands in English."

    /// How the lead tells the user what the hands did: one short human sentence per
    /// hand, hand's name first, everyday words for what changed for the user, file
    /// details on one short second line only, no audit-speak, no icons.
    private static let reportStyleRule = "For each hand, give one plain sentence starting with its name and describing the user-visible result; add the file details only when useful, and keep the whole account bounded to a line or two per hand. Omit repeated summaries, audit jargon and decorative symbols. Say plainly what you still need from the user and what you settled or fixed yourself."

    /// Appended to the lead's system prompt on providers that run hands natively: when to
    /// delegate, how to split the work and what to do with the reports.
    static func policy(for provider: ProviderKind, _ launch: CrewLaunch) -> String {
        let maxHands = launch.maxHands
        let autoMerges = launch.autoMerges
        let reviewsHands = launch.reviewsHands
        let projects = launch.projects
        // The pair's routable profiles, in the words the lead routes with: on Claude and
        // Copilot the profile's agent variant, on Codex the model and effort to pass. Each
        // carries its purpose when it has one, which is what tells the lead which work
        // belongs to it; purposes may hold commas, so the profiles part with semicolons.
        let profiles = nativeRoutableProfiles(of: launch)
        let profileRule: String
        if profiles.isEmpty {
            profileRule = ""
        } else {
            switch provider {
            case .claude, .copilot:
                let variants = profiles.map { "`\(workerAgentName(for: $0))` for \($0.briefLabel)" }.joined(separator: "; ")
                profileRule = " Some hands are tuned for a purpose: route the task to \(variants) instead of the plain `\(workerAgentName)` when the work belongs to that purpose."
            case .codex:
                let routes = profiles.map { profile -> String in
                    var parts: [String] = []
                    if let model = profile.model { parts.append("`model` = \(model)") }
                    if let effort = profile.effort { parts.append("`reasoning_effort` = \(effort)") }
                    let route = "\(profile.trimmedName) (pass \(parts.joined(separator: " and ")))"
                    return profile.trimmedPurpose.map { "\(route): \($0)" } ?? route
                }.joined(separator: "; ")
                profileRule = " Some hands are tuned for a purpose: route the task by passing the profile's own knobs in `spawn_agent` — \(routes) — when the work belongs to that purpose."
            default:
                profileRule = ""
            }
        }
        let howToSpawn: String
        let howToWait: String
        switch provider {
        case .claude:
            howToSpawn = "Two agent types are yours: `\(workerAgentName)` (edits files, runs commands) and `\(scoutAgentName)` (read-only research). Spawn them with the Agent tool and prefer them over other agents while Crew is on: they run on the model and effort the user chose for hands.\(profileRule)"
            howToWait = "Launch every hand for a job in one message so they run in parallel, in the foreground" + (maxHands.map { ", and run at most \($0) at once." } ?? ".")
        case .codex:
            // Codex's own worker and explorer carry none of the rules the other providers'
            // hands get in their definitions (`workerPrompt`), so the lead writes them in.
            howToSpawn = "Spawn hands with `spawn_agent`: the `worker` agent for anything that edits files or runs commands, the `explorer` agent for read-only research.\(profileRule) Those agents carry none of the rules Bottega gives its own hands, so end every brief with them: \(briefedHandRules(stripsChecks: launch.stripsVerificationSteps))"
            howToWait = "Spawn every hand for a job before waiting, so they run in parallel, and collect them with `wait_agent`; never leave a hand running when you answer." + (maxHands.map { " Run at most \($0) at once." } ?? "")
        case .copilot:
            howToSpawn = "Two agents are yours: `\(workerAgentName)` (edits files, runs commands) and `\(scoutAgentName)` (read-only research). Start them with the task tool and prefer them over other agents while Crew is on: they run on the model and effort the user chose for hands.\(profileRule)"
            howToWait = "Start every hand for a job at once so they run in parallel" + (maxHands.map { ", and run at most \($0) at a time." } ?? ".")
        default:
            howToSpawn = ""
            howToWait = maxHands.map { "Run at most \($0) hands at once." } ?? "Send every hand for a job out at once so they run in parallel."
        }
        let policy = """
        # Crew

        The user switched on Crew for this chat: you lead a team of helper agents, called hands, that work in parallel in your checkout. Switching Crew on is the user asking you to use them, so it overrides any standing rule that says not to spawn agents unless asked. \(howToSpawn) The agents you spawn this way are the Crew hands themselves: Bottega shows each one in the team's panel under a roster name and posts its outcome in the chat. There is no other launcher and no other kind of hand, so never tell the user that Crew hands cannot be started here; when the user asks for hands, spawn them this way.

        Work alone by default. Hands are for two situations only: work that is genuinely parallel — independent pieces in files that do not overlap — and work too large for one context. Everything else stays with you: reading, reviewing, planning, running commands, a change in one or two files. A hand that does work you could have done inline is spent budget, not parallelism.

        Before you send anyone out, say in one line how many hands you want, what each will do, and roughly what it will cost, and let the user answer. Before any code has been written — while the job is still research, review, design or arguing a plan — at most one hand goes out, never a panel of viewpoints in parallel. Size the team to the work, not to how many hands are free: a handful of genuinely independent pieces is a team; steps that depend on each other are one hand's task, or your own.

        When you delegate:
        - \(coordinationRule)
        - \(handCoordinationRule)
        - \(howToWait)
        - Give each hand one self-contained task with the exact files, symbols and acceptance criteria it needs. Hands share the checkout but not your context, so write the task as if to a capable colleague who has read nothing yet.
        - \(briefRule)
        - \(verificationRule(stripsChecks: launch.stripsVerificationSteps))
        - \(projectRule(projects, allowedRoots: launch.allowedProjectRoots))
        - Split the work so no two hands edit the same file and no hand's task rests on another hand's unfinished changes: they run in parallel and never see each other's work. Keep integration, verification and the final answer for yourself: never send out a hand to verify, redo or finish another hand's work.
        - Tell the user in one line which hands you sent out and what each one does. Bottega names the hands in roster order (\(CrewRoster.personas.map(\.name).joined(separator: ", ")), then \(CrewRoster.personas[0].name) 2 and so on): announce each hand by its task and use exactly those names in that order, never invented ones. There is no hand called Ives; the roster has Ivo.
        - After delegating, yield until reports arrive.
        - Hands go out through the tools above, never through a fenced crew block: that is the delegation format for providers without agent tools of their own. If you end a reply with one anyway, Bottega still sends those hands out as threads of their own, but your turn ends there and their reports come back as a later message.

        When they report back:
        - The checkout changes under you while hands work, and the user may be editing too. Never use git status or git diff to check on a hand, and never reconcile, revert, stash or move changes you did not make.
        - Take a report as done work: read the files it names if something matters, but do not redo the task and do not start it over because the tree looks different from what you expected. A hand that failed leaves its part to you: do it or send it out again. A hand the user stopped leaves its part alone unless the user asks.
        \(reviewsHands ? "- " + reviewRule + "\n" : "")- Then finish the job: integrate what came back, run the one verification and answer the user. \(finalCheckRule)
        - \(closeoutRule)
        - \(languageRule) \(reportStyleRule)
        \(autoMerges ? "\n" + autoMergeRule + mergeModeRule(launch.mergeMode) + "\n" : "")
        """
        return appendingLeadInstructions(launch.leadInstructions, to: policy, heading: "# The user's rules for leading")
    }

    /// A lead's policy with the user's own rules for every lead after it
    /// (`CrewLaunch.leadInstructions`), under a heading in the policy's own form: they bind
    /// the lead as much as the policy does, and where the two disagree the user's win,
    /// since the user wrote them for exactly this. The policy is left as it is without any.
    private static func appendingLeadInstructions(_ instructions: String?, to policy: String, heading: String) -> String {
        guard let instructions = instructions?.trimmingCharacters(in: .whitespacesAndNewlines), !instructions.isEmpty else { return policy }
        return policy.trimmingCharacters(in: .whitespacesAndNewlines) + """


            \(heading)

            The user set these rules for every lead. They bind you as much as everything above, and where the two disagree, follow these.

            \(instructions)
            """
    }

    /// The system prompt a worker hand runs with: the same rules a hosted hand's brief
    /// carries (`howToWork`, `workplaceRules`), in one paragraph, since a native hand's
    /// task is the lead's own words and nothing of ours goes with it but this.
    /// `stripsVerificationSteps` is the check rule (see `CrewLaunch.stripsVerificationSteps`).
    static func workerPrompt(stripsVerificationSteps: Bool) -> String {
        let checks = stripsVerificationSteps
            ? "Never run a build, a test, a lint or any other check of your work: the lead runs every check itself."
            : "Run a check only when your task names one, and only one that writes no build output (a type-check, a lint, a single targeted test), never a build of the whole project: the lead runs that one itself."
        return """
        You are a Crew hand in Bottega: one of several helpers working in parallel for a lead agent, in the lead's own checkout, where the lead and the other hands are changing other files at the same time. Do exactly the task you were given, and only that: do not widen it, do not touch files it does not name unless the task cannot be done otherwise (then keep that change minimal and mark the file "outside claim" in your report), and never revert, reformat or clean up work that is not yours. Find each symbol or line the task names in the file as it is now before you change it, and edit incrementally, with targeted replacements: never rewrite a whole file, since others may be changing other lines of it. Changes you did not make are expected in the checkout: leave them alone, never stash, check out, reset, restore or clean anything, and never move work into a branch or worktree, whatever the project's own guidelines say about agents and worktrees. Do not use git status or git diff to check your work: they show everyone's changes. \(checks) \(sharedServiceRule) Leave the coordination files (TASKS, logs, INDEX) to the lead unless your task says otherwise. Make the edits your task names and report. If something blocks you, say so instead of guessing.

        \(howToReport)
        """
    }

    /// The system prompt a scout hand runs with.
    static let scoutPrompt = """
    You are a Crew hand in Bottega: a read-only researcher working in parallel for a lead agent, in the lead's own checkout, where the lead and other hands are changing files at the same time; uncommitted changes there are theirs and expected. Answer exactly the question you were given, from the code and sources you can read. Change nothing. \(sharedServiceRule)

    Reply with a short report the lead can act on: the findings, with file paths and line references, and anything that contradicts what the lead assumed. No preamble.
    """

    /// Claude's `--agents` definitions: the two hands on the pair's model and effort.
    static func claudeAgents(_ launch: CrewLaunch) -> JSONValue {
        let antiSlop = AntiSlopPolicy.instructions(enabled: launch.antiSlopEnabled)
        var worker: [String: JSONValue] = [
            "description": "Crew hand that implements one delegated task: edits files, runs commands. Use proactively when a request splits into independent pieces or touches several parts of the codebase.",
            "prompt": .string(workerPrompt(stripsVerificationSteps: launch.stripsVerificationSteps) + "\n\n" + antiSlop),
            "background": false,
        ]
        // The scout stays read-only (no Bash, no Edit), but gets the language server for
        // definitions and references, and the deferred-tool loader for tools the session
        // defers until asked for. The CLI leaves out a listed tool it does not have (LSP
        // without a language server plugin), so the list may name more than is installed.
        var scout: [String: JSONValue] = [
            "description": "Crew hand for read-only research: finds files, reads code, gathers facts and reports back. Use proactively when a job needs many files or sources read.",
            "prompt": .string(scoutPrompt + "\n\n" + antiSlop),
            "tools": ["Read", "Grep", "Glob", "LSP", "ToolSearch", "WebFetch", "WebSearch"],
            "background": false,
        ]
        if let model = launch.workerModel, model != "default" {
            worker["model"] = .string(model)
            scout["model"] = .string(model)
        }
        if let effort = launch.workerEffort, !effort.isEmpty {
            worker["effort"] = .string(effort)
            scout["effort"] = .string(effort)
        }
        var agents: [String: JSONValue] = ["\(workerAgentName)": .object(worker), "\(scoutAgentName)": .object(scout)]
        // One worker variant per routable profile: the lead routes a task to the profile
        // by spawning the variant, whose model and effort are the profile's own.
        for profile in nativeRoutableProfiles(of: launch) {
            var variant = worker
            variant["description"] = .string(variantDescription(for: profile))
            if let model = profile.model, model != "default" { variant["model"] = .string(model) }
            if let effort = profile.effort, !effort.isEmpty { variant["effort"] = .string(effort) }
            agents[workerAgentName(for: profile)] = .object(variant)
        }
        return .object(agents)
    }

    /// How a profile's worker variant describes itself in the agent list a lead picks from
    /// (Claude's and Copilot's alike): the profile's name, and what it is for when the user
    /// said, so the tool's own description already routes the work.
    private static func variantDescription(for profile: CrewHandProfile) -> String {
        let name = profile.trimmedName
        guard let purpose = profile.trimmedPurpose else {
            return "Crew hand that implements one delegated task, tuned as the '\(name)' profile. Prefer it for work that belongs to '\(name)' over the plain \(workerAgentName)."
        }
        return "Crew hand that implements one delegated task, tuned as the '\(name)' profile for: \(purpose). Prefer it for that work over the plain \(workerAgentName)."
    }

    /// Copilot's `customAgents`: the same two hands, in the CLI's own shape.
    static func copilotAgents(_ launch: CrewLaunch) -> [JSONValue] {
        let antiSlop = AntiSlopPolicy.instructions(enabled: launch.antiSlopEnabled)
        func agent(name: String, display: String, description: String, prompt: String, tools: [String]?) -> JSONValue {
            var object: [String: JSONValue] = [
                "name": .string(name),
                "displayName": .string(display),
                "description": .string(description),
                "prompt": .string(prompt),
                "infer": true,
            ]
            if let tools { object["tools"] = .array(tools.map(JSONValue.string)) }
            if let model = launch.workerModel, model != "auto" { object["model"] = .string(model) }
            if let effort = launch.workerEffort, !effort.isEmpty { object["reasoningEffort"] = .string(effort) }
            return .object(object)
        }
        let worker = agent(
            name: workerAgentName,
            display: "Crew worker",
            description: "Crew hand that implements one delegated task: edits files, runs commands. Use when a request splits into independent pieces.",
            prompt: workerPrompt(stripsVerificationSteps: launch.stripsVerificationSteps) + "\n\n" + antiSlop,
            tools: nil
        )
        var agents = [
            worker,
            // The scout's prompt keeps it read-only; the CLI's tool names are not pinned
            // here, since a name it does not know could refuse the whole session.
            agent(
                name: scoutAgentName,
                display: "Crew scout",
                description: "Crew hand for read-only research: finds files, reads code and reports back. Use when a job needs many files or sources read.",
                prompt: scoutPrompt + "\n\n" + antiSlop,
                tools: nil
            ),
        ]
        // One worker variant per routable profile, as on Claude: the lead routes a task
        // to the profile by starting the variant.
        for profile in nativeRoutableProfiles(of: launch) {
            guard case .object(var variant) = worker else { continue }
            variant["name"] = .string(workerAgentName(for: profile))
            variant["displayName"] = .string("Crew worker · \(profile.trimmedName)")
            variant["description"] = .string(variantDescription(for: profile))
            if let model = profile.model, model != "auto" { variant["model"] = .string(model) }
            if let effort = profile.effort, !effort.isEmpty { variant["reasoningEffort"] = .string(effort) }
            agents.append(.object(variant))
        }
        return agents
    }

    /// Codex's config overrides for the thread: the hands' model and effort, and how many
    /// may run at once; uncapped, Codex keeps its own limit. With profiles to route to,
    /// the spawn tool's model and effort knobs are exposed, so the lead can pass the
    /// profile's own when it spawns the hand (the session matches them back).
    static func codexConfig(_ launch: CrewLaunch) -> [String: JSONValue] {
        var agents: [String: JSONValue] = [:]
        if let cap = launch.maxHands { agents["max_concurrent_threads_per_session"] = .int(cap) }
        if let model = launch.workerModel, !model.isEmpty { agents["default_subagent_model"] = .string(model) }
        if let effort = launch.workerEffort, !effort.isEmpty { agents["default_subagent_reasoning_effort"] = .string(effort) }
        var features: [String: JSONValue] = ["multi_agent": true]
        if !nativeRoutableProfiles(of: launch).isEmpty {
            features["multi_agent_v2"] = .object(["expose_spawn_agent_model_overrides": true])
        }
        return ["agents": .object(agents), "features": .object(features)]
    }

    // MARK: - hosted hands

    /// The lead's team in a line, for the front of its messages: it should never have to
    /// guess who is still out or what came back.
    static func teamStatus(_ hands: [CrewHandInfo], now: Date = .now) -> String? {
        guard !hands.isEmpty else { return nil }
        let parts = hands.map { info -> String in
            let name = info.persona.name
            switch info.status {
            case .running:
                let minutes = Int(now.timeIntervalSince(info.startedAt) / 60)
                return "\(name) (working\(minutes > 0 ? " for \(minutes)m" : ""))"
            case .completed:
                if let landing = info.landing {
                    if landing.isEmpty { return "\(name) (done, changed nothing)" }
                    if landing.landed { return "\(name) (done, landed \(landing.files.count == 1 ? "1 file" : "\(landing.files.count) files"))" }
                    return "\(name) (done, patch kept)"
                }
                return "\(name) (done)"
            case .failed: return "\(name) (failed)"
            case .stopped: return "\(name) (stopped)"
            }
        }
        return "Your hands so far: " + parts.joined(separator: ", ") + "."
    }

    /// The standing rules for a lead on a provider that runs no hands of its own: when to
    /// delegate, how, and what the reports mean. An API session keeps this in its system
    /// prompt, once; a CLI session gets it in front of every message.
    static func fallbackPolicy(maxHands: Int?, isolated: Bool, autoMerges: Bool = false, reviewsHands: Bool = false, stripsVerificationSteps: Bool = true, hands: String? = nil, projects: [CrewProjectRef] = [], profiles: [CrewHandProfile] = [], allowedRoots: [String] = []) -> String {
        let whereHandsWork = isolated
            ? "Each hand works in a copy of its project of its own and Bottega lands its changes in that project's checkout when it reports"
            : "The hands work in your checkout"
        let team = maxHands.map { "a team of up to \($0) helper agents" } ?? "a team of helper agents"
        // Hands on another provider are a different model from the lead, chosen for speed
        // more often than not: the lead hears what they are, so it briefs them as such and
        // keeps the thinking, and it is told the block is the only way to them, since its own
        // agent tools would run hands on its own model instead.
        let whoTheHandsAre = hands.map {
            " Your hands run on \($0), a different model from yours: it is quick, so give each one a well-bounded task with everything it needs written down, and keep the design, the judgement calls and the integration for yourself. The block is the only way to send hands out: never use an agent, task or sub-agent tool of your own, which would run hands on your own model instead of the one the user chose."
        } ?? ""
        // Whatever the provider, a hand has the read, edit and run tools: on the API
        // providers the app gives it those itself, so no hand is ever a text-only
        // researcher. How far it may use them is the chat's own permissions at most
        // (`CrewHandAuthority`), which change mid-chat while this text may not (an API
        // session keeps it in its system prompt), so it says what holds in every mode.
        let whatHandsCanDo = " Every hand, whatever its provider, has the read, edit and run tools, and may use them as far as this chat's own permissions go and no further than the ceiling the user set in Settings › Crew: a hand never gets more than the chat that sends it, and a chat in plan mode sends hands that only plan. Where the permissions reach, brief it to do the work, not just to describe it. Where they do not, brief it to find out and report what it would change: nobody answers an approval inside a hand, and a hand left waiting on one sits there until the watchdog or its time limit stops it."
        // A pair in advanced mode tunes some hands for a purpose: the lead hears which, by
        // name and, where the user gave one, by what each is for, and routes a task to one
        // by name. Purposes may hold commas, so the profiles part with semicolons.
        let profileRule = profiles.isEmpty ? "" : " Some hands are tuned for a purpose: \(profiles.map(\.briefLabel).joined(separator: "; ")). Route a task to one with \"profile\", as in `{\"task\": \"...\", \"prompt\": \"...\", \"profile\": \"\(profiles[0].trimmedName)\"}`; a task that fits none of them goes without a profile and runs on the hands' shared model, as does one whose profile names none of these."
        return """
        [Crew is on] You lead \(team) ("hands"). Work alone by default. Hands are for two situations only: work that is genuinely parallel — independent pieces in files that do not overlap — and work too large for one context. Reading, reviewing, planning, running commands and a change in one or two files all stay with you; a hand doing work you could have done inline is spent budget, not parallelism. Before you send anyone out, say in one line how many hands you want, what each will do and roughly what it will cost, and let the user answer. Before any code has been written — research, review, design, arguing a plan — at most one hand goes out, never a panel of viewpoints in parallel. Size the team to the work, not to how many hands are free.         Finish your reply with one fenced block

        ```crew
        [{"task": "short title", "prompt": "complete, self-contained instructions with the exact files and acceptance criteria", "files": ["path/to/one.swift", "path/to/another.swift"]}]
        ```

        and stop there: do not wait, poll or verify anything after it. When a request is yours to do alone, do it and end with no block at all: an empty block sends no hands and is not needed. An entry may also carry its hand's announced name, as in `{"task": "...", "prompt": "...", "name": "Otto"}`: the announced name is authoritative and the spawned hand carries exactly it, so repeating the same block spawns the same names. An entry for work in another project carries `"project"` with that project's name or path, as in `{"task": "...", "prompt": "...", "project": "gaze-site"}`. Name new hands with the next roster names in order after the team listed above (\(CrewRoster.personas.map(\.name).joined(separator: ", ")), then \(CrewRoster.personas[0].name) 2 and so on), and omit the name when unsure: the next hands in order go out instead. Never invent names outside the roster: there is no hand called Ives (the roster has Ivo), and an unknown name falls back to the next hand in order rather than renaming anyone.\(profileRule) Every entry carries `"files"`: the files that hand will change, as project-relative paths. No two entries may name the same file — hands run in parallel and never see each other's work, so an overlap means one silently overwrites the other, and the app tells you when your entries overlap. An entry may also carry `"claim"` naming the coordination row the work was claimed under. Inside a prompt never open a fenced code block of your own (three backticks would end the crew block early and no hand would go out): describe code in words, quote identifiers with single backticks, or indent a snippet by four spaces. Each entry goes out to a hand the moment its closing brace streams, before the block is finished, so write the entries in the order the hands should start and complete one entry before beginning the next. \(whereHandsWork); hands never see your context, so write every prompt for a capable colleague who has read nothing yet, with the exact files, symbols and acceptance criteria, give no two hands the same file, and give no hand work that rests on another hand's unfinished changes: they run in parallel and never see each other's work. Keep each brief bounded: one hand, one task, one report you can act on. \(coordinationRule) \(handCoordinationRule) \(briefRule) \(verificationRule(stripsChecks: stripsVerificationSteps)) \(projectRule(projects, allowedRoots: allowedRoots)) A hand can be sent to read and report as well as to change files, so the reading goes out in parallel too. Say in one line which hands you sent out and what each one does.\(whoTheHandsAre)\(whatHandsCanDo) The reports arrive as a later message with the work already in place: build on them, do not redo them, never send out hands to verify or redo other hands, and never use git status or git diff to check on hands, since the checkout changes under you while they work. A message that opens with [Crew] is from Bottega, not the user.\(reviewsHands ? " " + reviewRule : "") \(finalCheckRule)\(autoMerges ? " " + autoMergeRule : "") \(languageRule) \(reportStyleRule)
        """
    }

    /// The policy for a lead whose session keeps it in its system prompt: the API
    /// providers, and the providers with hands of their own whose pair sends the hands out
    /// on another provider.
    static func fallbackPolicy(_ launch: CrewLaunch) -> String {
        let policy = fallbackPolicy(maxHands: launch.maxHands, isolated: launch.isolatesHands, autoMerges: launch.autoMerges, reviewsHands: launch.reviewsHands, stripsVerificationSteps: launch.stripsVerificationSteps, hands: launch.handsLabel, projects: launch.projects, profiles: launch.handProfiles, allowedRoots: launch.allowedProjectRoots)
            // What the auto-merge amounts to, after the rest: the merge rule inside says
            // what Bottega handles, this says where it stops.
            + (launch.autoMerges ? mergeModeRule(launch.mergeMode) : "")
        return appendingLeadInstructions(launch.leadInstructions, to: policy, heading: "[Crew: the user's rules for leading]")
    }

    /// The true merge state for the front of a lead's message, so it never promises a merge that already happened or claims one that failed.
    static func mergeStatus(merges: [CrewMergeRecord], unmergedFiles: Int, now: Date = .now) -> String? {
        func files(_ count: Int) -> String { count == 1 ? "1 file" : "\(count) files" }
        func when(_ date: Date) -> String { date.formatted(date: .omitted, time: .shortened) }
        func merged(_ record: CrewMergeRecord) -> String {
            let scope = record.project.map { " in \($0)" } ?? ""
            // A record from before the modes is a merge request's, as every merge was then.
            switch record.mode ?? .mergeRequest {
            case .mergeRequest:
                let link = [record.label, record.url?.absoluteString].compactMap { $0 }.joined(separator: " ")
                return "the team's work already merged as \(link)\(scope) at \(when(record.at)) (\(files(record.files)))."
            case .pushBranch:
                let open = record.url.map { " (a merge request can be opened at \($0.absoluteString))" } ?? ""
                return "the team's work was already pushed as branch \(record.label ?? "of its own")\(scope) at \(when(record.at)) (\(files(record.files)))\(open); no merge request was opened, so the default branch does not have it yet."
            case .localCommit:
                return "the team's work was already committed locally as \(record.label ?? "a commit")\(scope) at \(when(record.at)) (\(files(record.files))); nothing was pushed."
            }
        }
        func pendingLine(_ prefix: String) -> String {
            let one = unmergedFiles == 1
            return "\(prefix) \(one ? "is" : "are") not merged yet; \(one ? "it goes" : "they go") out by \(one ? "itself" : "themselves") once you finish this answer with every hand back."
        }
        let pending = unmergedFiles > 0
            ? pendingLine("\(files(unmergedFiles)) changed since then")
            : "Nothing has changed since, so no merge is pending and none is coming."
        guard let last = merges.last else {
            guard unmergedFiles > 0 else { return nil }
            return "Merge state: \(pendingLine("\(files(unmergedFiles)) changed in this chat"))"
        }
        switch last.outcome {
        case .merged:
            return "Merge state: \(merged(last)) \(pending)"
        case .failed:
            return "Merge state: the last merge failed at \(when(last.at)): \(last.detail ?? "no reason recorded"). The work is still in the checkout and is retried once you finish this answer; if the reason needs the user (a sign-in, a conflict), say exactly that."
        case .held:
            return "Merge state: the merge is waiting at \(when(last.at)): \(last.detail ?? "another chat holds this job's files") The work is still in the checkout. Explain which chat holds it; unfinished work there must finish before these files can merge. Crew retries automatically while this job remains eligible. Do not claim a successful merge."
        case .stray:
            // A stray note right after a merge is a footnote to it: the merge is the news.
            var parts = ["Merge state:"]
            if let landed = merges.last(where: { $0.outcome == .merged }) { parts.append(merged(landed)) }
            parts.append("At \(when(last.at)) \(last.detail ?? "files were changed outside every project")")
            parts.append(pending)
            return parts.joined(separator: " ")
        }
    }

    /// The user's enabled Crew setting expressed alongside their request. Codex's
    /// manual delegation mode needs a user/project/skill request, not only the standing
    /// developer policy. Keep this out of automatic reports, which have their own limit.
    static func delegationRequest(_ launch: CrewLaunch) -> String {
        let route = launch.runsNatively
            ? "Use the configured Crew agent tools and hand profiles."
            : "Delegate through the final crew block so Bottega runs the selected hands; do not use native agent tools."
        return """
        [Crew setting: enabled]
        Hands are available. Use them only when this request is genuinely parallel — independent pieces in files that do not overlap — or too large for one context, and only after you have said how many hands and roughly what they cost. Reading, reviewing, planning and a change in one or two files stay with you. \(route) Follow any explicit request below to work alone or limit delegation.

        ---


        """
    }

    /// In front of the user's own message: the team so far, when there is one.
    static func fallbackTurnNote(team: String?, merge: String? = nil) -> String {
        let parts = [team, merge].compactMap { $0 }
        guard !parts.isEmpty else { return "" }
        return "[Crew] " + parts.joined(separator: " ") + "\n\n---\n\n"
    }

    /// In front of a report message: the hands are back, and the lead's job is to
    /// finish, not to send out more. `canDelegate` says a round of hands is still left
    /// for what failed or turned out to be missing; otherwise the block is not offered.
    static func fallbackReportNote(team: String?, merge: String? = nil, canDelegate: Bool) -> String {
        let more = canDelegate
            ? "If the reports open up the next stage of the work, or a hand failed, or a piece of the user's request is still undone, send hands out again for exactly that, with the same ```crew block at the end of your reply; never for verifying, redoing or finishing what a hand already did."
            : "Send out no more hands for this request; whatever is left, do yourself."
        return "[Crew] Your hands reported back below." + (team.map { " " + $0 } ?? "") + (merge.map { " " + $0 } ?? "") + " " + more + "\n\n---\n\n"
    }

    /// Policy and note together, for a CLI provider with no system prompt to keep the
    /// policy in.
    static func fallbackPreamble(_ launch: CrewLaunch, team: String?, merge: String? = nil) -> String {
        fallbackPolicy(launch) + "\n\n" + (fallbackTurnNote(team: team, merge: merge).isEmpty ? "---\n\n" : fallbackTurnNote(team: team, merge: merge))
    }

    static func fallbackReportPreamble(_ launch: CrewLaunch, team: String?, merge: String? = nil, canDelegate: Bool) -> String {
        fallbackPolicy(launch) + "\n\n" + fallbackReportNote(team: team, merge: merge, canDelegate: canDelegate)
    }

    /// What the lead hears when its delegation block is refused: the request has had its
    /// rounds of hands (`CrewLimits.maxDelegationRounds`), and the rest is the lead's own.
    static func heldBackMessage(count: Int, maxDelegationRounds: Int) -> String {
        """
        Crew held back \(count == 1 ? "a hand" : "\(count) hands").
        None of the hands you asked for went out: this request has had all \(maxDelegationRounds) of its rounds of hands already. Do the rest yourself now, without git status or git diff on the hands' work, and answer the user.
        """
    }

    /// The opening fence of a delegation block: three backticks, the word crew in any
    /// case, and the end of that line.
    private static func delegationOpener(in text: String) -> Range<String.Index>? {
        text.firstMatch(of: #/```[ \t]*crew(?:-sent|-leaving)?[ \t]*\r?\n/#.ignoresCase())?.range
    }

    /// Every three backticks that open a line from `start` on, in order. A fence with an
    /// info string after it counts too: inside a prompt that is what a quoted snippet
    /// looks like, and the parser needs those to know where not to cut.
    private static func lineStartFences(in text: String, from start: String.Index) -> [Range<String.Index>] {
        var fences: [Range<String.Index>] = []
        var cursor = start
        while let fence = text[cursor...].firstRange(of: "```") {
            if fence.lowerBound == text.startIndex || text[text.index(before: fence.lowerBound)].isNewline {
                fences.append(fence)
            }
            cursor = fence.upperBound
        }
        return fences
    }

    /// Where the delegation block sits in a reply: from its opening fence up to and
    /// including its closing fence, which is the last three backticks that open a line
    /// after the opener, never the first. A prompt may quote a fenced snippet of its own,
    /// and cutting at the first fence inside it lost the whole block. Without a closing
    /// fence at all (the reply is still streaming, or the lead forgot it) the block runs
    /// to the end of the text.
    static func delegationBlockRange(in text: String) -> Range<String.Index>? {
        guard let opener = delegationOpener(in: text) else { return nil }
        let end = lineStartFences(in: text, from: opener.upperBound).last?.upperBound ?? text.endIndex
        return opener.lowerBound..<end
    }

    /// Whether the reply has a delegation block at all, readable or not.
    static func hasDelegationBlock(in text: String) -> Bool { delegationBlockRange(in: text) != nil }

    /// The same test over a tail of a reply still streaming: a block is there once its
    /// opening fence is, so a reader that has scanned the text up to some point only
    /// needs to scan what arrived since (see `CrewButton`).
    static func hasDelegationOpener(in tail: Substring) -> Bool {
        tail.firstMatch(of: #/```[ \t]*crew(?:-sent|-leaving)?[ \t]*\r?\n/#.ignoresCase()) != nil
    }

    /// The delegation block at the end of a reply, if the lead wrote one. The body up to
    /// the last fence is tried first; when that does not parse, a prompt has most likely
    /// quoted a fence of its own and the closer is somewhere else, so every earlier fence
    /// is tried in turn, from the last back to the first, and lastly the end of the text
    /// for a block whose closer never came. An empty array is a block that asks for no
    /// hands (the lead did the work itself) and comes back as an empty list, not as nil.
    /// While the reply streams, `streamedDelegations(in:)` reads the same block entry by entry.
    static func delegations(in text: String) -> [CrewDelegation]? {
        guard let opener = delegationOpener(in: text) else { return nil }
        let bodyStart = opener.upperBound
        var ends = lineStartFences(in: text, from: bodyStart).reversed().map(\.lowerBound)
        ends.append(text.endIndex)
        for end in ends {
            if let parsed = delegations(fromBody: String(text[bodyStart..<end])) { return parsed }
        }
        return nil
    }

    /// The hands a block body asks for: a JSON array of entries (or one bare entry),
    /// read by `delegation(fromEntry:)`, which holds the rules for which entries are kept
    /// and how they are titled and named.
    /// An empty array is read as asking for nothing; entries with nothing usable in them
    /// are not read at all.
    private static func delegations(fromBody body: String) -> [CrewDelegation]? {
        guard let json = JSONValue.parse(body) else { return nil }
        if let array = json.array, array.isEmpty { return [] }
        let entries: [JSONValue] = json.array ?? (json.object == nil ? [] : [json])
        let parsed = entries.compactMap(delegation(fromEntry:))
        return parsed.isEmpty ? nil : parsed
    }

    /// The reply with its delegation block taken out, for what the lead said to the user.
    static func withoutDelegationBlock(_ text: String) -> String {
        var kept = text
        if let range = delegationBlockRange(in: text) { kept.removeSubrange(range) }
        return kept.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// How long the delegation card stays once its hands are out, wearing its done wave,
    /// before it leaves the reply.
    static let delegationSentHold: Duration = .seconds(3.2)

    /// How long the card takes to fade out in place, still holding its room in the reply,
    /// before the block goes and the text below closes up (see `markingDelegationBlockLeaving`).
    static let delegationLeaveFade: Duration = .milliseconds(450)

    /// Whether the reply holds a delegation block already marked sent or on its way out
    /// (see `markingDelegationBlockSent` and `markingDelegationBlockLeaving`).
    static func hasSentDelegationBlock(in text: String) -> Bool {
        text.contains(#/```[ \t]*crew-(?:sent|leaving)[ \t]*\r?\n/#.ignoresCase())
    }

    /// Whether the reply's delegation block is fading out (its info string is `crew-leaving`).
    static func hasLeavingDelegationBlock(in text: String) -> Bool {
        text.contains(#/```[ \t]*crew-leaving[ \t]*\r?\n/#.ignoresCase())
    }

    /// The reply with its sent block's info string turned into `crew-leaving`: the card fades
    /// where it stands, still taking up its room, so nothing below slides up under it. A
    /// removal that glided the card over the closing text read as a ghost of the card.
    static func markingDelegationBlockLeaving(_ text: String) -> String {
        guard hasSentDelegationBlock(in: text), !hasLeavingDelegationBlock(in: text), let opener = delegationOpener(in: text) else { return text }
        return text.replacingCharacters(in: opener, with: "```crew-leaving\n")
    }

    /// The reply with its delegation block's info string turned from `crew` into `crew-sent`,
    /// so the timeline draws the card in its done state for a moment before the block goes.
    static func markingDelegationBlockSent(_ text: String) -> String {
        guard let opener = delegationOpener(in: text), !hasSentDelegationBlock(in: text) else { return text }
        return text.replacingCharacters(in: opener, with: "```crew-sent\n")
    }

    /// What the lead hears when its block was there but could not be read: no hands went
    /// out, how to write the block so it can be read, and that the attempt cost it nothing.
    static func unreadableBlockMessage(reason: String?) -> String {
        var message = "[Crew] Your delegation block could not be read, so no hands went out."
        if let reason = reason?.trimmingCharacters(in: .whitespacesAndNewlines), !reason.isEmpty {
            message += " " + reason + (reason.last.map { ".!?".contains($0) } == true ? "" : ".")
        }
        return message + " Write the block again at the end of your reply: one fenced block whose info string is crew, holding a JSON array of objects with a task string, a prompt string and a files array. Inside a prompt never open a fenced code block of your own (no three backticks): describe code in words, quote identifiers with single backticks, or indent snippets. This does not count as a round of hands."
    }

    /// The commands that run a check, as a lead writes them in a brief.
    private static let checkCommands = #"xcodebuild|xcrun\s+xctest|swift\s+(?:build|test)|swiftlint|(?:npm|pnpm|yarn|bun)\s+(?:run\s+)?(?:test|build|lint|typecheck|type-check)|npx\s+(?:tsc|eslint|jest|vitest)|cargo\s+(?:build|test|check|clippy)|go\s+(?:test|build|vet)|pytest|gradlew?|mvn|make\s+(?:test|check)|tsc|eslint|jest|vitest|mypy|ruff|flake8|rspec|phpunit|dotnet\s+(?:build|test)"#

    /// A lead has been told never to put a check in a brief, but some do anyway: the
    /// lines that ask a hand to run one are taken out before the hand sees them. Only a
    /// line that asks the hand to run something counts: an order to run a check command,
    /// the build, the tests or the linter; an order to make sure it still builds or the
    /// tests pass; a check command on a line of its own. A line that only names a tool is
    /// an edit and stays: change `build.gradle`, move the `tsc` output folder, fix the
    /// `pytest` line in a script. Matching a bare tool name anywhere in a line (what this
    /// used to do) took such edits out of the brief without a word to anyone.
    private static let checkStepPatterns: [NSRegularExpression] = {
        let commands: String = CrewPrompts.checkCommands
        let run: String = #"\b(?:run|re-?run|execute|invoke|kick\s+off|trigger)\s+(?:(?:the|a|an|all|every|your|full|whole|entire|complete|existing|relevant|affected|unit|ui|integration|e2e|xcode|project|app|package|new|same)\s+)*"#
        let verify: String = #"\b(?:verify|confirm|ensure|make\s+sure|check|prove|validate|double-check)\s+(?:that\s+)?"#
        // "run the test fixtures", "check the build settings": the word is a noun here.
        let notAThing: String = #"(?!\s*(?:fixtures?|helpers?|data|files?|folders?|directory|numbers?|names?|ids?|plans?|settings?|config\w*|flags?|options?|doubles?|harness|utilit\w*|target\s+names?|coverage)\b)"#
        let patterns: [String] = [
            // Run a check command, or the build, the tests, the linter.
            run + #"[`'"$]*\s*(?:\#(commands))\b"#,
            run + #"(?:builds?|tests?|test\s+suites?|suites?|specs?|linters?|lint|type[- ]?checks?|type[- ]?checkers?|checks|compilers?|ci)\b"# + notAThing,
            // Make sure it still builds, the tests pass, nothing else broke.
            verify + #"(?:it|everything|this|the\s+(?:project|app|code|target|package|module|workspace|scheme|change|changes|fix))\s+(?:still\s+)?(?:builds|compiles|type-?checks)\b(?!\s+(?:the|a|an|its|their|this|that|to|into|from|as)\b)"#,
            verify + #"(?:everything|the\s+(?:build|tests|test\s+suite|suite|checks)|all\s+(?:the\s+)?(?:tests|checks))\s+(?:still\s+)?(?:pass|passes|succeeds?|is\s+green|are\s+green)\b"#,
            verify + #"nothing(?:\s+else)?\s+(?:broke|breaks|is\s+broken)\b"#,
            // Verify the build; confirm with a check command; build to verify.
            #"\b(?:verify|confirm|validate|prove|check)\s+(?:the\s+)?(?:build|tests|test\s+suite)\b"# + notAThing + #"(?!\s+(?:cover|covers|for)\b)"#,
            #"\b(?:verify|confirm|check|prove|validate)\s+(?:it\s+|this\s+|that\s+)?(?:with|using|by\s+running|via)\s+[`'"$]*\s*(?:\#(commands))\b"#,
            #"\bbuild\s+(?:it\s+|the\s+(?:project|app|target|scheme)\s+)?(?:and|to)\s+(?:verify|confirm|prove|check|make\s+sure|ensure)\b"#,
            // A result to hand back, or acceptance written as a check to pass.
            #"\bbuild\s+succeeded\b|\bpass(?:ing)?\s+reports?\b"#,
            #"(?:\#(commands))`?\s+(?:passes|succeeds|is\s+green|must\s+pass|should\s+pass)\b"#,
            #"\b(?:the\s+)?(?:build|tests|test\s+suite)\s+(?:must|should)\s+(?:pass|succeed|be\s+green)\b"#,
            // A check command alone on its line, in backticks or after a prompt sign.
            #"^\s*(?:[-*•]\s+|\d+[.)]\s+)?(?:\$\s*(?:\#(commands))\b.*|`\s*\$?\s*(?:\#(commands))\b[^`]*`\s*[.;:]?)\s*$"#,
        ]
        // A pattern that does not compile is left out rather than trapping: the brief then
        // goes out with that kind of line in it, and the hand's own rules still hold.
        return patterns.compactMap { try? NSRegularExpression(pattern: $0, options: [.caseInsensitive]) }
    }()

    /// A line that tells the hand not to run a check ("do not run the tests", "never
    /// build in your copy") asks for none: it stays.
    private static let negatedCheckPattern = try? NSRegularExpression(
        pattern: #"\b(?:don['’]t|do\s+not|never|no\s+need\s+to|without|not)\s+(?:[\w'’-]+\s+){0,2}?(?:run|re-?run|execute|invoke|build|test|verify|compile|lint|check)"#,
        options: [.caseInsensitive]
    )

    /// Whether one line of a brief asks the hand to run a check (see `checkStepPatterns`).
    private static func isCheckStep(_ line: String) -> Bool {
        let range = NSRange(line.startIndex..., in: line)
        if negatedCheckPattern?.firstMatch(in: line, range: range) != nil { return false }
        return checkStepPatterns.contains { $0.firstMatch(in: line, range: range) != nil }
    }

    /// A brief split in two: the lines the hand is sent, and the lines taken out because
    /// they ask it to run a check. A brief with no such line comes back whole. The lines
    /// taken out are never dropped in silence: the hand's brief quotes them (see
    /// `removedChecksNote`) and the lead's chat names them.
    static func verificationSteps(in prompt: String) -> (kept: String, removed: [String]) {
        var kept: [Substring] = []
        var removed: [String] = []
        for line in prompt.split(separator: "\n", omittingEmptySubsequences: false) {
            if isCheckStep(String(line)) {
                removed.append(line.trimmingCharacters(in: .whitespaces))
            } else {
                kept.append(line)
            }
        }
        guard !removed.isEmpty else { return (prompt, []) }
        return (kept.joined(separator: "\n"), removed)
    }

    /// The check lines `verificationSteps(in:)` took out of a brief, told to the hand
    /// itself: which they were, and that an edit one of them also names is still its to
    /// make, so a line that held an edit as well as a check never loses the edit.
    private static func removedChecksNote(_ removed: [String]) -> String {
        guard !removed.isEmpty else { return "" }
        let one = removed.count == 1
        let lines = removed.map { "- \(quoted($0, limit: 300))" }.joined(separator: "\n")
        return """


            ## Checks left to the lead
            Bottega took \(one ? "this line" : "these lines") out of your brief because \(one ? "it asks" : "they ask") you to run a check, and the lead runs every check itself:
            \(lines)
            Make any edit \(one ? "it names" : "they name"), run none of the checks, and say under Verification in your report that they are left to the lead.
            """
    }

    /// What a hosted hand does before its first edit: the user's start-of-work rules (read
    /// the coordination state, check for duplicates, claim, check the current state before
    /// acting) as they fall to one hand of a team. The lead has read the notes and any row
    /// is its own, so the hand checks the code rather than re-reading the notes, and adds
    /// no row of its own: every hand re-reading the same files is spent budget, and several
    /// hands editing one TASKS file at once collide. `delegated` is a task the lead sent
    /// out; otherwise the user queued it and nobody claimed it.
    private static func beforeYouStart(delegated: Bool) -> String {
        let coordination = delegated
            ? "The lead read the project's coordination state before sending you out, and any row for this task is its to claim and close: do not claim the task again or re-read the project's notes for it; what you need from them is in this brief."
            : "Nobody claimed a coordination row for this task: do not claim one yourself; the lead records your work from the TASKS row in your report."
        return """
        1. \(coordination)
        2. Check that the code is as the task describes before you change it: find each symbol or line it names in the file as it is now (the read your edit needs is enough); a line number or a plan written earlier may be out of date.
        3. If the code shows the task already done, or unfinished work that is not yours in those lines, stop and say so in your report rather than redoing or overwriting it.
        4. Leave the coordination files (TASKS, logs, INDEX) to the lead: it closes your row from your report. Edit one only when your brief asks you to, and then only your own row, with a targeted edit of the file at the absolute path the brief gives: never a copy of it in your own checkout, and never a rewrite of the whole file.
        """
    }

    /// The main chat in a few lines, for a hand that has none of the lead's context: what
    /// the user last asked, what the lead last said (its delegation block left out: that
    /// holds the other hands' briefs), and, when `touched` names them, the files the lead
    /// has been changing, which the hand leaves alone.
    private static func chatLines(_ context: ChatContext, reply: String, touched: String?) -> [String] {
        var lines: [String] = []
        if let prompt = context.lastUserPrompt, !prompt.isEmpty {
            lines.append("- The user last asked the lead: \(quoted(prompt, limit: 1_200))")
        }
        if let said = context.lastReply.map({ withoutDelegationBlock($0) }), !said.isEmpty {
            lines.append("- \(reply): \(quoted(said, limit: 800))")
        }
        if let touched, !context.touchedPaths.isEmpty {
            let paths = context.touchedPaths.prefix(20).joined(separator: ", ")
            lines.append("- \(touched): \(paths). Leave these alone; if your task cannot be done without touching one, keep the change minimal and say so in your report.")
        }
        return lines
    }

    /// What a hosted hand is sent for a task the lead delegated. `context` is the main
    /// chat as it stands (see `AppModel.crewChatContext`): a delegated hand has none of the
    /// lead's own context, and without it knew only its brief. `stripsVerificationSteps`
    /// takes the brief's check lines out (see `verificationSteps(in:)`) and tells the hand
    /// which; off, the brief goes out whole and the hand may run the check it names.
    static func delegatedHandPrompt(persona: CrewPersona, delegation: CrewDelegation, workplace: Workplace, context: ChatContext? = nil, stripsVerificationSteps: Bool = true) -> String {
        let brief = stripsVerificationSteps ? verificationSteps(in: delegation.prompt) : (kept: delegation.prompt, removed: [String]())
        // The claimed files and the coordination row, when the lead gave them. Appended
        // after the check lines are taken out rather than inside the prompt, so the line
        // filter can never take them out.
        var scope = ""
        if !delegation.files.isEmpty {
            scope += """


                ## The files that are yours
                \(delegation.files.map { "- \($0)" }.joined(separator: "\n"))

                The lead claimed these for you. Other hands are in other files at the same moment and none of you sees the others' work, so change nothing outside this list unless the task is impossible otherwise — and mark such a file "(outside claim)" in your report when you do.
                """
        }
        if let claim = delegation.claimID, !claim.isEmpty {
            scope += """


                ## Your claim
                This task is claimed as \(claim) in the project's coordination file. Name it in your report's TASKS row so the lead can close that row.
                """
        }
        // The main chat in a few lines, after the task: what the user asked for and what
        // the lead said when it sent the hand out.
        let chat = context.map { chatLines($0, reply: "The lead's reply that sent you out", touched: "Files the lead has been changing") } ?? []
        let chatBlock = chat.isEmpty ? "" : "\n\n## What is going on in the main chat\n" + chat.joined(separator: "\n")
        return """
        You are \(persona.name), a Crew hand in Bottega: one of several helpers working in parallel for a lead agent. The lead delegated this task to you.

        ## Your task: \(delegation.task)
        \(brief.kept)\(removedChecksNote(brief.removed))\(scope)\(chatBlock)

        ## Before you start
        \(beforeYouStart(delegated: true))

        ## Where you work
        \(workplaceRules(workplace, stripsChecks: stripsVerificationSteps))

        ## How to work
        \(howToWork(stripsChecks: stripsVerificationSteps))

        ## Your report
        \(howToReport)
        """
    }

    /// What the lead's chat looked like when a task was handed to a hand, queued or
    /// delegated, so the hand knows what is going on without the lead's context. Built
    /// from the timeline, not from another model call, so it costs nothing extra.
    struct ChatContext {
        var lastUserPrompt: String?
        var lastReply: String?
        var touchedPaths: [String] = []
    }

    /// What a hosted hand is sent for a task the user queued while the lead worked: the
    /// lead is busy, so the hand is told what it is busy with. The user's own words go out
    /// whole, check lines and all: `stripsVerificationSteps` only decides what the hand's
    /// rules say about checks.
    static func queuedHandPrompt(persona: CrewPersona, task: String, context: ChatContext, workplace: Workplace, stripsVerificationSteps: Bool = true) -> String {
        let lines = chatLines(
            context,
            reply: "The lead's latest reply so far",
            touched: "Files the lead is changing in its current turn"
        )
        let contextBlock = lines.isEmpty ? "The lead has not said anything yet." : lines.joined(separator: "\n")
        return """
        You are \(persona.name), a Crew hand in Bottega: a helper running in parallel with the lead agent. The user queued this task for you while the lead works on something else.

        ## What is going on in the main chat
        \(contextBlock)

        ## Your task
        \(task)

        ## Before you start
        \(beforeYouStart(delegated: false))

        ## Where you work
        \(workplaceRules(workplace, stripsChecks: stripsVerificationSteps))

        ## How to work
        \(howToWork(stripsChecks: stripsVerificationSteps))

        ## Your report
        \(howToReport)
        """
    }

    /// The message the lead receives once hands have reported: one section per hand, what
    /// landed where, and what to do about it. `stillWorking` names the hands yet to report;
    /// `reviewsHands` has the lead audit the files listed before it finishes.
    /// Where the one verification stands after a report message: nothing to verify yet,
    /// held until the last hand is back, or due now.
    enum VerificationStep {
        case none, hold, runNow
    }

    /// The one verification, review or not: once for the whole change, never hand by hand
    /// (the user's rule: acceptance is done once, after all the code is in), so with hands
    /// still out it waits for the last report. A batch that changed nothing (a scout, a hand
    /// whose copy came back clean) has nothing to build of its own, but a hold told to the
    /// lead by an earlier report (`owed`) still ends here: the last hand back says run it,
    /// whatever that hand changed. A hand without a copy of its own worked in the checkout
    /// itself, and may have changed anything.
    static func verificationStep(_ reports: [CrewReport], stillWorking: [String], owed: Bool) -> VerificationStep {
        let changed = reports.contains { $0.landing.map { !$0.isEmpty } ?? ($0.copyPath == nil) }
        guard changed || owed else { return .none }
        return stillWorking.isEmpty ? .runNow : .hold
    }

    static func reportMessage(_ reports: [CrewReport], stillWorking: [String] = [], reviewsHands: Bool = false, verificationOwed: Bool = false) -> String {
        let names = reports.map { CrewRoster.persona(at: $0.handIndex).name }
        var opening = "Crew reports: \(list(names)) finished."
        if !stillWorking.isEmpty {
            opening += " \(list(stillWorking)) \(stillWorking.count == 1 ? "is" : "are") still at work; \(stillWorking.count == 1 ? "its report comes" : "their reports come") as a later message."
        }

        var sections: [String] = []
        for report in reports {
            let persona = CrewRoster.persona(at: report.handIndex)
            let outcome = switch report.status {
            case .completed: ""
            case .failed: " (failed)"
            case .stopped: " (stopped by the user before finishing)"
            case .running: ""
            }
            var effort: [String] = []
            if let elapsed = report.elapsed, elapsed >= 1 { effort.append(duration(elapsed)) }
            if report.toolCalls > 0 { effort.append(report.toolCalls == 1 ? "1 tool" : "\(report.toolCalls) tools") }
            let took = effort.isEmpty ? "" : " (" + effort.joined(separator: ", ") + ")"
            var lines = ["## \(persona.name): \(report.task)\(outcome)\(took)"]
            if let landing = landingLine(for: report) { lines.append(landing) }
            // Where the whole report was kept on disk, right under the landing: the section
            // header stays as it is, since `CrewReportDigest` reads the hands off it.
            if let path = report.reportPath { lines.append("\(reportFilePrefix) \(path)") }
            let body = report.text.trimmingCharacters(in: .whitespacesAndNewlines)
            lines.append(body.isEmpty ? "No report." : body)
            sections.append(lines.joined(separator: "\n"))
        }

        var closing: [String] = []
        if reports.contains(where: { $0.landing?.landed == true }) {
            closing.append("The changes listed above are in your checkout already; nothing needs applying.")
        }
        if reports.contains(where: { $0.copyPath == nil && $0.status == .completed }) {
            closing.append("The hands worked in your checkout, so their changes are there already.")
        }
        if reports.contains(where: { !($0.landing?.conflicts.isEmpty ?? true) }) {
            closing.append("A file listed with conflicts was merged three-way and keeps conflict markers: resolve those first, and tell the user in one line that you settled them, so the landing card above does not read as broken.")
        }
        // Two hands on one file is the split the lead was told not to make; it hears
        // which file, so it reads the result instead of trusting it.
        var owners: [String: [String]] = [:]
        for report in reports {
            for file in report.landing?.files ?? [] {
                owners[file.path, default: []].append(CrewRoster.persona(at: report.handIndex).name)
            }
        }
        for (path, names) in owners.sorted(by: { $0.key < $1.key }) where names.count > 1 {
            closing.append("\(list(names)) both changed \(path): two hands on one file is the split you were told not to make. Treat the hand that finished first as the one that stands, read the file as it is now, and redo the other's part yourself as an incremental edit — do not trust either report on that file, and record the correction where the work was claimed.")
        }
        for report in reports {
            guard let landing = report.landing, let path = landing.patchPath else { continue }
            closing.append("\(CrewRoster.persona(at: report.handIndex).name)'s changes would not apply on their own; the patch is at \(path). Apply it with `git apply --3way \(path)` and settle what conflicts.")
        }
        // A hand Bottega would not start in a plan chat (`CrewHandAuthority.planRefusal`)
        // is not one to send out again, so it gets its own line.
        func refusedInPlan(_ report: CrewReport) -> Bool {
            report.status == .failed && report.text.contains(CrewHandAuthority.refusalPrefix)
        }
        if reports.contains(where: { $0.status == .failed && !refusedInPlan($0) }) {
            closing.append("A hand that failed leaves its part to you: do it yourself or send it out again.")
        }
        if reports.contains(where: refusedInPlan) {
            closing.append("A hand that was not started because this chat is in plan mode leaves its part to you: plan it yourself, and send nothing out again until the user leaves plan mode.")
        }
        if reports.contains(where: { $0.status == .stopped }) {
            closing.append("A hand the user stopped leaves its part alone unless the user asks for it.")
        }
        if reports.contains(where: { $0.origin == .queued }) {
            closing.append("The user queued that work for the hands while you were busy; take it into account and carry on with the main job.")
        }
        // With the check on, the lead reads every file listed rather than only the ones
        // that matter, and fixes what it finds itself.
        if reviewsHands {
            closing.append("Do not check any of this with git status or git diff: the checkout changes under you while hands work, and the user may be editing too. Do not reconcile or revert anything. " + reviewRule)
        } else {
            closing.append("Do not check any of this with git status or git diff: the checkout changes under you while hands work, and the user may be editing too. Do not reconcile, revert or redo anything. Build on the reports, read the files they name if something matters, and finish the job.")
        }
        // The one verification (see `verificationStep`).
        switch verificationStep(reports, stillWorking: stillWorking, owed: verificationOwed) {
        case .none:
            break
        case .runNow:
            closing.append("Every hand is back: run the one verification now, the narrowest build or type-check that proves the whole change, in your checkout, for all of the work together rather than hand by hand, and report its pass or fail line as it is.")
        case .hold:
            closing.append("Hold the verification until the last hand is back: it runs once, for all of the work together, never hand by hand.")
        }
        closing.append(closeoutRule)
        closing.append(languageRule)
        closing.append(reportStyleRule)
        if !stillWorking.isEmpty {
            closing.append("Do not wait for \(list(stillWorking)) and do not take over \(stillWorking.count == 1 ? "its" : "their") tasks; tell the user \(stillWorking.count == 1 ? "it is" : "they are") still at work and that you will hear from \(stillWorking.count == 1 ? "it" : "them").")
        }

        return """
        \(opening)

        \(sections.joined(separator: "\n\n"))

        \(closing.joined(separator: " "))
        """
    }

    /// One line on where a hand's work went, for its section of the report. A hand that
    /// worked in the checkout itself lists the files its tools changed there as landed,
    /// and says nothing when none of its tools landed an edit: whatever it changed is simply there.
    private static func landingLine(for report: CrewReport) -> String? {
        guard let landing = report.landing else {
            guard let path = report.copyPath else { return nil }
            return report.status == .completed ? "Changed nothing." : "Nothing landed; its unfinished work is in its copy at \(path)."
        }
        if landing.isEmpty { return "Changed nothing." }
        let files = landing.files.prefix(25).map { file -> String in
            var line = "\(file.path) (+\(file.additions) −\(file.deletions))"
            if landing.conflicts.contains(file.path) { line += " with conflicts" }
            return line
        }
        let listed = files.joined(separator: ", ")
            + (landing.files.count > 25 ? ", and \(landing.files.count - 25) more" : "")
        let omitted = landing.droppedBuildOutputFiles == 0 ? "" : "; \(landing.droppedBuildOutputFiles) build-output file\(landing.droppedBuildOutputFiles == 1 ? " was" : "s were") left out"
        if landing.patchPath != nil { return "Did not land: \(listed)\(omitted)." }
        if let error = landing.error { return "Did not land (\(error)): \(listed)\(omitted)." }
        return "Landed in your checkout: \(listed)\(omitted)."
    }

    private static func duration(_ seconds: TimeInterval) -> String {
        let total = Int(seconds)
        return total < 60 ? "\(total)s" : "\(total / 60)m \(total % 60)s"
    }

    private static func list(_ names: [String]) -> String {
        switch names.count {
        case 0: ""
        case 1: names[0]
        default: names.dropLast().joined(separator: ", ") + " and " + names[names.count - 1]
        }
    }

    private static func quoted(_ text: String, limit: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let cut = trimmed.count > limit ? String(trimmed.prefix(limit)) + "…" : trimmed
        return "“\(cut)”"
    }
}
