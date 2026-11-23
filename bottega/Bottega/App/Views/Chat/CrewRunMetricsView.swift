// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// What a Crew run recorded, read off the records themselves: the tokens the lead's turns
/// and each hand spent, and the phases the hands went through.
///
/// Scoped as conversation totals on purpose. A request a provider continues lands on a
/// following turn, so no turn can be tied back to the delegation that started it; these
/// numbers are labelled as what the whole conversation recorded, never as one run's
/// comparison. A hand's spend is what its own turns recorded, or, for a hand running
/// inside the lead's session, the running total its provider reports; the view reads that
/// total on a slow tick of its own rather than with every report (see `CrewRunMetricsView`).
struct CrewRunMetrics: Equatable {
    /// What one scope spent, as the providers reported it: the lead's turns, or the hands.
    struct Scope: Equatable {
        /// What the scope counts over (the lead's turns, or its hands), and how many of
        /// them carried usage.
        var parts = 0
        var partsWithUsage = 0
        var hasUnloadedHistory = false
        /// Nil when nothing in the scope recorded usage: missing, never shown as zero.
        var tokens: Int?
        /// Where the scope's tokens came from, biggest first.
        var groups: [Group] = []

        /// Some of the scope's parts recorded nothing, so its total is a floor.
        var isPartial: Bool { hasUnloadedHistory || (tokens == nil ? parts > 0 : partsWithUsage < parts) }
    }

    /// One provider and model's share of a scope.
    struct Group: Equatable, Identifiable {
        var id: String
        var provider: ProviderKind
        var model: String?
        var tokens: Int
    }

    /// A phase boundary that was never recorded stays unavailable rather than reading as a
    /// duration nobody measured.
    enum Phase: Equatable {
        case measured(TimeInterval)
        case running
        case unavailable

        var text: String {
            switch self {
            case .measured(let seconds): RelativeTime.duration(max(0, seconds))
            case .running: "in progress"
            case .unavailable: "unavailable"
            }
        }
    }

    /// One hand's phases: the same boundaries the hand's record holds, and nothing inferred.
    struct HandTiming: Equatable, Identifiable {
        var id: UUID
        var personaIndex: Int
        var name: String
        var status: CrewHandInfo.Status
        /// Nil where a phase does not apply: a hand running inside the lead's session has
        /// no queue, startup or landing of Bottega's, a hosted hand runs nothing before
        /// its brief goes out and lands nothing until its work is applied, and a record from
        /// before the phases were kept holds none of them.
        var queue: Phase?
        var startup: Phase?
        var execution: Phase?
        var landing: Phase?
        /// What the hand spent, as its turns recorded it or its provider reported it; nil
        /// where neither did.
        var tokens: Int?
    }

    var lead = Scope()
    var hands = Scope()
    var timings: [HandTiming] = []

    /// The recorded total, nil when neither the lead nor its hands recorded anything.
    var recordedTokens: Int? {
        switch (lead.tokens, hands.tokens) {
        case let (leadTokens?, handTokens?): leadTokens + handTokens
        case let (leadTokens?, nil): leadTokens
        case let (nil, handTokens?): handTokens
        case (nil, nil): nil
        }
    }

    /// Some of the lead's turns or some hands recorded no usage, so the total is a floor.
    var isPartial: Bool { lead.isPartial || hands.isPartial }

    /// Two providers or more are blended into one number, which guides rather than bills.
    var spansProviders: Bool {
        Set((lead.groups + hands.groups).map(\.provider)).count > 1
    }

    var hasTimings: Bool { !timings.isEmpty }
}

/// Reads the snapshot. Closures supply each hand's turns and the running total its
/// provider reported, so the view stays the only place that touches the hand runtimes.
@MainActor
enum CrewRunMetricsReader {
    /// `handTurns` gives a hand's own turns, nil while its history is still loading;
    /// `handReported` the running total its provider reported for it while it works (a
    /// hand inside the lead's session reports its spend that way), zero where none was.
    static func snapshot(
        leadTurns: [TurnRecord],
        hands: [ChatThread],
        handTurns: @MainActor (ChatThread) -> [TurnRecord]?,
        handReported: @MainActor (ChatThread) -> Int
    ) -> CrewRunMetrics {
        var metrics = CrewRunMetrics()
        // A spend a provider reported inside the lead's session for one of its hands
        // carries that hand's id: it is the hand's alone, never the lead's as well (see
        // `TokenSpend.agentID`), so it counts under the hand it names.
        metrics.lead = spend(in: leadTurns.map { ($0.tokenSpends ?? []).filter { $0.agentID == nil } })
        var tagged: [String: [TokenSpend]] = [:]
        for turn in leadTurns {
            for entry in turn.tokenSpends ?? [] {
                guard let agent = entry.agentID else { continue }
                tagged[agent, default: []].append(entry)
            }
        }
        var handScope = CrewRunMetrics.Scope()
        var handGroups: [CrewRunMetrics.Group] = []
        var timings: [CrewRunMetrics.HandTiming] = []
        for hand in hands {
            guard let info = hand.crew else { continue }
            handScope.parts += 1
            let turns = handTurns(hand)
            var spends = (turns ?? []).flatMap { $0.tokenSpends ?? [] }
            if let nativeID = info.nativeID { spends += tagged[nativeID] ?? [] }
            var tokens: Int?
            if !spends.isEmpty {
                tokens = spends.reduce(0) { $0 + $1.totalTokens }
                handGroups += groups(of: spends)
            } else {
                // Nothing on its turns: a hand inside the lead's session reports a running
                // total instead, live while it works and on its record once it is done.
                let reported = max(info.tokens, handReported(hand))
                if reported > 0 {
                    tokens = reported
                    let id = "\(hand.provider.rawValue)|\(hand.model ?? "")"
                    handGroups.append(CrewRunMetrics.Group(id: id, provider: hand.provider, model: hand.model, tokens: reported))
                } else if turns == nil {
                    handScope.hasUnloadedHistory = true
                }
            }
            if let tokens {
                handScope.partsWithUsage += 1
                handScope.tokens = (handScope.tokens ?? 0) + tokens
            }
            timings.append(timing(of: hand, info: info, tokens: tokens))
        }
        handScope.groups = merged(handGroups)
        metrics.hands = handScope
        metrics.timings = timings
        return metrics
    }

    /// The tokens the given turns recorded, one list of spends per turn, with every
    /// spend grouped by the provider and model that reported it.
    private static func spend(in turns: [[TokenSpend]]) -> CrewRunMetrics.Scope {
        var record = CrewRunMetrics.Scope()
        record.parts = turns.count
        var spent: [CrewRunMetrics.Group] = []
        var total = 0
        for spends in turns where !spends.isEmpty {
            record.partsWithUsage += 1
            total += spends.reduce(0) { $0 + $1.totalTokens }
            spent += groups(of: spends)
        }
        guard record.partsWithUsage > 0 else { return record }
        record.tokens = total
        record.groups = merged(spent)
        return record
    }

    /// Each spend as a group of its own provider and model; `merged` adds them up.
    private static func groups(of spends: [TokenSpend]) -> [CrewRunMetrics.Group] {
        spends.map { entry in
            CrewRunMetrics.Group(id: "\(entry.provider.rawValue)|\(entry.model ?? "")", provider: entry.provider, model: entry.model, tokens: entry.totalTokens)
        }
    }

    /// One group per provider and model, biggest first.
    private static func merged(_ groups: [CrewRunMetrics.Group]) -> [CrewRunMetrics.Group] {
        var byID: [String: CrewRunMetrics.Group] = [:]
        for group in groups {
            if var existing = byID[group.id] {
                existing.tokens += group.tokens
                byID[group.id] = existing
            } else {
                byID[group.id] = group
            }
        }
        return byID.values.sorted {
            $0.tokens == $1.tokens ? $0.id < $1.id : $0.tokens > $1.tokens
        }
    }

    /// A hand's phases, from the boundaries its record carries, and its spend.
    private static func timing(of hand: ChatThread, info: CrewHandInfo, tokens: Int?) -> CrewRunMetrics.HandTiming {
        var timing = CrewRunMetrics.HandTiming(
            id: hand.id,
            personaIndex: info.index,
            name: info.persona.name,
            status: info.status,
            tokens: tokens
        )
        let isRunning = info.status == .running
        switch info.kind {
        case .native:
            // Inside the lead's session its whole life is its run: from the provider's
            // word that it started to the word that it ended.
            timing.execution = phase(from: info.startedAt, to: info.finishedAt, isRunning: isRunning)
        case .hosted:
            if let queued = info.queuedAt {
                timing.queue = .measured(info.startedAt.timeIntervalSince(queued))
            }
            if let executing = info.executionStartedAt {
                timing.startup = .measured(executing.timeIntervalSince(info.startedAt))
                timing.execution = phase(from: executing, to: info.executionFinishedAt, isRunning: isRunning)
            } else if isRunning {
                // Its copy is still being made: the brief has not gone out yet.
                timing.startup = .running
            }
            // A hand that never landed has no landing phase; only a hand that started one
            // and cannot show its end is missing data. It lands while still marked at work.
            if info.landingStartedAt != nil || info.landingFinishedAt != nil {
                timing.landing = phase(from: info.landingStartedAt, to: info.landingFinishedAt, isRunning: isRunning)
            }
        }
        return timing
    }

    private static func phase(from start: Date?, to end: Date?, isRunning: Bool) -> CrewRunMetrics.Phase {
        guard let start else { return .unavailable }
        guard let end else { return isRunning ? .running : .unavailable }
        return .measured(end.timeIntervalSince(start))
    }
}

/// A run's recorded usage and hand timings, at the top of the right sidebar's Crew deck
/// while the usage switch on the team bar is on (see `CrewDeckPane`). One snapshot is read
/// per change to the records behind it, and every few seconds while a hand inside the
/// lead's session works, since the running total such a hand reports is not observed (see
/// `ThreadRuntime.crewTokens`); the card itself is equatable, so a snapshot that did not
/// change leaves its text alone, whatever redrew the deck.
struct CrewRunMetricsView: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let hands: [ChatThread]

    /// How often the snapshot is read again while a native hand works.
    private static let tick: TimeInterval = 5

    var body: some View {
        Group {
            if hands.contains(where: { $0.crew?.status == .running && $0.crew?.kind == .native }) {
                TimelineView(.periodic(from: .now, by: Self.tick)) { _ in
                    CrewRunMetricsCard(metrics: snapshot)
                        .equatable()
                }
            } else {
                CrewRunMetricsCard(metrics: snapshot)
                    .equatable()
            }
        }
        .task(id: hands.map(\.id)) {
            await runtime.ensureLoaded()
            for hand in hands {
                guard !Task.isCancelled else { return }
                await model.runtime(for: hand.id).ensureLoaded()
            }
        }
    }

    private var snapshot: CrewRunMetrics {
        CrewRunMetricsReader.snapshot(
            leadTurns: runtime.turns,
            hands: hands,
            handTurns: { hand in
                guard let handRuntime = model.existingRuntime(for: hand.id), !handRuntime.isLoadingHistory else { return nil }
                return handRuntime.turns
            },
            handReported: { hand in
                model.existingRuntime(for: hand.id)?.crewTokens ?? 0
            }
        )
    }
}

/// The card itself: what was recorded, where it was recorded, and what each hand's phases
/// and spend were. Deliberately flat, the control fill the deck's own cards sit on.
private struct CrewRunMetricsCard: View, Equatable {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    let metrics: CrewRunMetrics

    /// Only the snapshot decides the drawing: the panel redraws for plenty of reasons that
    /// leave the card's text alone.
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.metrics == rhs.metrics }


    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            headline
            VStack(alignment: .leading, spacing: 6) {
                scopeRow("Lead", metrics.lead, counting: "turns")
                scopeRow("Hands", metrics.hands, counting: "hands")
                Divider()
                HStack(spacing: 8) {
                    Text(verbatim: "Total recorded")
                        .zoomedFont(size: 12, weight: .medium)
                        .foregroundStyle(Chrome.primaryText)
                    Spacer(minLength: 8)
                    Text(verbatim: totalText)
                        .zoomedFont(size: 12, weight: .medium, monospacedDigit: true)
                        .foregroundStyle(Chrome.primaryText)
                }
            }
            ForEach(metrics.lead.groups) { group in
                groupRow(group, scope: "Lead")
            }
            ForEach(metrics.hands.groups) { group in
                groupRow(group, scope: "Hands")
            }
            if metrics.hasTimings {
                Divider()
                Text(verbatim: "Hand timings")
                    .zoomedFont(size: 12, weight: .medium)
                    .foregroundStyle(Chrome.primaryText)
                ForEach(metrics.timings) { timing in
                    timingRow(timing)
                }
            }
            notes
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(Chrome.panelControlFill(isDark: colorScheme == .dark))
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Recorded usage and timings"))
    }

    private var headline: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: metrics.recordedTokens == nil ? "No usage recorded yet" : "Recorded usage and timings")
                .zoomedFont(size: 12.5, weight: .semibold)
                .foregroundStyle(Chrome.primaryText)
            Text(verbatim: "This conversation's lead and hands, as their providers reported it")
                .zoomedFont(size: 11)
                .foregroundStyle(Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var totalText: String {
        guard let tokens = metrics.recordedTokens else { return "Not recorded" }
        let reported = "\(UsagePanel.tokens(tokens)) tokens"
        return metrics.isPartial ? reported + " or more" : reported
    }

    /// One scope's line; `counting` names what its parts are ("turns", "hands"), for the
    /// share of them that carried usage.
    private func scopeRow(_ label: String, _ scope: CrewRunMetrics.Scope, counting: String) -> some View {
        HStack(spacing: 8) {
            Text(verbatim: label)
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
            Spacer(minLength: 8)
            Text(verbatim: scopeText(scope, counting: counting))
                .zoomedFont(size: 12, monospacedDigit: true)
                .foregroundStyle(scope.tokens == nil ? Chrome.secondaryText : Chrome.primaryText)
        }
    }

    private func scopeText(_ scope: CrewRunMetrics.Scope, counting: String) -> String {
        guard let tokens = scope.tokens else {
            return scope.hasUnloadedHistory ? "Loading history…" : (scope.parts == 0 ? "None yet" : "Not recorded")
        }
        let reported = "\(UsagePanel.tokens(tokens)) tokens"
        guard scope.partsWithUsage < scope.parts else { return reported }
        return reported + " · \(scope.partsWithUsage) of \(scope.parts) \(counting)"
    }

    /// The pair that reported this share, so a blended total says which models it blends.
    private func groupRow(_ group: CrewRunMetrics.Group, scope: String) -> some View {
        HStack(spacing: 6) {
            ProviderIcon(provider: group.provider, size: 11)
                .foregroundStyle(Chrome.secondaryText)
            Text(verbatim: modelName(group))
                .zoomedFont(size: 11)
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 8)
            Text(verbatim: "\(UsagePanel.tokens(group.tokens))")
                .zoomedFont(size: 11, monospacedDigit: true)
                .foregroundStyle(Chrome.secondaryText)
        }
        .padding(.leading, 18)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text("\(scope), \(modelName(group)), \(group.tokens) tokens"))
    }

    private func modelName(_ group: CrewRunMetrics.Group) -> String {
        let short = group.model.flatMap { model.providers.model($0, for: group.provider)?.shortName }
        return short ?? group.model ?? group.provider.displayName
    }

    /// One hand's phases on one line: whatever its record actually carries.
    private func timingRow(_ timing: CrewRunMetrics.HandTiming) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                CrewGlyph(persona: CrewRoster.persona(at: timing.personaIndex), size: 14, isRunning: timing.status == .running, status: timing.status)
                Text(verbatim: timing.name)
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(1)
                if timing.status == .running { MiniSpinner() }
                Spacer(minLength: 8)
            }
            Text(verbatim: phaseText(timing))
                .zoomedFont(size: 11, monospacedDigit: true)
                .foregroundStyle(Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.leading, 20)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text("\(timing.name), \(phaseText(timing))"))
    }

    /// The phases that apply to the hand, in order, then what it spent. A hand whose record
    /// holds none of them (one from before they were kept) says so.
    private func phaseText(_ timing: CrewRunMetrics.HandTiming) -> String {
        var parts: [String] = []
        if let queue = timing.queue { parts.append("Queue \(queue.text)") }
        if let startup = timing.startup { parts.append("Startup \(startup.text)") }
        if let execution = timing.execution { parts.append("Run \(execution.text)") }
        if let landing = timing.landing { parts.append("Landing \(landing.text)") }
        if parts.isEmpty { parts.append("Phases not recorded") }
        if let tokens = timing.tokens { parts.append("\(UsagePanel.tokens(tokens)) tokens") }
        return parts.joined(separator: " · ")
    }

    /// What the numbers are, and what they are not.
    private var notes: some View {
        VStack(alignment: .leading, spacing: 3) {
            note("Totals cover the conversation and its retained hands, not one run. Providers may omit usage for some requests.")
            if metrics.isPartial {
                note("Some turns or hands recorded no usage, so the totals are lower bounds.")
            }
            if metrics.spansProviders {
                note("Recorded across providers, so the total guides rather than bills.")
            }
            if metrics.hasTimings {
                note("Hands work at the same time, so their durations overlap and are not added up.")
            }
        }
    }

    private func note(_ text: String) -> some View {
        Text(verbatim: text)
            .zoomedFont(size: 10.5)
            .foregroundStyle(Chrome.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
    }
}
