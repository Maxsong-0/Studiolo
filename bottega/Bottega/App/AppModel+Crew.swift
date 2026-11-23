// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

// Crew's hands are threads: each one has a timeline of its own, sits in its lead's
// floating panel while it works, and drops under the lead in the sidebar once dismissed,
// like any helper. Native hands run inside the lead's provider session and their
// timelines fill from the session's events; hosted hands have sessions of their own,
// each in a copy of the checkout made for it, and their work lands in the lead's checkout
// the moment they report.

/// What a hand changed in its copy since the tree it started from: the patch itself, the
/// files it touches, and the tree the copy holds now, which becomes the hand's new base
/// once the patch lands.
private struct CrewPatch: Sendable {
    var text: String
    var files: [CrewLanding.File]
    var droppedBuildOutputFiles: Int
    var after: String
}

/// One capture of a checkout's working tree, shared by the hands sent out together.
/// Capturing means `git add -A` and `write-tree` over the whole project, seconds on a big
/// repository, and the hands of one delegation block ask for it within milliseconds of
/// each other: without this each hand would pay for its own and the last one would start
/// half a minute after the first. A capture stands for a few seconds only, and a landing
/// into the checkout throws it away, so no hand ever starts from a stale tree.
@MainActor
private enum CrewTreeCache {
    private struct Capture {
        var tree: String
        var commit: String
        var capturedAt: Date
    }

    /// How long a capture stands in for the checkout: long enough for one batch to go out
    /// on it, short enough that the next hand sees the checkout as it is.
    private static let lifetime: TimeInterval = 3

    private static var captures: [String: Capture] = [:]
    /// Captures under way, so hands asking at the same moment wait for the one running
    /// rather than each starting another.
    private static var inFlight: [String: Task<(tree: String, commit: String), any Error>] = [:]

    static func capture(of checkout: String, with git: Git) async throws -> (tree: String, commit: String) {
        if let ready = captures[checkout], Date.now.timeIntervalSince(ready.capturedAt) < lifetime {
            return (ready.tree, ready.commit)
        }
        if let running = inFlight[checkout] { return try await running.value }
        let task = Task { () async throws -> (tree: String, commit: String) in
            let tree = try await git.captureTree()
            // The commit is the hands' starting point and no branch ever sees it, so it
            // carries no hand's name: several hands share this one.
            let commit = try await git.commitTree(tree, message: "Bottega: a Crew hand's starting point")
            return (tree, commit)
        }
        inFlight[checkout] = task
        defer { inFlight[checkout] = nil }
        let made = try await task.value
        captures[checkout] = Capture(tree: made.tree, commit: made.commit, capturedAt: .now)
        return made
    }

    /// The checkout changed under the capture: the next hand captures it afresh.
    static func invalidate(_ checkout: String) {
        captures[checkout] = nil
    }
}

/// What the watchdog over the hosted hands remembers between looks (see
/// `AppModel.startCrewWatchdog`): its one loop, each hand's tool count at the last
/// look, the hands it has labelled as thinking, and the hands it stopped, whose reports
/// open with why.
@MainActor
private enum CrewWatchdog {
    /// How often it looks. How long a hand is quiet before it is called thinking, then
    /// stalled, is the user's (`CrewLimits.watchdogThinkingAfter`, `.watchdogStalledAfter`).
    static let tick: Duration = .seconds(30)

    static var loop: Task<Void, Never>?
    static var toolCounts: [UUID: Int] = [:]
    static var labelled: Set<UUID> = []
    /// The hands it stopped, each with how long it had to be quiet to be stopped then and
    /// what the quiet was.
    static var stalledHands: [UUID: (after: TimeInterval, reason: StallReason)] = [:]

    /// What a hand quiet this long, with no new tool, was doing.
    enum StallReason {
        /// Waiting on an approval or a question that nobody answered.
        case unanswered
        /// Nothing its tools did this turn wrote a file.
        case wroteNothing
    }

    /// Whether a hand that has been quiet past the stall limit, with no tool since the last
    /// look, has stalled, and why; nil when it has only gone quiet to think. A hand waiting
    /// on a person has stalled however much it wrote before: nothing it does next can
    /// begin until someone answers. One that waits on nobody has stalled only if it has
    /// written nothing all turn, since a long think after real work is not a stall.
    static func stallReason(awaitsPerson: Bool, landedEdits: Int) -> StallReason? {
        if awaitsPerson { return .unanswered }
        return landedEdits == 0 ? .wroteNothing : nil
    }

    /// What `held` reads for a hand that ran in plan mode, where the way out differs from a
    /// hand held by its permissions.
    static let planMode = "plan mode"

    /// What the lead can do about a hand that what the chat allows held back.
    private static func remedy(held: String) -> String {
        held == planMode
            ? "Do the work yourself, or leave plan mode to send out hands that edit."
            : "Do the work yourself, or ask the user to raise the chat's permissions."
    }

    /// `held` names the permissions a hand under less than full access ran in: it may have
    /// sat on an approval nobody answers, and the same brief would stall the same way.
    static func stalledNote(stalledAfter: TimeInterval, reason: StallReason, held: String? = nil) -> String {
        let minutes = max(1, Int((stalledAfter / 60).rounded()))
        let span = minutes == 1 ? "1 minute" : "\(minutes) minutes"
        switch reason {
        case .unanswered:
            let quiet = "Stalled: waiting \(span) on an approval or a question nobody answered; stopped by Bottega."
            guard let held else { return quiet + " Resend with a brief that needs no answer." }
            return quiet + " The hand ran under \(held): resending the same brief stalls the same way. \(remedy(held: held))"
        case .wroteNothing:
            let quiet = "Stalled: no edits for \(span); stopped by Bottega."
            guard let held else { return quiet + " Resend with a sharper brief." }
            return quiet + " The hand ran under \(held), so it may have been waiting on an approval nobody answers: resending the same brief stalls the same way. \(remedy(held: held))"
        }
    }
}

/// The hands whose copies are being released right now (see `AppModel.releaseCrewCopy`):
/// the patch is written before the copy goes, and a second release started meanwhile
/// would remove the folder the first is still reading.
@MainActor
private enum CrewCopyRelease {
    static var inFlight: Set<UUID> = []
}

extension AppModel {
    /// Whether a chat leads a team right now: Crew on for the app (the master) and on
    /// for this chat's own switch. Helpers lead no team of their own.
    func crewIsOn(_ thread: ChatThread) -> Bool {
        settings.crewEnabled && thread.crewEnabled && !thread.isHelper
    }

    /// Whether the provider has hands of its own: it runs them inside its session, with
    /// the pair's model and effort, and keeps the lead's Crew policy in its system prompt.
    /// Every other provider gets hosted hands and the delegation block. Whether a given
    /// chat's hands actually run natively is `CrewLaunch.runsNatively`: a pair that sends
    /// the hands out on another provider makes them hosted here too.
    static func crewIsNative(_ provider: ProviderKind) -> Bool {
        provider == .claude || provider == .codex || provider == .copilot
    }

    /// The pair a chat leads with: the one picked for it in the composer's model picker,
    /// for as long as the chat stays on that pair's provider. A chat with none leads its
    /// hands on its own model and effort; no pair applies to a chat on its own.
    func crewPair(for thread: ChatThread) -> CrewPair? {
        guard let pair = settings.crewPair(thread.crewPairID), pair.provider == thread.provider else { return nil }
        return pair
    }

    /// The provider a pair's hands go out on, as far as this Mac can run it: a hands'
    /// provider that is not installed or is switched off would only fail every hand, so the
    /// hands stay on the lead's provider until it is.
    func crewHandsProvider(of pair: CrewPair) -> ProviderKind {
        guard let provider = pair.workerProvider, provider != pair.provider,
              crewProviderUnavailableReason(provider) == nil else { return pair.provider }
        return provider
    }

    /// Why hands cannot go out on a provider, as the end of a sentence ("… is switched off
    /// in Settings"), or nil when they can. Switched off is asked first: a provider that is
    /// off was never checked (see `ProviderRegistry.refreshAll`), so it has no install
    /// status to go by. One reading for every note that names the reason.
    func crewProviderUnavailableReason(_ provider: ProviderKind) -> String? {
        guard settings.isEnabled(provider) else { return "is switched off in Settings" }
        return providers.countsAsInstalled(provider) ? nil : "is not installed on this Mac"
    }

    /// Why a pair's hands are not on the provider the pair names, in a sentence for the
    /// lead's timeline, or nil when they are. Read when hands go out, so the user hears
    /// once per job rather than never (the fallback used to be silent).
    func crewHandsFallbackNote(of pair: CrewPair) -> String? {
        guard let provider = pair.workerProvider, provider != pair.provider,
              let reason = crewProviderUnavailableReason(provider) else { return nil }
        return "The pair's hands run on \(pair.provider.displayName) for now: \(provider.displayName) \(reason)."
    }

    /// The effort hands inherit from a lead when the pair leaves it open: a working
    /// effort when the lead thinks above it, since a hand's brief is a bounded task and
    /// the lead keeps the judgement; the lead's own effort at the working rung or below,
    /// or on a scale the lead's word is not on; nil when the lead has none. The working
    /// rung is a literal medium when the model's scale has one, else the scale's own
    /// middle rung — Z.ai's low/high/max works out at high, which its API counts as the
    /// medium effort (see `ZaiSession.reasoningEffort`).
    static func crewHandsEffort(leadEffort: String?, scale: [String]) -> String? {
        guard let leadEffort, !leadEffort.isEmpty else { return nil }
        guard let lead = scale.firstIndex(of: leadEffort) else { return leadEffort }
        let working = scale.firstIndex(of: "medium") ?? (scale.isEmpty ? nil : (scale.count - 1) / 2)
        guard let working else { return leadEffort }
        return lead > working ? scale[working] : leadEffort
    }

    /// What the hands run on while the chat leads, or nil with Crew off. Without a pair
    /// the hands inherit the chat's own model and effort, the effort as `crewHandsEffort`
    /// tempers it while that setting is on, under the Crew page's cap alone. The limits
    /// they run to are the Crew page's too, whatever the pair, and the user's own rules
    /// for leads ride along.
    func crewLaunch(for thread: ChatThread) -> CrewLaunch? {
        guard crewIsOn(thread) else { return nil }
        let refs = projects.map { CrewProjectRef(name: $0.name, path: $0.path) }
        let ownRefs = refs.filter { $0.path == project(thread.projectID)?.path } + refs.filter { $0.path != project(thread.projectID)?.path }
        let pair = crewPair(for: thread)
        let handsProvider = pair.map(crewHandsProvider) ?? thread.provider
        let elsewhere = handsProvider != thread.provider
        // A pair's hands' model and effort are its hands' provider's words: with the hands
        // kept on the lead's provider because that one cannot run here, they mean nothing,
        // and the hands inherit the chat's own model and effort instead.
        let keepsModel = pair.map { !$0.sendsHandsElsewhere || elsewhere } ?? true
        var label: String?
        if elsewhere {
            let model = providers.model(pair?.workerModel, for: handsProvider) ?? providers.defaultModel(for: handsProvider)
            label = "\(model?.shortName ?? pair?.workerModel ?? "the default model") on \(handsProvider.displayName)"
        }
        let pairEffort = keepsModel ? pair?.workerEffort : nil
        let workerEffort: String?
        if let pairEffort {
            workerEffort = pairEffort
        } else if settings.crewTempersHandEffort, !elsewhere {
            workerEffort = Self.crewHandsEffort(leadEffort: thread.effort, scale: providers.model(thread.model, for: thread.provider)?.efforts ?? [])
        } else {
            workerEffort = nil
        }
        // The profiles a lead can route to, named and told apart (`CrewHandProfile.usable`).
        // With the hands kept on the lead's provider because theirs cannot run here, a
        // profile that rides on the hands' provider names a model and effort that mean
        // nothing, as the pair's own do; only the ones with a provider of their own stay.
        let profiles = CrewHandProfile.usable(pair?.handProfiles ?? []).filter { keepsModel || $0.provider != nil }
        return CrewLaunch(
            handsProvider: handsProvider,
            runsNatively: !elsewhere && Self.crewIsNative(thread.provider),
            handsLabel: label,
            workerModel: keepsModel ? pair?.workerModel : nil,
            workerEffort: workerEffort,
            // The lower of the pair's cap and the Crew page's; with neither, the hands run uncapped.
            maxHands: [pair?.maxHands, settings.crewMaxHands].compactMap { $0 }.min(),
            limits: settings.crewLimits,
            handProfiles: profiles,
            isolatesHands: settings.crewIsolateHands,
            autoMerges: settings.crewAutoMerge,
            // Only the auto-merge reads the mode, so with it off the mode stays out of the
            // launch and changing it restarts no session.
            mergeMode: settings.crewAutoMerge ? settings.crewMergeMode : .localCommit,
            reviewsHands: settings.crewReviewHands,
            stripsVerificationSteps: settings.crewStripsVerificationSteps,
            holdsDelegations: settings.crewHoldsDelegations,
            projects: ownRefs,
            allowedProjectRoots: settings.crewAllowedProjectRoots,
            antiSlopEnabled: settings.antiSlopEnabled,
            leadInstructions: settings.crewLeadInstructionsText
        )
    }

    /// The per-chat Crew switch: off kills hands for that chat only, on restores them
    /// (with the app-wide switch on). The flip is this chat's alone: new threads start as
    /// "New chats lead a team" in Settings › Crew has it (`AppSettings.crewDefaultEnabled`),
    /// so switching one chat on when it needs a team never turns the default back on.
    func setCrew(_ on: Bool, for id: UUID) {
        guard let thread = thread(id) else { return }
        guard on else {
            updateThread(id) { $0.crewEnabled = false }
            return
        }
        let pair = settings.crewPair(for: thread.provider, model: thread.model)
        updateThread(id) {
            $0.crewEnabled = true
            $0.crewPairID = pair?.id
        }
        guard let pair else { return }
        if let lead = pair.orchestratorModel, lead != thread.model, providers.model(lead, for: thread.provider) != nil {
            updateThread(id) {
                $0.model = lead
                $0.effort = pair.orchestratorEffort ?? $0.effort
            }
        } else if let effort = pair.orchestratorEffort {
            updateThread(id) { $0.effort = effort }
        }
    }

    /// The hands a lead still shows in its panel, in the order they were sent out. Read
    /// through the parent index and the hands' own cells, so the chat showing them is left
    /// alone when any other thread changes.
    func crewHands(of parentID: UUID) -> [ChatThread] {
        panelHands(of: parentID)
    }

    /// How many of a lead's hosted hands are still at work.
    func runningHostedHands(of parentID: UUID) -> Int {
        crewHands(of: parentID).count { $0.crew?.kind == .hosted && $0.crew?.status == .running }
    }

    /// How many of a lead's hands are still at work, native and hosted alike. A native
    /// hand lives inside the lead's own session, so this is what says whether stopping the
    /// lead's turn would take hands down with it.
    func runningCrewHands(of parentID: UUID) -> Int {
        crewHands(of: parentID).count { $0.crew?.status == .running }
    }

    /// How many distinct files a merge could still send for this lead: the lead's
    /// own unmerged turns and every hand whose landing no merge has taken yet.
    /// Build output, paths under no sidebar project, and hands whose work was
    /// kept as a patch are left out. Feeds the merge-state note in front of
    /// the lead's messages, so it never says nothing changed while hands'
    /// files sit in the checkout.
    func crewUnmergedFileCount(of leadID: UUID) -> Int {
        guard let runtime = existingRuntime(for: leadID) else { return 0 }
        var paths = Set(runtime.crewUnmergedTurns.filter { $0.status != .running }.flatMap { $0.touchedPaths ?? [] })
        let lastMergedTurnStart = runtime.turns.last { $0.crewMerged }?.startedAt
        for hand in crewTeam(of: leadID) {
            guard let info = hand.crew, info.mergedAt == nil, !(info.landing?.patchPath != nil || info.landing?.error != nil) else { continue }
            if let lastMergedTurnStart, let finished = info.finishedAt, finished < lastMergedTurnStart { continue }
            for file in info.landing?.files ?? [] { paths.insert(file.path) }
        }
        paths = paths.filter { path in !TouchedPaths.isBuildOutput(path) && projects.contains { TouchedPaths.relative(path, root: $0.path) != nil } }
        return paths.count
    }

    /// Every hand a lead has sent out, in the panel or not, in the order they went.
    func crewTeam(of parentID: UUID) -> [ChatThread] {
        children(of: parentID)
            .filter { !$0.isArchived && $0.isCrewHand }
            .sorted { ($0.crew?.index ?? 0) < ($1.crew?.index ?? 0) }
    }

    /// The names of a lead's hosted hands still at work, other than `excluding`.
    func workingCrewHandNames(of parentID: UUID, excluding: Set<Int> = []) -> [String] {
        crewHands(of: parentID)
            .filter { $0.crew?.kind == .hosted && $0.crew?.status == .running && !excluding.contains($0.crew?.index ?? -1) }
            .compactMap { $0.crew?.persona.name }
    }

    /// The checkout a lead works in: its worktree, else the project folder.
    private func crewCheckout(of lead: ChatThread) -> String? {
        lead.worktreePath ?? project(lead.projectID)?.path
    }

    /// The project a delegation entry sends its hand to: the chat's own with no name, a
    /// sidebar project by name or by path (the path itself or one inside it), or a folder
    /// inside a git repository that lies in one of the folders the user opened to hands in
    /// Settings › Crew, which joins the sidebar. Nil when the name matches nothing, or
    /// names a repository outside those folders (none by default: hands then stay in the
    /// sidebar's projects).
    func crewProject(named reference: String?, for lead: ChatThread) -> Project? {
        guard let reference, !reference.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return project(lead.projectID) }
        let expanded = (reference as NSString).expandingTildeInPath
        if let named = projects.first(where: { $0.name.caseInsensitiveCompare(expanded) == .orderedSame }) { return named }
        let standard = (expanded as NSString).standardizingPath
        let inside = projects.filter {
            TouchedPaths.relative(expanded, root: $0.path) != nil
                || ($0.path as NSString).standardizingPath == standard
        }
        if let best = inside.max(by: { $0.path.count < $1.path.count }) { return best }
        guard let repository = crewRepositoryRoot(containing: expanded), crewAllowsRepository(at: repository) else { return nil }
        return addProject(at: URL(fileURLWithPath: repository, isDirectory: true))
    }

    /// The root of the git repository holding a folder on this Mac, found by walking up
    /// from it; nil for a relative path, one that is not a folder, or a folder in no
    /// repository.
    func crewRepositoryRoot(containing path: String) -> String? {
        guard path.hasPrefix("/") else { return nil }
        var isDir: ObjCBool = false
        guard FileManager.default.fileExists(atPath: path, isDirectory: &isDir), isDir.boolValue else { return nil }
        var dir = (path as NSString).standardizingPath
        while true {
            if FileManager.default.fileExists(atPath: dir + "/.git") { return dir }
            if dir == "/" { return nil }
            dir = (dir as NSString).deletingLastPathComponent
            if dir.isEmpty { dir = "/" }
        }
    }

    /// Whether a repository outside the sidebar may take a hand: the whole repository has
    /// to lie in a folder the user opened to hands (or be that folder), since the hand gets
    /// the whole checkout, not only the folder the lead named inside it.
    func crewAllowsRepository(at root: String) -> Bool {
        let repository = (root as NSString).standardizingPath
        return settings.crewAllowedProjectRoots.contains { allowed in
            let folder = ((allowed as NSString).expandingTildeInPath as NSString).standardizingPath
            return repository == folder || TouchedPaths.relative(repository, root: folder) != nil
        }
    }

    /// Why a delegation's `project` sent its hand nowhere (see `crewProject`), in words
    /// for the lead's timeline: a repository that is there but outside every folder open
    /// to hands is refused rather than missing, so the note says where to open it up.
    private func crewProjectRefusal(_ reference: String) -> String {
        let expanded = (reference as NSString).expandingTildeInPath
        if crewRepositoryRoot(containing: expanded) != nil {
            return "\(reference) is outside the sidebar, and no folder open to hands in Settings › Crew holds it"
        }
        var isDirectory: ObjCBool = false
        if expanded.hasPrefix("/"), FileManager.default.fileExists(atPath: expanded, isDirectory: &isDirectory), isDirectory.boolValue {
            return "\(reference) is in no git repository, so no hand can work there"
        }
        return "no project named \(reference) is in the sidebar or on this Mac"
    }

    /// The checkout a hand's copy is made from and its work lands in: the lead's own for
    /// a hand in the lead's project, else the hand's project folder.
    private func crewCheckout(of hand: ChatThread, lead: ChatThread) -> String? {
        hand.projectID == lead.projectID ? crewCheckout(of: lead) : project(hand.projectID)?.path
    }

    // MARK: - Sending hands out

    /// Sends out a hand that mirrors one the provider started inside the lead's session.
    /// Its timeline only rehearses what the session reports, and it works wherever the
    /// lead does.
    @discardableResult
    func spawnNativeHand(from parentID: UUID, spawn: AgentSpawn) -> ChatThread? {
        guard let hand = insertCrewHand(from: parentID, task: spawn.description, kind: .native, origin: .delegated, native: spawn, batchID: nil, profile: spawn.profile) else { return nil }
        // The brief, when the provider has said what it is; otherwise the timeline starts
        // on the working line and the brief slots in above once it arrives.
        runtime(for: hand.id).rehearseTurn(spawn.prompt)
        return hand
    }

    /// Sends out a hand with a session of its own, on the pair's model and effort. With
    /// isolation on and a git repository to copy, the hand first gets a worktree of its
    /// own, made as it is from the checkout of the project the delegation names (the
    /// lead's own when it names none); otherwise it works in that checkout itself.
    /// `brief` writes the prompt once it is known where the hand works. A `profile`
    /// naming one of the pair's hand profiles runs the hand on that profile's model,
    /// effort and provider instead of the pair's shared worker fields. `queuedAt` is when
    /// the task joined the queue of work waiting for a hand; nil for one handed out at once.
    @discardableResult
    func spawnHostedHand(
        from parentID: UUID,
        task: String,
        origin: CrewHandInfo.Origin,
        attachments: [Attachment] = [],
        batchID: UUID? = nil,
        preferredIndex: Int? = nil,
        project: String? = nil,
        profile: String? = nil,
        queuedAt: Date? = nil,
        brief: @escaping @Sendable (CrewPersona, CrewPrompts.Workplace) -> String
    ) -> ChatThread? {
        guard let hand = insertCrewHand(from: parentID, task: task, kind: .hosted, origin: origin, native: nil, batchID: batchID, preferredIndex: preferredIndex, project: project, profile: profile, queuedAt: queuedAt) else { return nil }
        Task { await startHostedHand(hand.id, attachments: attachments, brief: brief) }
        return hand
    }

    private func insertCrewHand(
        from parentID: UUID,
        task: String,
        kind: CrewHandInfo.Kind,
        origin: CrewHandInfo.Origin,
        native: AgentSpawn?,
        batchID: UUID?,
        preferredIndex: Int? = nil,
        project: String? = nil,
        profile: String? = nil,
        queuedAt: Date? = nil
    ) -> ChatThread? {
        guard let parent = thread(parentID) else { return nil }
        // A lead with no hands left starts the roster over. The count would otherwise
        // climb for the life of the chat, and a lead that has sent out twenty-five hands
        // over a morning would name its next one "Alfa 2" with one hand in the panel.
        let sequential = crewTeam(of: parentID).isEmpty ? 0 : parent.crewSpawnCount
        // A delegation that announced its hand by name is authoritative: the spawned hand
        // carries exactly the announced roster name, so "Sent Gus" never spawns Otto.
        // Two hands sharing one index would merge each other's reports, so a name a
        // finished hand of the team already carries goes to the next round of that name
        // ("Tova" is done, so this one is "Tova 2"), and a name a hand still at work
        // carries, or an unknown one, falls back to the next free hand in order. The team
        // counts finished hands too, so the fallback skips every index a pinned hand ever
        // took: the counter alone would name a second Tova. The spawn count moves past
        // the index the fallback used, and by one for a pin, so ordering and future
        // names never shift for a pin: "Gus" on a fresh team does not make the next hand
        // "Ezra 2".
        let team = crewTeam(of: parentID)
        let taken = Set(team.compactMap { $0.crew?.index })
        let rounds = CrewRoster.personas.count
        var pinned: Int?
        if let base = preferredIndex, base >= 0 {
            var candidate = base
            while taken.contains(candidate), team.contains(where: { $0.crew?.index == candidate && $0.crew?.isFinished == true }) {
                candidate += rounds
            }
            if !taken.contains(candidate) { pinned = candidate }
        }
        var fallback = sequential
        while taken.contains(fallback) { fallback += 1 }
        let index = pinned ?? fallback
        updateThread(parentID) { $0.crewSpawnCount = max((pinned == nil ? index : sequential) + 1, parent.crewSpawnCount + 1) }
        let launch = crewLaunch(for: parent)
        // A delegation routed to one of the pair's hand profiles runs on that profile's
        // model, effort and provider instead of the pair's shared worker fields; an
        // unknown profile name falls back to the shared fields, like an unrouted one.
        var routed = profile.flatMap { launch?.profile(named: $0) }
        let persona = CrewRoster.persona(at: index)
        // A profile that sends its hand out on a provider this Mac cannot run (not
        // installed, or switched off in Settings) would only fail the hand, as the pair's
        // own hands would (see `crewHandsProvider`): the hand goes out unrouted instead,
        // and the lead's timeline says why.
        if kind == .hosted, let routedProfile = routed, let provider = routedProfile.provider,
           let reason = crewProviderUnavailableReason(provider) {
            runtime(for: parentID).appendCrewNote("Crew: the \(routedProfile.trimmedName) profile runs on \(provider.displayName), which \(reason), so \(persona.name) goes out as an ordinary hand.")
            routed = nil
        }
        let resolved = crewProject(named: project, for: parent)
        let target = resolved ?? self.project(parent.projectID)
        let sameProject = target?.id == parent.projectID
        if let reference = project, !reference.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, resolved == nil {
            let fallback = self.project(parent.projectID)?.name ?? "this chat's project"
            runtime(for: parentID).appendCrewNote("Crew: \(crewProjectRefusal(reference)), so \(persona.name) works in \(fallback).")
        }

        // A hosted hand goes out on the pair's hands' provider, which may not be the
        // lead's: there it runs the pair's model or that provider's default, and the lead's
        // model and effort mean nothing to it. A native hand lives in the lead's session.
        // A profile with a provider of its own takes its hand there, where the pair's
        // shared model and effort are another provider's words: the hand runs the
        // profile's own, else that provider's default.
        let handsProvider = kind == .hosted ? (routed?.provider ?? launch?.handsProvider ?? parent.provider) : parent.provider
        let elsewhere = handsProvider != parent.provider
        let sharedFieldsApply = kind != .hosted || handsProvider == (launch?.handsProvider ?? parent.provider)
        let sharedModel = sharedFieldsApply ? launch?.workerModel : nil
        let sharedEffort = sharedFieldsApply ? launch?.workerEffort : nil
        // A hosted hand never outranks the lead that sent it (`CrewHandAuthority`): it runs
        // as Settings › Crew says (`AppSettings.crewHandRuntimeMode`, full access unless
        // asked) up to the lead's own mode, and a plan lead's hands plan. The lead's reply is
        // the model's own words, so the permission cannot come from it. Nobody sits at a
        // hosted hand's chat to answer a prompt, so a hand held below full access asks in its
        // card and sits there until the watchdog or its time limit stops it: the lead's
        // timeline says so (below).
        // A native hand lives in the lead's session and takes the lead's mode.
        let authority = CrewHandAuthority(
            leadRuntime: parent.runtimeMode,
            leadInteraction: parent.interactionMode,
            settingsCeiling: settings.crewHandRuntimeMode
        )
        var hand = ChatThread(
            projectID: target?.id ?? parent.projectID,
            provider: handsProvider,
            model: native?.model ?? routed?.model ?? sharedModel ?? (elsewhere ? providers.defaultModel(for: handsProvider)?.id : parent.model),
            effort: routed?.effort ?? sharedEffort ?? (elsewhere ? nil : parent.effort),
            runtimeMode: kind == .hosted ? authority.runtimeMode : parent.runtimeMode,
            fastMode: false
        )
        if kind == .hosted { hand.interactionMode = authority.interactionMode }
        hand.parentThreadID = parentID
        hand.isInPanel = true
        hand.worktreePath = sameProject ? parent.worktreePath : nil
        hand.branch = sameProject ? parent.branch : nil
        if task.isEmpty {
            hand.title = persona.name
        } else if !sameProject, let target {
            hand.title = "\(persona.name) · \(TextCleanup.singleLine(task, limit: 60)) · \(target.name)"
        } else {
            hand.title = "\(persona.name) · \(TextCleanup.singleLine(task, limit: 60))"
        }
        // A routed hand wears its profile in the title: "Alfa · audit the parser · deep".
        if let routed { hand.title += " · \(routed.name)" }
        hand.hasCustomTitle = true
        var info = CrewHandInfo(index: index, task: task, kind: kind, origin: origin)
        // A hosted hand's phases start with the queue: the time its task waited for a slot
        // (none for one handed out at once). A native hand's timing is its provider's.
        if kind == .hosted { info.queuedAt = queuedAt ?? info.startedAt }
        info.profile = routed?.name
        info.nativeID = native?.id
        info.nativeTaskID = native?.taskID
        info.toolUseID = native?.toolUseID
        info.batchID = batchID
        info.isBackground = native?.isBackground ?? true
        info.canStop = kind == .hosted || native?.taskID != nil || parent.provider == .codex
        hand.crew = info
        // The lead hears once per authority the team carries that a hand was held back, so
        // a hand stopped on a card nobody answers is not a mystery, and a team of eight is
        // not eight notes.
        // A plan hand that will not start (`startHostedHand`) gets no note: its report says why.
        let held = kind == .hosted
            && CrewHandAuthority.planRefusal(provider: handsProvider, interaction: authority.interactionMode) == nil
            && !team.contains {
                $0.crew?.kind == .hosted && $0.runtimeMode == hand.runtimeMode && $0.interactionMode == hand.interactionMode
                    && CrewHandAuthority.planRefusal(provider: $0.provider, interaction: $0.interactionMode) == nil
            }
        insertThread(hand)
        if held, let note = authority.note(for: persona.name) { runtime(for: parentID).appendCrewNote(note) }

        // A new hand brings the deck up and is the one the popover points at.
        let leadRuntime = runtime(for: parentID)
        leadRuntime.showsCrewDeck = true
        leadRuntime.crewSelectedHandID = hand.id
        return hand
    }

    /// Gives a hosted hand its copy of the checkout, then its brief.
    private func startHostedHand(_ id: UUID, attachments: [Attachment], brief: @Sendable (CrewPersona, CrewPrompts.Workplace) -> String) async {
        // A plan hand goes out only where plan is held below the prompt: nothing has started
        // yet, no copy, no brief, so it finishes as failed and the lead hears why in the report.
        if let hand = thread(id), let refusal = CrewHandAuthority.planRefusal(provider: hand.provider, interaction: hand.interactionMode) {
            finishCrewHand(id, status: .failed, summary: refusal)
            return
        }
        guard let hand = thread(id), let info = hand.crew, let parentID = hand.parentThreadID, let lead = thread(parentID),
              let checkout = crewCheckout(of: hand, lead: lead) else { return }
        // A hand on another provider than its lead, with no model chosen for it, runs that
        // provider's default: the catalogue it comes from may not have loaded yet.
        if hand.provider != lead.provider, hand.model == nil {
            await providers.loadCatalog(hand.provider)
            if let model = providers.defaultModel(for: hand.provider)?.id {
                updateThread(id) { $0.model = model }
            }
        }
        var workplace = CrewPrompts.Workplace.shared(path: checkout)
        if settings.crewIsolateHands, let copy = await makeCrewCopy(for: hand, of: checkout) {
            updateThread(id) {
                $0.worktreePath = copy.path
                $0.branch = nil
            }
            updateCrewHand(id) { $0.baseTree = copy.tree }
            workplace = .ownCopy(path: copy.path)
        }
        // Stopped while its copy was being made: it never starts.
        guard thread(id)?.crew?.status == .running else { return }
        let handRuntime = runtime(for: id)
        // Its run starts with the brief; the copy and the catalogue before it were startup.
        updateCrewHand(id) { $0.executionStartedAt = .now }
        // A plan hand's task usually asks for a change it cannot make: its brief ends by saying so.
        let text = brief(info.persona, workplace)
        handRuntime.sendCrewBrief(hand.interactionMode == .plan ? text + "\n\n" + CrewHandAuthority.planHandNotice : text, attachments: attachments)
    }

    /// A worktree for `hand` holding the checkout exactly as it is, uncommitted and
    /// untracked work included: a tree of the working files, committed on top of HEAD
    /// where no branch will ever see it, and checked out detached. Nil where the project
    /// cannot be copied (no git, no commits yet) or git refuses; the hand then works in
    /// the checkout itself.
    private func makeCrewCopy(for hand: ChatThread, of checkout: String) async -> (path: String, tree: String)? {
        guard let info = hand.crew, let project = project(hand.projectID) else { return nil }
        // A hand's work landing in the checkout right now finishes first, so the copy
        // starts from the checkout whole rather than half-way through a patch.
        if let parentID = hand.parentThreadID { await existingRuntime(for: parentID)?.crewLanding?.value }
        let git = Git(checkout)
        guard await git.isRepository(), await git.hasCommits() else { return nil }
        let folder = project.name.replacingOccurrences(of: " ", with: "-").lowercased()
        let name = info.persona.name.replacingOccurrences(of: " ", with: "-").lowercased()
        let suffix = String(hand.id.uuidString.lowercased().prefix(8))
        let path = Storage.worktreesDirectory.appendingPathComponent("\(folder)-\(name)-\(suffix)").path
        do {
            // The hands of one delegation block start within milliseconds of each other
            // and want the same checkout, so they share one capture of it (see
            // `CrewTreeCache`); only the worktree itself is made per hand.
            let capture = try await CrewTreeCache.capture(of: checkout, with: git)
            try await git.addDetachedWorktree(at: path, commit: capture.commit)
            return (path, capture.tree)
        } catch {
            try? FileManager.default.removeItem(atPath: path)
            return nil
        }
    }

    func updateCrewHand(_ id: UUID, _ change: (inout CrewHandInfo) -> Void) {
        updateThread(id) { thread in
            guard var info = thread.crew else { return }
            change(&info)
            thread.crew = info
        }
    }

    // MARK: - Reporting back

    /// A hand is done: its status and report land on it, and its lead hears about it. A
    /// native hand's result reaches the lead through the provider; a hosted hand's
    /// report is relayed by the lead's runtime, which waits for the rest of a batch.
    func finishCrewHand(_ id: UUID, status: TurnStatus, summary: String?, landing: CrewLanding? = nil) {
        guard let hand = thread(id), let info = hand.crew, !info.isFinished else { return }
        // A hand the watchdog stopped tells its lead why, ahead of whatever it had said.
        var summary = summary
        if let stall = CrewWatchdog.stalledHands.removeValue(forKey: id) {
            let held: String? = hand.interactionMode == .plan ? CrewWatchdog.planMode : (hand.runtimeMode == .fullAccess ? nil : hand.runtimeMode.title)
            summary = [CrewWatchdog.stalledNote(stalledAfter: stall.after, reason: stall.reason, held: held), summary ?? ""].filter { !$0.isEmpty }.joined(separator: "\n\n")
        }
        let outcome: CrewHandInfo.Status = switch status {
        case .completed: .completed
        case .failed: .failed
        case .interrupted: .stopped
        case .running: .running
        }
        // The progress a native hand reported while it ran lands on the record now, once,
        // and so does what either kind spent: a native hand's running total, a hosted
        // hand's recorded turns (see `ThreadRuntime.crewSpentTokens`).
        let live = existingRuntime(for: id)
        // The report goes to disk as well as to the lead, so it can be read without the
        // chat, and after a run that stopped half way.
        let reportPath = writeCrewReport(of: hand, info: info, status: outcome, summary: summary, landing: landing, live: live)
        updateCrewHand(id) {
            $0.status = outcome
            $0.summary = summary ?? $0.summary
            $0.finishedAt = .now
            if let landing { $0.landing = landing }
            if let live {
                if let activity = live.crewActivity { $0.activity = activity }
                $0.toolCalls = max($0.toolCalls, live.crewToolCalls)
                $0.tokens = max($0.tokens, live.crewSpentTokens)
            }
            $0.reportPath = reportPath
        }
        // Opt-in: a finished hand leaves the panel on its own, for the sidebar under its
        // lead, instead of waiting for "Clear finished hands". Running hands stay put. It
        // goes once its panel has played the hand's finish (see `FloatingPanelMetrics.finishHold`),
        // not the instant it is done, and only if it is still there to clear.
        if settings.crewAutoClearFinished {
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(FloatingPanelMetrics.finishHold + 0.2))
                guard let self, let hand = self.thread(id), hand.isInPanel, hand.crew?.isFinished == true else { return }
                self.updateThread(id) { $0.isInPanel = false }
                if let parentID = hand.parentThreadID {
                    let leadRuntime = self.runtime(for: parentID)
                    if leadRuntime.crewSelectedHandID == id { leadRuntime.crewSelectedHandID = nil }
                }
            }
        }
        guard let parentID = hand.parentThreadID, let finished = thread(id)?.crew else { return }
        let leadRuntime = runtime(for: parentID)
        let copyPath = info.hasOwnCopy ? hand.worktreePath : nil
        leadRuntime.crewHandFinished(hand.id, info: finished, status: outcome, summary: summary, landing: landing, copyPath: copyPath)
        // The copy goes back only once the lead has the report, and only from a hand that
        // finished: a hand that was stopped or that failed hands its report the path to
        // its copy, where its half-done work is, and that folder has to still be there
        // when the lead reads it. Those copies go when the hands are cleared by hand, or
        // when the hand is steered on for another turn.
        if settings.crewAutoClearFinished, outcome == .completed {
            releaseCrewCopy(of: id)
        }
    }

    /// A hosted hand's turn ended: its last reply is its report, and the work in its
    /// copy lands in the lead's checkout before the lead hears of it. Native hands finish
    /// through their provider's own events instead.
    func crewHandTurnFinished(_ hand: ChatThread, status: TurnStatus) {
        guard let info = hand.crew, info.kind == .hosted else { return }
        let handRuntime = existingRuntime(for: hand.id)
        // The turn that just closed. A failure is read against it alone, so an error an
        // earlier turn left behind can never be reported as this one's cause.
        let turnID = handRuntime?.turns.last(where: { $0.completedAt != nil })?.id
        // The error this turn itself recorded. Only an error-level notice in the current
        // turn counts: a warning, a note, or tool output is no failure cause.
        let error: String? = if let handRuntime, let turnID {
            handRuntime.entries.last(where: { entry in
                guard entry.turnID == turnID, case .notice(let note) = entry.item.content else { return false }
                return note.level == .error
            }).flatMap { entry -> String? in
                guard case .notice(let note) = entry.item.content else { return nil }
                return note.message
            }
        } else { nil }
        // A completed turn's report is its last reply, as ever. A failed turn's prose is
        // output it did not finish, so it is read from this turn alone.
        let spoken = handRuntime?.entries.last(where: { entry in
            guard case .assistant(let message) = entry.item.content else { return false }
            return status != .failed || (turnID != nil && entry.turnID == turnID && !message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        })
            .flatMap { entry -> String? in
                guard case .assistant(let message) = entry.item.content else { return nil }
                return message.text
            } ?? ""
        let report = status == .failed ? Self.failedTurnReport(error: error, partial: spoken) : spoken
        if let handRuntime {
            let tools = handRuntime.entries.count { $0.kind == .tool }
            updateCrewHand(hand.id) { $0.toolCalls = tools }
        }
        // An API session is memory only, and keeping it is what lets the hand be steered
        // on with its brief and its work still in mind; a process goes, and resumes by id.
        if !hand.provider.isAPIKeyBased { handRuntime?.stopSession() }
        // A hand that died at the door, before a tool ran (the agent's own store locked
        // while several started at once, or its process gone before a word), is started
        // again on its brief after a short pause rather than reported as failed: the lead
        // never hears of a hand that only stumbled on the way out. Two more goes, then it
        // reports the failure.
        // Classified on the turn's error, never on the prose it left behind: a hand that
        // did say something before it fell over is past the door, and partial output is
        // no launch failure.
        if status == .failed, Self.isLaunchFailure(error ?? ""), let handRuntime,
           handRuntime.entries.contains(where: { $0.kind == .tool }) == false,
           crewHandRetries[hand.id, default: 0] < 2,
           let brief = handRuntime.entries.first(where: { $0.kind == .user }).flatMap({ entry -> String? in
               guard case .user(let message) = entry.item.content else { return nil }
               return message.text
           }) {
            crewHandRetries[hand.id, default: 0] += 1
            let attempt = crewHandRetries[hand.id, default: 0]
            let pause = Double(attempt) * 2 + Double(info.index % 4) * 0.5
            Task {
                try? await Task.sleep(for: .seconds(pause))
                // Stopped from the panel, or steered on by hand, meanwhile: leave it.
                guard let handRuntime = existingRuntime(for: hand.id), !handRuntime.isRunning,
                      thread(hand.id)?.crew?.status == .running else { return }
                handRuntime.draft = ComposerDraft(text: brief, attachments: [])
                handRuntime.send()
            }
            return
        }
        // The turn it reports on is over, however it ended: its run stops here, and the
        // landing that may follow is timed on its own.
        updateCrewHand(hand.id) { $0.executionFinishedAt = .now }
        Task {
            // Only finished work lands: a stopped or failed hand keeps its half-done edits
            // in its copy, where a later turn can carry on from them.
            var landing: CrewLanding?
            if status == .completed, info.hasOwnCopy { landing = await landCrewHand(hand.id) }
            // A hand in the checkout itself has nothing to land, finished or not: what it
            // changed is there already. Its files are read off its own tools instead, so
            // its report, the merge and the unmerged count still know which are its.
            if !info.hasOwnCopy { landing = sharedCrewLanding(of: hand.id, turnID: turnID) }
            // Landing takes seconds, and minutes behind a queue or a lead mid-turn. The
            // user may have steered the hand on from the panel in that time: it is at work
            // again, so it is not finished. Marking it so would hand the lead a report for
            // a turn still running and give back the copy the hand's tools are writing
            // into. It reports again when this new turn ends.
            guard existingRuntime(for: hand.id)?.isRunning != true else { return }
            // The hand reported; a report arriving on an already-finished hand (the user
            // steered it on from the panel) goes to the lead as a fresh one.
            if thread(hand.id)?.crew?.isFinished == true { updateCrewHand(hand.id) { $0.status = .running } }
            finishCrewHand(hand.id, status: status, summary: report, landing: landing)
        }
    }

    /// What the lead hears of a hand's failed turn: the error the turn recorded, ahead of
    /// anything the hand managed to say. Its prose is named as partial output rather than
    /// passed off as the finished report, and a turn that recorded no error says so.
    private static func failedTurnReport(error: String?, partial: String) -> String {
        var parts = [error ?? "The turn failed without a recorded error."]
        let partial = partial.trimmingCharacters(in: .whitespacesAndNewlines)
        if !partial.isEmpty { parts.append("Partial output before the failure:\n\n\(partial)") }
        return parts.joined(separator: "\n\n")
    }

    /// Whether a hand's failure report is the agent failing to get going at all, rather
    /// than anything about the task: its store locked, no session, or the process gone.
    private static func isLaunchFailure(_ report: String) -> Bool {
        let text = report.lowercased()
        return ["database is locked", "database locked", "sqlite_busy", "did not start a session", "the agent exited", "stopped unexpectedly"]
            .contains { text.contains($0) }
    }

    /// Where the work of a hand with no copy of its own went: into the checkout, as it
    /// made it. The files are the ones its tools changed in `turnID` (every turn, without
    /// one), relative to the checkout, with the lines those edits added and removed summed
    /// per file: the same record a thread's turn keeps of its own files, since the
    /// checkout's diff mixes in the lead's and the other hands' changes. Only edits that
    /// landed are listed (see `ToolCall.landedEdits`): a write the hand proposed and never
    /// got approved, one the user declined and one that failed are on its rows but are not
    /// in the checkout, and a hand stopped on an approval would otherwise be reported to
    /// its lead as having changed a file it never touched. Build output is left out, as a
    /// landing leaves it. Nil when no tool changed anything in the checkout: a hand writing
    /// through a shell alone is not "changed nothing".
    private func sharedCrewLanding(of id: UUID, turnID: UUID?) -> CrewLanding? {
        guard let hand = thread(id), let parentID = hand.parentThreadID, let lead = thread(parentID),
              let checkout = crewCheckout(of: hand, lead: lead), let handRuntime = existingRuntime(for: id) else { return nil }
        var order: [String] = []
        var files: [String: CrewLanding.File] = [:]
        for entry in handRuntime.entries where entry.kind == .tool && (turnID == nil || entry.turnID == turnID) {
            guard case .tool(let call) = entry.item.content else { continue }
            for edit in call.landedEdits {
                guard let path = TouchedPaths.relative(edit.path, root: checkout), !TouchedPaths.isBuildOutput(path) else { continue }
                if files[path] == nil {
                    order.append(path)
                    files[path] = CrewLanding.File(path: path, additions: 0, deletions: 0)
                }
                files[path]?.additions += edit.additions
                files[path]?.deletions += edit.deletions
            }
        }
        guard !order.isEmpty else { return nil }
        var landing = CrewLanding()
        landing.files = order.compactMap { files[$0] }
        return landing
    }

    /// Carries what changed in a hand's copy since its base into the lead's checkout: the
    /// patch between the two trees, applied there, three-way where the checkout has moved
    /// on. What lands moves the base forward, so a hand steered on later lands only what
    /// is new; a patch that will not apply is kept as a file and the base stays.
    ///
    /// The patch is read from the hand's own copy, which nothing else is touching, so
    /// hands finishing together prepare theirs side by side. Writing into the checkout is
    /// the part that goes one at a time: two patches at once would leave the later one
    /// judged against files the earlier was still changing. A patch over a file the lead
    /// is editing this very turn waits for the lead to come to rest first (see
    /// `waitForSettledCheckout`); the others do not wait for it.
    private func landCrewHand(_ id: UUID) async -> CrewLanding {
        guard let parentID = thread(id)?.parentThreadID else { return CrewLanding() }
        var landing = CrewLanding()
        let prepared: CrewPatch
        do {
            guard let made = try await prepareCrewPatch(id) else { return landing }
            prepared = made
        } catch {
            landing.error = error.localizedDescription
            return landing
        }
        landing.files = prepared.files
        landing.droppedBuildOutputFiles = prepared.droppedBuildOutputFiles
        await waitForSettledCheckout(parentID, paths: prepared.files.map(\.path))
        let leadRuntime = runtime(for: parentID)
        let previous = leadRuntime.crewLanding
        let applied = Task<CrewLanding, Never> {
            await previous?.value
            return await self.applyCrewHand(id, landing: landing, patch: prepared)
        }
        leadRuntime.crewLanding = Task { _ = await applied.value }
        return await applied.value
    }

    /// A hand's work as a patch against the tree its copy started from, with the files it
    /// touches. Nil where the hand changed nothing or has no copy to compare.
    private func prepareCrewPatch(_ id: UUID) async throws -> CrewPatch? {
        guard let hand = thread(id), let info = hand.crew, let base = info.baseTree, let copy = hand.worktreePath else { return nil }
        let copyGit = Git(copy)
        let after = try await copyGit.captureTree()
        guard after != base else { return nil }
        let work = try await Self.crewWork(in: copyGit, from: base, to: after)
        guard !work.paths.isEmpty else { return nil }
        let text = try await copyGit.diff(from: base, to: after, binary: true, paths: work.paths)
        guard !text.isEmpty else { return nil }
        // A binary patch runs to megabytes, and parsing it counts every line: off the
        // main actor, so the chat keeps streaming while a hand lands.
        return await Task.detached(priority: .utility) {
            let files = DiffParser.parse(text).map {
                CrewLanding.File(path: $0.path, additions: $0.additions, deletions: $0.deletions)
            }
            return CrewPatch(
                text: text,
                files: files,
                droppedBuildOutputFiles: work.droppedBuildOutputFiles,
                after: after
            )
        }.value
    }

    /// The files a hand changed in its copy between two trees that are its work: build
    /// output and tool caches (see `TouchedPaths.isBuildOutput`) are left out, and
    /// counted. They are cut from the list before any patch is made rather than from the
    /// patch afterwards: a copy that built the app holds hundreds of megabytes of them
    /// where the project's .gitignore missed them, which was minutes of git, and as much
    /// memory, for a patch of a few source files. A Command Code hand writes its taste
    /// file into its copy on every run, so without this every one of them "changed
    /// something".
    private static func crewWork(in copyGit: Git, from base: String, to after: String) async throws -> (paths: [String], droppedBuildOutputFiles: Int) {
        let changed = try await copyGit.changedPaths(from: base, to: after)
        let paths = changed.filter { !TouchedPaths.isBuildOutput($0) }
        return (paths, changed.count - paths.count)
    }

    /// Waits for the lead's checkout to be a safe place to write: the lead is not in the
    /// middle of a turn that has already edited one of `paths` (a three-way merge would
    /// leave conflict markers in a file the lead is still writing), and the team's work is
    /// not on its way to the remote. Gives up after half an hour and lands anyway, so a
    /// hand's work is never lost to a lead that never comes to rest.
    private func waitForSettledCheckout(_ parentID: UUID, paths: [String]) async {
        guard let leadRuntime = existingRuntime(for: parentID) else { return }
        let wanted = Set(paths)
        let deadline = Date.now.addingTimeInterval(30 * 60)
        while Date.now < deadline {
            let busy = leadRuntime.phase != .idle && !wanted.isEmpty
                && !wanted.isDisjoint(with: crewChatContext(for: parentID).touchedPaths)
            guard busy || leadRuntime.isCrewMerging else { return }
            // A cancelled task's sleep throws at once, without suspending: swallowing
            // that would turn this into a tight loop on the main actor for the rest of
            // the half hour, with the whole app frozen behind it. The landing goes ahead
            // instead, as it does at the deadline.
            guard (try? await Task.sleep(for: .milliseconds(250))) != nil else { return }
        }
    }

    private func applyCrewHand(_ id: UUID, landing: CrewLanding, patch: CrewPatch) async -> CrewLanding {
        var landing = landing
        guard let hand = thread(id), let info = hand.crew,
              let parentID = hand.parentThreadID, let lead = thread(parentID), let checkout = crewCheckout(of: hand, lead: lead) else { return landing }
        // The write itself is timed from here: the wait behind another hand's landing,
        // before this call, is queue rather than landing. It ends however it ends, cleanly,
        // in conflict markers or as a kept patch.
        updateCrewHand(id) {
            $0.landingStartedAt = .now
            $0.landingFinishedAt = nil
        }
        defer { updateCrewHand(id) { $0.landingFinishedAt = .now } }
        let checkoutGit = Git(checkout)
        // A rebase or a merge is half done in the checkout: a patch laid on top of that
        // would be impossible to tell from the operation's own conflicts, and resolving
        // either would lose the other. The work waits as a patch file instead.
        guard !(await checkoutGit.hasOperationInProgress()) else {
            landing.patchPath = keepCrewPatch(patch.text, of: info, handID: hand.id)
            landing.error = "a rebase or merge is underway in the checkout"
            noteCrewLanding(landing, of: info, to: parentID)
            return landing
        }
        do {
            landing.conflicts = try await checkoutGit.apply(patch.text)
            // Only a clean landing moves the base. With conflict markers in the checkout
            // the work is not settled, so the hand keeps its whole diff: steered on, it
            // offers all of it again rather than leaving the unresolved part to nobody.
            if landing.conflicts.isEmpty { updateCrewHand(id) { $0.baseTree = patch.after } }
        } catch {
            landing.patchPath = keepCrewPatch(patch.text, of: info, handID: hand.id)
            landing.error = error.localizedDescription
        }
        // The checkout has moved, so the next hand captures it afresh rather than starting
        // from the tree this patch just changed.
        CrewTreeCache.invalidate(checkout)
        // Work that landed whole is committed at once when the user asked for it, still
        // inside the landing's turn, so the next hand's patch never writes in between.
        // Conflict markers, a kept patch or an error leave the work for the lead to settle.
        if settings.crewCommitsLandingsInEffect, landing.landed, landing.conflicts.isEmpty {
            await commitCrewLanding(landing, of: info, handID: hand.id, leadID: parentID, checkout: checkout)
        }
        // The lead's diff panel now shows the hand's work too.
        existingRuntime(for: parentID)?.noteDiffChanged()
        noteCrewLanding(landing, of: info, to: parentID)
        return landing
    }

    /// Keeps a patch that could not be applied as a file beside the others, and hands back
    /// where it went; nil when even that failed.
    private func keepCrewPatch(_ text: String, of info: CrewHandInfo, handID: UUID) -> String? {
        let slug = info.persona.name.replacingOccurrences(of: " ", with: "-").lowercased()
        let url = Storage.patchesDirectory.appendingPathComponent("\(slug)-\(handID.uuidString.lowercased().prefix(8)).patch")
        guard (try? text.write(to: url, atomically: true, encoding: .utf8)) != nil else { return nil }
        return url.path
    }

    /// A landing that did not go cleanly says so in the lead's timeline: conflict markers
    /// and kept patches are otherwise only a few words in a hand's row, and the lead is
    /// told not to look at git status, so nobody would see them.
    private func noteCrewLanding(_ landing: CrewLanding, of info: CrewHandInfo, to parentID: UUID) {
        guard let leadRuntime = existingRuntime(for: parentID) else { return }
        let name = info.persona.name
        if !landing.conflicts.isEmpty {
            let files = landing.conflicts.map { "`\($0)`" }.joined(separator: ", ")
            // Worded as what happens next, not as damage: the lead settles the markers
            // before anything goes out, so the reader has nothing to do.
            leadRuntime.appendCrewNote("""
            \(name)'s work landed; the lead is settling a conflict in \(landing.conflicts.count == 1 ? "1 file" : "\(landing.conflicts.count) files").
            \(files)

            The checkout had moved on where \(name) was working, so the merge was three-way and left markers in \(landing.conflicts.count == 1 ? "this file" : "these files"). The lead resolves them before the work goes out; nothing to do on your side.
            """)
        }
        if let path = landing.patchPath {
            let files = landing.files.map { "`\($0.path)`" }.joined(separator: ", ")
            leadRuntime.appendCrewNote("""
            \(name)'s work did not land and was kept as a patch.
            \(files)

            \(landing.error.map { "Why it did not land: \($0).\n\n" } ?? "")Apply it with `git apply --3way \(path)` and settle what conflicts.
            """)
        }
    }

    // MARK: - Stopping and clearing

    /// Stops a hand where it runs: a hosted hand's own turn, a native hand through
    /// the lead's session.
    func stopCrewHand(_ id: UUID) {
        guard let hand = thread(id), let info = hand.crew, !info.isFinished else { return }
        switch info.kind {
        case .hosted:
            if let handRuntime = existingRuntime(for: id), handRuntime.isRunning {
                handRuntime.interrupt()
            } else {
                // Not started yet, its copy still being made: it never will.
                finishCrewHand(id, status: .interrupted, summary: nil)
            }
        case .native:
            guard let parentID = hand.parentThreadID, let nativeID = info.nativeID else { return }
            let leadRuntime = runtime(for: parentID)
            Task {
                let stopped = await leadRuntime.stopNativeHand(nativeID)
                if !stopped { self.updateCrewHand(id) { $0.canStop = false } }
            }
        }
    }

    /// Stops every hand of a lead's still at work, and the delegated tasks waiting for one:
    /// otherwise each stopped hand's room goes straight to the next task in line, and hands
    /// are at work again the moment the last one stops.
    func stopAllCrewHands(of parentID: UUID) {
        runtime(for: parentID).dropWaitingHands()
        for hand in crewHands(of: parentID) where hand.crew?.status == .running {
            stopCrewHand(hand.id)
        }
    }

    /// Clears the panel: finished hands move under the lead in the sidebar, and the panel
    /// stays out of the way until the next hand starts. Running hands keep working.
    func dismissCrewHands(of parentID: UUID) {
        clearFinishedCrewHands(of: parentID)
        let leadRuntime = runtime(for: parentID)
        leadRuntime.showsCrewDeck = false
        leadRuntime.crewSelectedHandID = nil
    }

    /// Finished hands leave the panel for the sidebar, and give their copies back.
    func clearFinishedCrewHands(of parentID: UUID) {
        for hand in crewHands(of: parentID) where hand.crew?.isFinished == true {
            updateThread(hand.id) { $0.isInPanel = false }
            releaseCrewCopy(of: hand.id)
        }
    }

    /// Removes the copy of the checkout a hand worked in, once nothing lives only there:
    /// its work has landed, or what never landed is kept as a patch first. A copy whose
    /// work could not be kept stays where it is, still the hand's, and the lead is told
    /// where. A hand talked to afterwards works in the checkout itself.
    func releaseCrewCopy(of id: UUID) {
        guard let hand = thread(id), let info = hand.crew, info.hasOwnCopy, let copy = hand.worktreePath,
              let project = project(hand.projectID) else { return }
        // A hand back at work keeps its copy: `git worktree remove --force` on the folder
        // its tools are writing into would take the work with it.
        guard existingRuntime(for: id)?.isRunning != true else { return }
        // One release at a time per hand: "Clear finished hands" pressed again while the
        // patch is still being written must not start a second removal beside it.
        guard CrewCopyRelease.inFlight.insert(id).inserted else { return }
        // Work that never landed lives only in the copy, and the hand's report points the
        // lead at it. It is kept as a patch before the folder goes.
        let unlanded = info.status != .completed || info.landing == nil
        let base = info.baseTree
        let name = info.persona.name
        // The session's tools are rooted in the copy; the next turn starts a fresh one.
        existingRuntime(for: id)?.stopSession()
        Task {
            defer { CrewCopyRelease.inFlight.remove(id) }
            if unlanded, let base {
                // Until the patch is on disk the copy is the only place that work exists:
                // it is not let go of, nor named away from the hand, before then.
                guard await self.keepCrewCopyAsPatch(id, name: name, copy: copy, base: base) else { return }
            }
            // Talked to again while the patch was written: the copy is the hand's once more.
            guard self.existingRuntime(for: id)?.isRunning != true, self.thread(id)?.worktreePath == copy else { return }
            self.updateThread(id) {
                $0.worktreePath = nil
                $0.branch = nil
            }
            self.updateCrewHand(id) { $0.baseTree = nil }
            try? await Git(project.path).removeWorktree(at: copy)
        }
    }

    /// Writes what a hand changed in its copy but never landed to a patch file, and tells
    /// the lead where it went. Nothing to keep where the copy holds no changes of its own.
    /// Returns whether the copy may go: true once the work is kept or there is none, false
    /// when git or the disk failed on the way, in which case the lead is told that the copy
    /// stays and where it is.
    private func keepCrewCopyAsPatch(_ id: UUID, name: String, copy: String, base: String) async -> Bool {
        let copyGit = Git(copy)
        let patch: String
        do {
            // A copy that differs from its base only by build output, or by the cache the
            // hand's own tool wrote there, holds no work of the hand's: nothing to keep, and
            // nothing to tell the lead. Sixteen hands cleared together once put sixteen of
            // these notes into the lead's chat over five minutes, for 4 GB of build output.
            let after = try await copyGit.captureTree()
            guard after != base else { return true }
            let work = try await Self.crewWork(in: copyGit, from: base, to: after)
            guard !work.paths.isEmpty else { return true }
            patch = try await copyGit.diff(from: base, to: after, binary: true, paths: work.paths)
            guard !patch.isEmpty else { return true }
        } catch {
            noteCrewCopyKept(id, name: name, copy: copy, reason: error.localizedDescription)
            return false
        }
        let slug = name.replacingOccurrences(of: " ", with: "-").lowercased()
        let url = Storage.patchesDirectory.appendingPathComponent("\(slug)-\(id.uuidString.lowercased().prefix(8))-unlanded.patch")
        do {
            try patch.write(to: url, atomically: true, encoding: .utf8)
        } catch {
            noteCrewCopyKept(id, name: name, copy: copy, reason: error.localizedDescription)
            return false
        }
        guard let parentID = thread(id)?.parentThreadID, let leadRuntime = existingRuntime(for: parentID) else { return true }
        leadRuntime.appendCrewNote("""
        \(name)'s unfinished work was kept as a patch before its copy was cleared.
        \(url.path)

        Nothing of it had landed in your checkout. Apply it with `git apply --3way \(url.path)` if you still want it.
        """)
        return true
    }

    /// Tells the lead that a hand's copy was left in place because its unlanded work could
    /// not be kept as a patch. Read into the lead's thread even when that thread is not
    /// open: this note is the only pointer to work that lives nowhere else.
    private func noteCrewCopyKept(_ id: UUID, name: String, copy: String, reason: String) {
        guard let parentID = thread(id)?.parentThreadID else { return }
        runtime(for: parentID).appendCrewNote("""
        \(name)'s copy was kept: its unfinished work could not be saved as a patch (\(reason)).
        \(copy)

        Nothing of it has landed in your checkout, and the copy stays until its work is taken or the hand is cleared again.
        """)
    }

    // MARK: - Watchdog

    /// Keeps an eye on the hosted hands at work, every half minute. One quiet for
    /// `CrewLimits.watchdogThinkingAfter` (no tool, no word of a reply) says so in its row,
    /// "Thinking for 3 min…", rather than sitting on its last tool; one quiet for
    /// `watchdogStalledAfter` with no tool since the last look has stalled if it is waiting
    /// on an approval or a question, or if nothing its tools did this turn wrote a file
    /// (see `CrewWatchdog.stallReason`), and is stopped the way the panel's Stop stops it,
    /// with a report that opens by telling the lead why (see `finishCrewHand`). With no
    /// stall limit set it is only ever labelled. Native hands are their provider's to
    /// watch. One loop for the app; calling this again does nothing.
    func startCrewWatchdog() {
        guard CrewWatchdog.loop == nil else { return }
        CrewWatchdog.loop = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: CrewWatchdog.tick)
                guard let self else { return }
                self.watchCrewHands()
            }
        }
    }

    /// One look at the hands: the label goes on and comes off, a stalled hand is stopped.
    private func watchCrewHands() {
        let limits = settings.crewLimits
        var toolCounts: [UUID: Int] = [:]
        for hand in threads where hand.isCrewHand && hand.crew?.kind == .hosted && hand.crew?.status == .running {
            guard let handRuntime = existingRuntime(for: hand.id), handRuntime.isRunning else { continue }
            let idle = handRuntime.crewIdleSeconds
            let tools = handRuntime.crewToolCalls
            toolCounts[hand.id] = tools
            if let stalledAfter = limits.watchdogStalledAfter, idle >= stalledAfter, CrewWatchdog.toolCounts[hand.id] == tools,
               let reason = CrewWatchdog.stallReason(awaitsPerson: handRuntime.crewAwaitsPerson, landedEdits: handRuntime.crewLandedEdits) {
                // Stopping a hand closes its turn and clears the very request it was stopped
                // for, so a look that comes before the turn has closed may find another
                // reason: the first one stands, and it is the one the lead is told.
                if CrewWatchdog.stalledHands[hand.id] == nil { CrewWatchdog.stalledHands[hand.id] = (stalledAfter, reason) }
                CrewWatchdog.labelled.remove(hand.id)
                stopCrewHand(hand.id)
                continue
            }
            if idle >= limits.watchdogThinkingAfter {
                // Written only when the minute changes, so the row is not redrawn each look.
                let label = "Thinking for \(Int(idle / 60)) min…"
                if handRuntime.crewActivity != label { handRuntime.crewActivity = label }
                CrewWatchdog.labelled.insert(hand.id)
            } else if CrewWatchdog.labelled.remove(hand.id) != nil, handRuntime.crewActivity?.hasPrefix("Thinking for ") == true {
                // Back at work: the label comes off, and the row says "working" again.
                handRuntime.crewActivity = nil
            }
        }
        CrewWatchdog.toolCounts = toolCounts
        CrewWatchdog.labelled.formIntersection(toolCounts.keys)
    }

    /// Hands whose copies are gone from disk (deleted by hand, say) work in the checkout
    /// from now on, instead of failing every tool call. And the other way round: a copy
    /// on disk that no chat names any more (the app quit with a hand half-way made, or
    /// removing it failed) is swept away.
    func sweepCrewCopies() {
        // Copies deleted from disk still hold their names in the registry until pruned:
        // only the projects that ever had a copy, one git process at a time.
        let projects = self.projects
        let projectsWithCopies = Set(threads.filter { $0.crew?.hasOwnCopy == true }.map(\.projectID))
        Task {
            for project in projects where projectsWithCopies.contains(project.id) { await Git(project.path).pruneWorktrees() }
            await sweepOrphanCrewCopies(of: projects)
        }
        for hand in threads where hand.crew?.hasOwnCopy == true {
            guard let copy = hand.worktreePath, !FileManager.default.fileExists(atPath: copy) else { continue }
            updateThread(hand.id) {
                $0.worktreePath = nil
                $0.branch = nil
            }
            updateCrewHand(hand.id) { $0.baseTree = nil }
        }
        startCrewWatchdog()
    }

    /// Removes the folders in the worktrees root that were made for hands of `projects`
    /// and that no thread names. Only a hand's copy goes: its folder is named for its
    /// project, its persona and its thread ("bottega-gus-daa2ebed", see
    /// `makeCrewCopy`), and nothing else in that root matches. A chat's own worktree
    /// (kept on purpose when the chat was deleted) and folders other tools made there are
    /// never touched, nor is a folder a thread names, nor one young enough to be a copy
    /// still being made. Nothing goes while a library that did not read is kept beside this
    /// one: its threads may name copies this library knows nothing of, and their work is
    /// still in them (see `Storage.keepsUnreadableLibrary`).
    private func sweepOrphanCrewCopies(of projects: [Project]) async {
        guard !Storage.keepsUnreadableLibrary else { return }
        let root = Storage.worktreesDirectory
        guard let folders = try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: [.creationDateKey], options: [.skipsHiddenFiles]) else { return }
        let named = Set(threads.compactMap { $0.worktreePath.map { URL(fileURLWithPath: $0).lastPathComponent } })
        let slugs = projects.map { (project: $0, slug: $0.name.replacingOccurrences(of: " ", with: "-").lowercased()) }
        let personas = Set(CrewRoster.personas.map { $0.name.replacingOccurrences(of: " ", with: "-").lowercased() })
        for folder in folders where !named.contains(folder.lastPathComponent) {
            let name = folder.lastPathComponent
            // The longest project slug that opens the name is its project.
            guard let owner = slugs.filter({ name.hasPrefix($0.slug + "-") }).max(by: { $0.slug.count < $1.slug.count }) else { continue }
            // What follows the slug is "<persona>-<8 hex>", or "<persona>-<round>-<8 hex>".
            var parts = name.dropFirst(owner.slug.count + 1).split(separator: "-").map(String.init)
            guard parts.count >= 2, let suffix = parts.popLast(), suffix.count == 8, suffix.allSatisfy(\.isHexDigit) else { continue }
            if parts.count == 2, Int(parts[1]) != nil { parts.removeLast() }
            guard parts.count == 1, personas.contains(parts[0]) else { continue }
            if let born = try? folder.resourceValues(forKeys: [.creationDateKey]).creationDate, Date.now.timeIntervalSince(born) < 600 { continue }
            // Git takes it out of its registry along with the folder; a folder git does
            // not know (the app quit between making it and registering it) goes by hand.
            if (try? await Git(owner.project.path).removeWorktree(at: folder.path)) == nil {
                try? FileManager.default.removeItem(at: folder)
            }
        }
    }

    /// What a hosted hand is told about the main chat, queued or delegated: the
    /// user's last request, the lead's latest reply and the files it has touched this turn.
    /// Read from the timeline, so it costs no model call.
    func crewChatContext(for parentID: UUID) -> CrewPrompts.ChatContext {
        guard let leadRuntime = existingRuntime(for: parentID) else { return CrewPrompts.ChatContext() }
        var context = CrewPrompts.ChatContext()
        for entry in leadRuntime.entries.reversed() {
            switch entry.item.content {
            case .user(let message) where context.lastUserPrompt == nil && !message.isFromCrew:
                context.lastUserPrompt = message.text
            case .assistant(let message) where context.lastReply == nil:
                context.lastReply = message.text
            default:
                break
            }
            if context.lastUserPrompt != nil, context.lastReply != nil { break }
        }
        let currentTurn = leadRuntime.turns.last?.id
        var touched: [String] = []
        var seen: Set<String> = []
        for entry in leadRuntime.entries where entry.kind == .tool && entry.item.turnID == currentTurn {
            guard case .tool(let call) = entry.item.content else { continue }
            for edit in call.edits where !edit.path.isEmpty && seen.insert(edit.path).inserted {
                touched.append(edit.path)
            }
        }
        context.touchedPaths = touched
        return context
    }
}
