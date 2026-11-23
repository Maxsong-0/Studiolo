// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// Switches Crew on for the app and sets up its hands: how they work, which messages go
/// to them, what happens when the team is done, the user's own rules for every lead, and
/// which model leads which, per provider.
struct CrewSettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        VStack(alignment: .leading, spacing: Chrome.sectionHeaderSpacing) {
            ChromeCard {
                HStack(spacing: 14) {
                    HStack(spacing: -9) {
                        ForEach(0..<5, id: \.self) { index in
                            CrewGlyph(persona: CrewRoster.persona(at: index), size: 26)
                        }
                    }
                    .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 4) {
                            Text("Crew")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(Chrome.primaryText)
                            InfoHoverButton(help: "How Crew works") { CrewInfoPopover(topic: .crew) }
                        }
                        Text("One chat leads a team of hands on big jobs.")
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 12)
                    SettingsSwitch(isOn: $settings.crewEnabled)
                }
                .padding(.leading, 16)
                .padding(.trailing, Chrome.rowControlTrailingPadding)
                .padding(.vertical, 14)
            }
            // One note for the whole page: the sections below dim together rather than
            // each row repeating why it is off.
            if !settings.crewEnabled {
                Text("Switch Crew on to set up its hands.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .padding(.horizontal, 4)
            }
        }
        Group {
            ChromeSection(title: "Hands", accessory: AnyView(InfoHoverButton(help: "How hands work") { CrewInfoPopover(topic: .hands) })) {
                ChromeCard {
                    toggleRow("New chats lead a team", detail: "Off, a new chat works alone until you right-click its mark and switch Crew on; a project that starts its chats on a pair still leads. Switching one chat leaves this as it is.", isOn: $settings.crewDefaultEnabled)
                    ChromeRowDivider()
                    ChromeRow(
                        title: "Checkout",
                        detail: settings.crewIsolateHands
                            ? "Each hand works in a copy of its own; its changes land in the chat's checkout when it reports."
                            : "Hands work in the chat's checkout itself."
                    ) {
                        ChromeVisualPicker(options: [(true, "Own copy"), (false, "Shared")], selection: $settings.crewIsolateHands) { isolated in
                            Image(systemName: isolated ? "doc.on.doc" : "folder")
                        }
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Permissions", detail: Self.handPermissionsDetail(settings.crewHandRuntimeMode)) {
                        GlassPickerButton(
                            options: AppSettings.crewHandRuntimeModes.map { ($0, $0.title) },
                            selection: $settings.crewHandRuntimeMode,
                            symbol: { $0.symbol }
                        )
                    }
                    ChromeRowDivider()
                    // Sixteen caps and "No cap" are too many for a strip of buttons: the
                    // choices open from one capsule, as the pair editor's do.
                    ChromeRow(
                        title: "Hands at once",
                        detail: settings.crewMaxHands.map { "At most \($0) hands in parallel." }
                            ?? "No cap here. The hands Bottega runs still go out \(settings.crewLimits.uncappedConcurrency) at a time (Limits, below)."
                    ) {
                        GlassPickerButton(
                            options: [(Int?.none, "No cap")] + CrewPair.maxHandsRange.map { (Optional($0), String($0)) },
                            selection: $settings.crewMaxHands,
                            symbol: { $0 == nil ? "infinity" : nil }
                        )
                    }
                    ChromeRowDivider()
                    toggleRow("Clear finished hands", detail: "A finished hand moves from the panel to the sidebar. The right sidebar's Crew page keeps listing it under Finished.", isOn: $settings.crewAutoClearFinished)
                    ChromeRowDivider()
                    toggleRow("Auto-hide hands in the sidebar", detail: "A chat's hands show in the sidebar while one of them runs; the whole card leaves once every hand of that chat has finished, and comes back on its own when one starts again.", isOn: $settings.crewAutoHidesIdleHands)
                    ChromeRowDivider()
                    ChromeRow(title: "Cards", detail: "A hand's card opens to its whole transcript, or shows its progress and last steps until you open it.") {
                        ChromeVisualPicker(options: [(false, "Progress"), (true, "Every step")], selection: $settings.crewShowsHandDetails) { showsSteps in
                            Image(systemName: showsSteps ? "list.number" : "chart.bar")
                        }
                    }
                    ChromeRowDivider()
                    ChromeRow(title: "Panels dock on the right", detail: "Every panel stacks in a column at the right of the chat. Off, panels float in the corner they were dragged to.") {
                        SettingsSwitch(isOn: $settings.panelsDockRight)
                    }
                    ChromeRowDivider()
                    toggleRow("Hands work at a working effort", detail: "A lead thinking above medium sends its hands out at medium on the same model — on a scale without a medium (Z.ai), at the scale's own middle rung. A pair or hand profile that names an effort keeps it either way.", isOn: $settings.crewTempersHandEffort)
                }
            }
            ChromeSection(title: "Before they go out") {
                ChromeCard {
                    toggleRow("Hold hands until you say go", detail: "The lead states how many hands and roughly what they cost; nothing leaves until you send /go. Off, hands go out the moment the lead asks for them.", isOn: $settings.crewHoldsDelegations)
                    ChromeRowDivider()
                    toggleRow("Checks stay with the lead", detail: "A line asking a hand to build or test comes out of its brief, and the chat names it; the lead runs the one build. Off, a hand runs the check its brief names, if it writes no build output.", isOn: $settings.crewStripsVerificationSteps)
                }
            }
            ChromeSection(title: "Messages") {
                ChromeCard {
                    toggleRow("Queued follow-ups start on hands", detail: "A prompt queued behind a running turn goes out at once.", isOn: $settings.crewQueueHands)
                }
            }
            CrewAllowedFoldersSection()
            ChromeSection(title: "When the team is done") {
                ChromeCard {
                    toggleRow("Lead checks the hands' work", detail: "Reads every changed file and corrects it. Slower, more careful.", isOn: $settings.crewReviewHands)
                    ChromeRowDivider()
                    toggleRow("Merge automatically", detail: "Once the lead is done and every hand is back, the team's changes go out the way Merge as below sets.", isOn: $settings.crewAutoMerge)
                    ChromeRowDivider()
                    ChromeRow(title: "Merge as", detail: Self.mergeModeDetail(settings.crewMergeMode)) {
                        ChromeSegmentedPicker(
                            options: CrewMergeMode.allCases.map { ChromeSegmentedOption(value: $0, title: Self.mergeModeTitle($0)) },
                            selection: $settings.crewMergeMode
                        )
                    }
                    .disabled(!settings.crewAutoMerge)
                    .opacity(settings.crewAutoMerge ? 1 : 0.5)
                    ChromeRowDivider()
                    toggleRow("Commit each hand's work as it lands", detail: "A hand's work that lands cleanly is committed on the checkout's branch at once, so a job stopped half way loses none of it. Leaves a commit per hand.", isOn: $settings.crewCommitsLandings, blockedBy: landingCommitBlock(settings))
                }
            }
            CrewLeadInstructionsSection()
            // The cookbook opens from the section's header, at its far right: the card
            // below keeps its width for the words.
            ChromeSection(title: "Pairs", trailing: AnyView(CrewCookbookButton())) {
                ChromeCard {
                    HStack(alignment: .center, spacing: 14) {
                        // A lead and the hands it sends out: the pair, drawn with the roster.
                        HStack(spacing: 6) {
                            CrewGlyph(persona: CrewRoster.persona(at: 0), size: 30)
                            Image(systemName: "arrow.right")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(Chrome.secondaryText)
                            HStack(spacing: -8) {
                                ForEach(1..<4, id: \.self) { index in
                                    CrewGlyph(persona: CrewRoster.persona(at: index), size: 22)
                                }
                            }
                        }
                        .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 3) {
                            Text("One leads, the others build")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(Chrome.primaryText)
                            Text("A pair says which model leads and which runs the hands. Put a deep thinker over quick hands on one provider, or lead on one provider and send the hands out on another: the strongest lead with the fastest team, every time.")
                                .font(.system(size: 11))
                                .foregroundStyle(Chrome.secondaryText)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 14)
                }
                ChromeCard {
                    if settings.crewPairs.isEmpty {
                        ChromeRow(title: "No pairs yet", detail: "Hands run on the chat's own model until a chat joins a pair.") {
                            CrewAddPairButton()
                        }
                    } else {
                        ForEach(Array(settings.crewPairs.enumerated()), id: \.element.id) { index, pair in
                            VStack(spacing: 0) {
                                if index > 0 { ChromeRowDivider() }
                                CrewPairRow(pair: pair)
                            }
                        }
                        ChromeRowDivider()
                        ChromeRow(title: "Another pair", detail: "One per provider is enough.") {
                            CrewAddPairButton()
                        }
                    }
                }
                Text("Pairs sit at the top of the model picker: a tap puts the chat in one. Right-click a project in the sidebar to start each of its new chats on one pair.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .padding(.horizontal, 4)
            }
            CrewLimitsSection()
        }
        .disabled(!settings.crewEnabled)
        .opacity(settings.crewEnabled ? 1 : 0.5)
        .animation(.smooth(duration: 0.2), value: settings.crewEnabled)
        ChromeSection(title: "Where hands run") {
            ChromeCard {
                ChromeRow(title: "Claude, Codex and Copilot", detail: "In the provider's own session, through the lead's own agent tools.") {
                    HStack(alignment: .center, spacing: 6) {
                        ProviderIcon(provider: .claude, size: 14)
                        ProviderIcon(provider: .codex, size: 14)
                        // Copilot's mark is wider than tall, so it fills less of a
                        // square frame; size it up to share the same optical height.
                        ProviderIcon(provider: .copilot, size: 16.75)
                    }
                    .foregroundStyle(Chrome.secondaryText)
                }
                ChromeRowDivider()
                ChromeRow(title: "Other providers", detail: "As threads of their own, asked for at the end of the lead's reply.") {
                    EmptyView()
                }
                ChromeRowDivider()
                ChromeRow(title: "Across providers", detail: "A pair can lead on one provider and run its hands on another.") {
                    CrewPairMark(lead: .claude, hands: .antigravity, leadSize: 14)
                    .foregroundStyle(Chrome.secondaryText)
                }
            }
        }
    }

    private static func mergeModeTitle(_ mode: CrewMergeMode) -> String {
        switch mode {
        case .mergeRequest: "Merge request"
        case .pushBranch: "Push branch"
        case .localCommit: "Local commit"
        }
    }

    /// What a hand's permissions come to, for its row (see `AppSettings.crewHandRuntimeMode`).
    private static func handPermissionsDetail(_ mode: RuntimeMode) -> String {
        let ceiling = " This is the most a hand gets, fixed when it goes out: never more than the chat that sends it, so a Supervised chat's hands are Supervised, and a plan chat's hands only plan and are sent only on the providers that can hold a hand to that."
        switch mode {
        case .fullAccess: return "Hands run commands and edit without asking: no one sits at a hand's chat to answer." + ceiling
        case .supervised: return "Hands ask in their card before every command and edit, and a hand left waiting is stopped." + ceiling
        case .auto: return "Hands edit freely, and routine actions go through as the provider's Auto mode judges them; anything else asks in the hand's card, and a hand left waiting is stopped." + ceiling
        case .autoAcceptEdits: return "Hands edit freely; every command asks in the hand's card, and a hand left waiting is stopped." + ceiling
        }
    }

    private static func mergeModeDetail(_ mode: CrewMergeMode) -> String {
        switch mode {
        case .mergeRequest: "The work goes out on a branch of its own, a merge request is opened and merged, and the checkout is brought up to date."
        case .pushBranch: "The work is committed on a branch and pushed; from the default branch that is a branch of its own, which the checkout then stands on. Opening and merging the merge request is yours."
        case .localCommit: "The work is committed on the checkout's own branch and nothing is pushed. Needs no remote."
        }
    }

    /// Why committing each landing is not on offer right now, when it is not: the settings
    /// it depends on, as `AppSettings.crewCommitsLandingsInEffect` reads them.
    private func landingCommitBlock(_ settings: AppSettings) -> String? {
        if !settings.crewIsolateHands { return "Hands work in the chat's checkout itself, so no work lands to commit." }
        if settings.crewAutoMerge, settings.crewMergeMode.pushes {
            return "A merge request or a pushed branch is made from the uncommitted work: choose Local commit under Merge as, or switch Merge automatically off, to commit as hands land."
        }
        return nil
    }

    /// A toggle row. A reason it is blocked stands in for its detail, and the row is dimmed
    /// and inert while there is one.
    private func toggleRow(_ title: String, detail: String, isOn: Binding<Bool>, blockedBy reason: String? = nil) -> some View {
        ChromeRow(title: title, detail: reason ?? detail) {
            SettingsSwitch(isOn: isOn)
        }
        .disabled(reason != nil)
        .opacity(reason == nil ? 1 : 0.5)
    }
}

/// The user's own rules for every lead: typed here, and a rules file kept elsewhere whose
/// current contents follow the text. The lead's policy ends with them, on every provider
/// (`CrewLaunch.leadInstructions`); a change reaches a lead at its next message.
private struct CrewLeadInstructionsSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        ChromeSection(title: "Lead instructions") {
            ChromeCard {
                ZStack(alignment: .topLeading) {
                    TextEditor(text: $settings.crewLeadInstructions)
                        .font(.system(size: 13))
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: 120)
                    if settings.crewLeadInstructions.isEmpty {
                        Text("For example: before sending hands out, read the project's TASKS file and claim one row per hand; answer in Chinese")
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(.horizontal, 5)
                            .allowsHitTesting(false)
                    }
                }
                .padding(12)
                ChromeRowDivider()
                ChromeRow(title: "Rules file", detail: fileDetail) {
                    HStack(spacing: 6) {
                        if !settings.crewLeadInstructionsFile.isEmpty {
                            Button {
                                model.settings.crewLeadInstructionsFile = ""
                            } label: {
                                Image(systemName: "xmark.circle")
                                    .font(.system(size: 12, weight: .medium))
                                    .foregroundStyle(Chrome.secondaryText)
                                    .frame(width: 24, height: 24)
                                    .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                            .help("Stop reading this file")
                            .accessibilityLabel(Text("Stop reading this file"))
                        }
                        Button(settings.crewLeadInstructionsFile.isEmpty ? "Choose…" : "Change…") { chooseFile() }
                            .buttonStyle(.glass)
                    }
                }
            }
            Text("Every lead gets these after Crew's own rules and follows them where the two disagree. A change reaches a lead at its next message; on a provider without a system prompt they ride with every message.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .padding(.horizontal, 4)
        }
        // What the file holds now, not when it was chosen: it may have been edited since.
        .task { await model.settings.refreshCrewLeadInstructionsFile() }
    }

    /// The rules file row's second line: where the file is and what came of the last read.
    private var fileDetail: String {
        let settings = model.settings
        let path = settings.crewLeadInstructionsFile
        guard !path.isEmpty else {
            return "Optional: a file kept elsewhere, such as a CLAUDE.md. Its contents follow the text above and are read again before each message a lead gets."
        }
        let shown = (path as NSString).abbreviatingWithTildeInPath
        guard let read = settings.crewLeadInstructionsFileRead, read.path == path else { return "\(shown) · Reading…" }
        switch read.content {
        case .text(let text) where text.isEmpty:
            return "\(shown) · Empty, so the leads get the text above alone."
        case .text(let text):
            return "\(shown) · \(text.count.formatted()) characters, read again before each message a lead gets."
        case .unreadable(let reason):
            return "\(shown) · Can't be read (\(reason)), so the leads get the text above alone."
        }
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose a rules file for every Crew lead, such as a CLAUDE.md. Press ⌘⇧. to show hidden folders."
        panel.prompt = "Use for Leads"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.settings.crewLeadInstructionsFile = url.path
    }
}

/// The folders outside the sidebar that hands may be sent into (`crewAllowedProjectRoots`).
/// None, the default, keeps hands to the sidebar's projects; a folder opens every git
/// repository inside it, which joins the sidebar when a hand is first sent there. The
/// lead's policy lists the same folders, and says what a hand may write in the notes vault.
private struct CrewAllowedFoldersSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let roots = model.settings.crewAllowedProjectRoots
        ChromeSection(title: "Repositories outside the sidebar") {
            ChromeCard {
                ForEach(Array(roots.enumerated()), id: \.element) { index, root in
                    VStack(spacing: 0) {
                        if index > 0 { ChromeRowDivider() }
                        CrewAllowedFolderRow(path: root) {
                            withAnimation(Chrome.panelSlide) {
                                model.settings.crewAllowedProjectRoots.removeAll { $0 == root }
                            }
                        }
                    }
                }
                if !roots.isEmpty { ChromeRowDivider() }
                ChromeRow(
                    title: roots.isEmpty ? "Sidebar projects only" : "Another folder",
                    detail: roots.isEmpty
                        ? "Hands work only in the projects in the sidebar. Add a folder to let the lead send hands into the git repositories inside it; each joins the sidebar when a hand goes there."
                        : "The lead may send hands into any git repository inside these folders; each joins the sidebar when a hand goes there."
                ) {
                    Button {
                        chooseFolders()
                    } label: {
                        Label("Add folder…", systemImage: "plus")
                    }
                    .buttonStyle(.glass)
                }
            }
            Text(verbatim: "Wherever they work, hands that write in your notes vault keep to a project's 02-日志, 04-协调 and 06-BUG folders, stay out of _规则 and 01-框架, and never write a key or password there.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .padding(.horizontal, 4)
        }
    }

    /// The folder picker, as a sheet on the Settings window so it never blocks the app.
    private func chooseFolders() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.prompt = "Open to Hands"
        panel.message = "Choose folders whose git repositories Crew's hands may work in."
        let settings = model.settings
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window) { response in
                if response == .OK { Self.open(panel.urls, in: settings) }
            }
        } else {
            panel.begin { response in
                if response == .OK { Self.open(panel.urls, in: settings) }
            }
        }
    }

    /// Adds the chosen folders, each once, as the standardized absolute paths the lead is
    /// told and the delegations are checked against.
    private static func open(_ urls: [URL], in settings: AppSettings) {
        var roots = settings.crewAllowedProjectRoots
        for url in urls {
            let path = (url.path as NSString).standardizingPath
            if !roots.contains(path) { roots.append(path) }
        }
        guard roots != settings.crewAllowedProjectRoots else { return }
        withAnimation(Chrome.panelSlide) { settings.crewAllowedProjectRoots = roots }
    }
}

/// One folder opened to hands: its name over its path, and the button that closes it.
private struct CrewAllowedFolderRow: View {
    let path: String
    let remove: () -> Void

    var body: some View {
        let name = (path as NSString).lastPathComponent
        ChromeRow(title: name.isEmpty ? path : name, detail: (path as NSString).abbreviatingWithTildeInPath) {
            Button(action: remove) {
                Image(systemName: "minus.circle")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 24, height: 24)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Close this folder to hands")
            .accessibilityLabel(Text("Close \(name) to hands"))
        }
    }
}

/// Opens the cookbook: Claude's pairs as they resolve on this Mac, added with a click.
private struct CrewCookbookButton: View {
    @State private var isOpen = false

    var body: some View {
        Button {
            isOpen = true
        } label: {
            Label("Open cookbook", systemImage: "book")
        }
        .buttonStyle(.glass)
        .controlSize(.small)
        .popover(isPresented: $isOpen, arrowEdge: .bottom) {
            CrewCookbookPanel()
                .presentedChrome()
        }
    }
}

private struct CrewAddPairButton: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Button {
            let installed = ProviderKind.allCases.filter { model.providers.status($0).isInstalled && model.settings.isEnabled($0) }
            let taken = Set(model.settings.crewPairs.map(\.provider))
            let provider = installed.first { !taken.contains($0) } ?? installed.first ?? model.settings.defaultProvider
            withAnimation(Chrome.panelSlide) { model.settings.addCrewPair(CrewPair(provider: provider)) }
        } label: {
            Label("Add pair", systemImage: "plus")
        }
        .buttonStyle(.glass)
    }
}

/// One pair: who leads whom, on what effort, with the editor behind a click.
private struct CrewPairRow: View {
    @Environment(AppModel.self) private var model
    let pair: CrewPair

    @State private var isEditing = false
    @State private var isHovering = false

    var body: some View {
        let registry = model.providers
        HStack(spacing: 10) {
            Button {
                isEditing = true
            } label: {
                HStack(spacing: 12) {
                    CrewPairIcons(pair: pair)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: CrewPairSummary.title(pair, registry: registry))
                            .font(.system(size: 13))
                            .foregroundStyle(Chrome.primaryText)
                        Text(verbatim: summary(registry: registry))
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                    }
                    Spacer(minLength: 8)
                    Image(systemName: "slider.horizontal.3")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                        .opacity(isHovering ? 1 : 0)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .onHover { isHovering = $0 }
            .popover(isPresented: $isEditing, arrowEdge: .trailing) {
                // Wide enough for a hand profile's provider, model and effort side by side.
                CrewPairEditor(pairID: pair.id)
                    .frame(width: 480)
                    .presentedChrome()
            }
            Button {
                withAnimation(Chrome.panelSlide) { model.removeCrewPair(pair.id) }
            } label: {
                Image(systemName: "minus.circle")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 24, height: 24)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Remove this pair")
            .accessibilityLabel(Text("Remove this pair"))
        }
        .padding(.leading, 16)
        .padding(.trailing, Chrome.rowControlTrailingPadding)
        .padding(.vertical, 9)
        .task(id: pair.handsProvider) {
            await registry.loadCatalog(pair.provider)
            await registry.loadCatalog(pair.handsProvider)
        }
    }

    /// The row's second line: only what the title and the icons do not already say. The
    /// title names the models, the icons the providers, so this reads the efforts, a hand
    /// cap when one is set, the hand profiles when there are any, and a warning when the
    /// hands' provider is not set up. Defaults say nothing: a pair with no effort chosen,
    /// no cap and no profiles has one short line.
    private func summary(registry: ProviderRegistry) -> String {
        var parts = [CrewPairSummary.providers(pair)]
        if pair.customName != nil { parts.insert(CrewPairSummary.modelsTitle(pair, registry: registry), at: 0) }
        if let efforts = CrewPairSummary.efforts(pair) { parts.append(efforts) }
        if let cap = pair.maxHands {
            parts.append(cap == 1 ? "One hand at a time" : "Up to \(cap) hands")
        }
        if !pair.handProfiles.isEmpty {
            parts.append(pair.handProfiles.count == 1 ? "1 hand profile" : "\(pair.handProfiles.count) hand profiles")
        }
        // Hands on a provider this Mac cannot run stay on the lead's, on the chat's own
        // model (see `AppModel.crewHandsProvider`); the row says so rather than promising
        // a team that would fail.
        if pair.sendsHandsElsewhere, model.crewHandsProvider(of: pair) == pair.provider {
            parts.append("\(pair.handsProvider.displayName) is not set up, hands stay on \(pair.provider.displayName)")
        }
        return parts.joined(separator: " · ")
    }
}

/// A pair's provider marks: the lead's larger and the hands' smaller after an arrow,
/// the same mark wherever a pair is drawn, so a same-provider pair shows
/// provider → provider too and a row reads the same everywhere.
private struct CrewPairIcons: View {
    let pair: CrewPair

    var body: some View {
        CrewPairMark(lead: pair.provider, hands: pair.handsProvider)
        .frame(minWidth: 22, alignment: .leading)
    }
}

/// The pair's settings: provider, lead model and effort, the hands' provider, model and
/// effort, and how many hands go out at once. Every choice writes straight to the pair.
/// The hand profiles at the end are the pair's advanced mode: purpose-named overrides
/// of the shared hands' choices, so an empty list leaves the pair as it always was.
private struct CrewPairEditor: View {
    @Environment(AppModel.self) private var model
    let pairID: UUID

    /// The name field: a popover hands its first field the focus as it opens, and the
    /// caret landing there read as a prompt to rename the pair. The field is given back
    /// its focus only on a click.
    @FocusState private var isNameFocused: Bool

    var body: some View {
        if let pair = model.settings.crewPair(pairID) {
            let registry = model.providers
            let providers = ProviderKind.allCases.filter { model.settings.isEnabled($0) && (registry.status($0).isInstalled || $0 == pair.provider) }
            let handsProviders = ProviderKind.allCases.filter { model.settings.isEnabled($0) && (registry.status($0).isInstalled || $0 == pair.handsProvider) }
            let options = registry.models(for: pair.provider)
            let handsOptions = registry.models(for: pair.handsProvider)
            let leadOption = registry.model(pair.orchestratorModel, for: pair.provider)
            let workerOption = registry.model(pair.workerModel, for: pair.handsProvider)
            // A lead that may be any model offers every effort the provider's models know.
            let leadEfforts = Self.efforts(of: leadOption, fallback: Self.allEfforts(options))
            // Hands with no model chosen inherit the chat's, and its efforts; on another
            // provider they run its default model, so its efforts are the ones on offer.
            let workerEfforts = Self.efforts(of: workerOption, fallback: pair.sendsHandsElsewhere ? Self.efforts(of: registry.defaultModel(for: pair.handsProvider), fallback: Self.allEfforts(handsOptions)) : leadEfforts)
            VStack(alignment: .leading, spacing: 0) {
                // The name is the header: the pair's own word for itself, over what it
                // stands for in the picker when it has none.
                VStack(alignment: .leading, spacing: 4) {
                    TextField("", text: Binding(
                        get: { pair.name ?? "" },
                        set: { text in model.updateCrewPair(pairID) { $0.name = text.isEmpty ? nil : text } }
                    ), prompt: Text(CrewPairSummary.modelsTitle(pair, registry: registry)))
                    .textFieldStyle(.plain)
                    .focused($isNameFocused)
                    .focusEffectDisabled()
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Chrome.primaryText)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 7)
                    .background(Chrome.overlay(0.06), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                    Text("Shown in the picker and the composer")
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                        .padding(.horizontal, 2)
                }
                .padding(.horizontal, 14)
                .padding(.top, 14)
                sectionLabel("Lead")
                editorRow("Provider") {
                    GlassPickerButton(
                        options: providers.map { ($0, $0.displayName) },
                        selection: Binding(
                            get: { pair.provider },
                            set: { provider in
                                model.updateCrewPair(pairID) {
                                    guard $0.provider != provider else { return }
                                    $0.provider = provider
                                    $0.orchestratorModel = nil
                                    $0.orchestratorEffort = nil
                                    // Hands on the old lead provider go with it, and so do
                                    // the profiles riding on them; hands on a provider of
                                    // their own keep their model and effort, and when that
                                    // provider is the new lead's, they are simply on the
                                    // lead's provider again.
                                    if $0.workerProvider == nil {
                                        $0.workerModel = nil
                                        $0.workerEffort = nil
                                        $0.clearInheritedProfileChoices()
                                    } else if $0.workerProvider == provider {
                                        $0.workerProvider = nil
                                    }
                                }
                                Task { await registry.loadCatalog(provider) }
                            }
                        ),
                        asset: { $0.iconName }
                    )
                }
                editorRow("Model") {
                    GlassPickerButton(
                        options: [(String?.none, "Any model")] + options.map { (Optional($0.id), $0.shortName) },
                        selection: Binding(
                            get: { pair.orchestratorModel },
                            set: { id in
                                model.updateCrewPair(pairID) {
                                    $0.orchestratorModel = id
                                    if let effort = $0.orchestratorEffort, let option = registry.model(id, for: $0.provider), !option.efforts.contains(effort) {
                                        $0.orchestratorEffort = nil
                                    }
                                }
                            }
                        ),
                        maxWidth: 190
                    )
                }
                if !leadEfforts.isEmpty {
                    editorRow("Effort") {
                        effortPicker(
                            efforts: leadEfforts,
                            inherit: "Chat's effort",
                            selection: Binding(
                                get: { pair.orchestratorEffort },
                                set: { effort in model.updateCrewPair(pairID) { $0.orchestratorEffort = effort } }
                            )
                        )
                    }
                }
                sectionLabel("Hands")
                editorRow("Provider") {
                    GlassPickerButton(
                        options: [(ProviderKind?.none, "Same as the lead")] + handsProviders.filter { $0 != pair.provider }.map { (Optional($0), $0.displayName) },
                        selection: Binding(
                            get: { pair.sendsHandsElsewhere ? pair.workerProvider : nil },
                            set: { provider in
                                model.updateCrewPair(pairID) {
                                    let chosen = provider == $0.provider ? nil : provider
                                    guard $0.workerProvider != chosen else { return }
                                    // The hands' model and effort were the old provider's,
                                    // and so were those of the profiles riding on the hands.
                                    $0.workerProvider = chosen
                                    $0.workerModel = nil
                                    $0.workerEffort = nil
                                    $0.clearInheritedProfileChoices()
                                }
                                if let provider { Task { await registry.loadCatalog(provider) } }
                            }
                        ),
                        asset: { ($0 ?? pair.provider).iconName }
                    )
                }
                editorRow("Model") {
                    GlassPickerButton(
                        options: [(String?.none, pair.sendsHandsElsewhere ? "\(pair.handsProvider.displayName)'s default" : "Same as the chat")] + handsOptions.map { (Optional($0.id), $0.shortName) },
                        selection: Binding(
                            get: { pair.workerModel },
                            set: { id in
                                model.updateCrewPair(pairID) {
                                    $0.workerModel = id
                                    if let effort = $0.workerEffort, let option = registry.model(id, for: $0.handsProvider), !option.efforts.contains(effort) {
                                        $0.workerEffort = nil
                                    }
                                }
                            }
                        ),
                        maxWidth: 190
                    )
                }
                if !workerEfforts.isEmpty {
                    editorRow("Effort") {
                        // Left open, the hands' effort is what the launch works out
                        // (`AppModel.crewLaunch`): on another provider its default; on the
                        // lead's, the chat's own, brought down to the working rung while
                        // "Hands work at a working effort" is on, which "Same as the chat"
                        // left out.
                        effortPicker(
                            efforts: workerEfforts,
                            inherit: pair.sendsHandsElsewhere
                                ? "\(pair.handsProvider.displayName)'s default"
                                : (model.settings.crewTempersHandEffort ? "Working effort" : "Same as the chat"),
                            selection: Binding(
                                get: { pair.workerEffort },
                                set: { effort in model.updateCrewPair(pairID) { $0.workerEffort = effort } }
                            )
                        )
                    }
                }
                editorRow("At once") {
                    GlassPickerButton(
                        options: [(Int?.none, "As many as it takes")] + CrewPair.maxHandsRange.map { (Optional($0), String($0)) },
                        selection: Binding(
                            get: { pair.maxHands },
                            set: { count in model.updateCrewPair(pairID) { $0.maxHands = count } }
                        )
                    )
                }
                sectionLabel("Hand profiles")
                CrewHandProfilesEditor(pairID: pairID, pair: pair, handsEfforts: workerEfforts)
                ForEach(Self.catalogProviders(pair), id: \.self) { provider in
                    if registry.models(for: provider).isEmpty {
                        Text(registry.loadingCatalogs.contains(provider) ? "Loading \(provider.displayName)'s models…" : "No models loaded for \(provider.displayName) yet.")
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                            .padding(.horizontal, 14)
                            .padding(.top, 8)
                    }
                }
            }
            .padding(.bottom, 14)
            .onAppear {
                // After the popover's own first-responder pass, which runs as it shows.
                DispatchQueue.main.async { isNameFocused = false }
            }
            .task(id: Self.catalogProviders(pair)) {
                for provider in Self.catalogProviders(pair) {
                    await registry.loadCatalog(provider)
                }
            }
        }
    }

    private func editorRow<Control: View>(_ title: String, detail: String? = nil, @ViewBuilder control: () -> Control) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: title)
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let detail {
                    Text(verbatim: detail)
                        .font(.system(size: 11))
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            control()
                .layoutPriority(1)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 5)
    }

    /// A section's word, small and quiet, with a hairline above: the rows under it need
    /// no sentence each once the section says what they are about.
    private func sectionLabel(_ title: String) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Divider()
                .padding(.horizontal, 14)
                .padding(.top, 12)
            Text(verbatim: title)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Chrome.secondaryText)
                .padding(.horizontal, 14)
                .padding(.top, 10)
                .padding(.bottom, 4)
        }
    }

    private func effortPicker(efforts: [String], inherit: String, selection: Binding<String?>) -> some View {
        GlassPickerButton(
            options: [(String?.none, inherit)] + efforts.map { (Optional($0), ModelOption.effortTitle($0)) },
            selection: selection
        )
    }

    /// Every effort any of the provider's models offers, in the order the first lists them.
    fileprivate static func allEfforts(_ options: [ModelOption]) -> [String] {
        var seen: [String] = []
        for option in options {
            for effort in option.efforts where !seen.contains(effort) { seen.append(effort) }
        }
        return seen
    }

    /// A model's efforts for the picker: its declared scale, or the fallback when the
    /// declaration is empty — a catalog that has not walked the model's scale yet would
    /// otherwise take the effort row away from a model that does reason.
    fileprivate static func efforts(of option: ModelOption?, fallback: [String]) -> [String] {
        guard let option, !option.efforts.isEmpty else { return fallback }
        return option.efforts
    }

    /// The providers whose catalogs the editor shows: the lead's and the hands', then those
    /// the hand profiles name, each once and in that order, so the pickers and the loading
    /// notes cover every one.
    private static func catalogProviders(_ pair: CrewPair) -> [ProviderKind] {
        var providers = pair.sendsHandsElsewhere ? [pair.provider, pair.handsProvider] : [pair.provider]
        for provider in pair.handProfiles.compactMap(\.provider) where !providers.contains(provider) {
            providers.append(provider)
        }
        return providers
    }
}

/// The pair's hand profiles, its advanced mode: each a name the lead routes a task to, the
/// provider, model and effort it runs on in place of the pair's shared hands, and what it
/// is for, which the lead reads beside the name. Every edit writes straight to the pair;
/// with none, every hand runs on the choices above, as it always did.
private struct CrewHandProfilesEditor: View {
    @Environment(AppModel.self) private var model
    let pairID: UUID
    let pair: CrewPair
    /// The efforts the pair's own hands are offered, for a profile on their provider with
    /// no model of its own.
    let handsEfforts: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if pair.handProfiles.isEmpty {
                Text("None yet: every hand runs on the choices above. A profile is a hand tuned for one kind of work, and the lead routes that work to it by name.")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(pair.handProfiles) { profile in
                CrewHandProfileRow(pairID: pairID, pair: pair, profile: profile, handsEfforts: handsEfforts)
            }
            Button {
                withAnimation(Chrome.panelSlide) {
                    model.updateCrewPair(pairID) { pair in
                        let name = Self.newName(among: pair.handProfiles)
                        pair.handProfiles.append(CrewHandProfile(name: name))
                    }
                }
            } label: {
                Label("Add profile", systemImage: "plus")
            }
            .buttonStyle(.glass)
            .controlSize(.small)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 4)
    }

    /// The first "profile-N" that no profile of the pair answers to yet.
    private static func newName(among profiles: [CrewHandProfile]) -> String {
        var number = profiles.count + 1
        while profiles.contains(where: { $0.trimmedName.caseInsensitiveCompare("profile-\(number)") == .orderedSame }) {
            number += 1
        }
        return "profile-\(number)"
    }
}

/// One hand profile: its name and a remove button, the provider, model and effort it runs
/// on, and what it is for; under them, why the lead would not reach it as it stands.
private struct CrewHandProfileRow: View {
    @Environment(AppModel.self) private var model
    let pairID: UUID
    let pair: CrewPair
    let profile: CrewHandProfile
    let handsEfforts: [String]

    var body: some View {
        let registry = model.providers
        // No provider of its own keeps the profile on the hands' provider, and a provider
        // that is the hands' own reads the same way, so the pickers show what it runs on.
        let ownProvider = profile.provider.flatMap { $0 == pair.handsProvider ? nil : $0 }
        let provider = ownProvider ?? pair.handsProvider
        let providers = ProviderKind.allCases.filter {
            $0 != pair.handsProvider && model.settings.isEnabled($0) && (registry.status($0).isInstalled || $0 == profile.provider)
        }
        let options = registry.models(for: provider)
        // A profile on the hands' provider with no model of its own runs the hands' model,
        // with their efforts; on a provider of its own it runs that provider's default.
        let fallbackEfforts = ownProvider == nil
            ? handsEfforts
            : CrewPairEditor.efforts(of: registry.defaultModel(for: provider), fallback: CrewPairEditor.allEfforts(options))
        let efforts = CrewPairEditor.efforts(of: registry.model(profile.model, for: provider), fallback: fallbackEfforts)
        let inherit = ownProvider.map { "\($0.displayName)'s default" } ?? "Same as the hands"
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                TextField("", text: Binding(
                    get: { profile.name },
                    set: { name in update { $0.name = name } }
                ), prompt: Text("Name the lead routes by"))
                .textFieldStyle(.plain)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Chrome.primaryText)
                Button {
                    let id = profile.id
                    withAnimation(Chrome.panelSlide) {
                        model.updateCrewPair(pairID) { pair in pair.handProfiles.removeAll { $0.id == id } }
                    }
                } label: {
                    Image(systemName: "minus.circle")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 24, height: 24)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Remove this profile")
                .accessibilityLabel(Text("Remove this profile"))
            }
            HStack(spacing: 6) {
                GlassPickerButton(
                    options: [(ProviderKind?.none, pair.handsProvider.displayName)] + providers.map { (Optional($0), $0.displayName) },
                    selection: Binding(
                        get: { ownProvider },
                        set: { chosen in
                            let hands = pair.handsProvider
                            update { profile in
                                let before = profile.provider ?? hands
                                profile.provider = chosen == hands ? nil : chosen
                                // The model and effort were the old provider's.
                                guard (chosen ?? hands) != before else { return }
                                profile.model = nil
                                profile.effort = nil
                            }
                            if let chosen { Task { await registry.loadCatalog(chosen) } }
                        }
                    ),
                    asset: { ($0 ?? pair.handsProvider).iconName }
                )
                GlassPickerButton(
                    options: [(String?.none, inherit)] + options.map { (Optional($0.id), $0.shortName) },
                    selection: Binding(
                        get: { profile.model },
                        set: { id in
                            update { profile in
                                profile.model = id
                                // An effort the new model does not offer goes back to the default.
                                if let effort = profile.effort, let option = registry.model(id, for: provider), !option.efforts.contains(effort) {
                                    profile.effort = nil
                                }
                            }
                        }
                    ),
                    maxWidth: 170
                )
                if !efforts.isEmpty {
                    GlassPickerButton(
                        options: [(String?.none, inherit)] + efforts.map { (Optional($0), ModelOption.effortTitle($0)) },
                        selection: Binding(
                            get: { profile.effort },
                            set: { effort in update { $0.effort = effort } }
                        )
                    )
                }
                Spacer(minLength: 0)
            }
            TextField("", text: Binding(
                get: { profile.purpose ?? "" },
                set: { text in update { $0.purpose = text.isEmpty ? nil : text } }
            ), prompt: Text("What it is for: root causes, reviews, design"), axis: .vertical)
            .textFieldStyle(.plain)
            .font(.system(size: 12))
            .foregroundStyle(Chrome.primaryText)
            .lineLimit(1...3)
            if let note {
                Text(verbatim: note)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.warning)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(10)
        .background(Chrome.overlay(0.05), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    /// Why the lead would not reach this profile as it stands, or nil when it would: a
    /// profile with no name, one another profile above already answers for, and on a lead
    /// that starts its hands itself (Claude, Codex, Copilot with the hands on its own
    /// provider) one its agent tools cannot start (see `CrewHandProfile.routesNatively`).
    private var note: String? {
        guard !profile.trimmedName.isEmpty else { return "Name it: the lead routes a task to a profile by its name." }
        if let first = pair.handProfiles.first(where: { $0.clashes(with: profile) }), first.id != profile.id {
            return "\(first.trimmedName) above answers to the same name, so the lead only ever reaches that one."
        }
        guard AppModel.crewIsNative(pair.provider), !pair.sendsHandsElsewhere, !profile.routesNatively(on: pair.handsProvider) else { return nil }
        if let provider = profile.provider, provider != pair.handsProvider {
            return "A \(pair.provider.displayName) lead starts its hands itself, on \(pair.provider.displayName) only, so it cannot send this one out."
        }
        return "Choose a model or an effort: a \(pair.provider.displayName) lead only routes to a profile that changes one."
    }

    /// Changes this profile in its pair, found by its id rather than its place in the list,
    /// which a removal above it may have moved.
    private func update(_ change: (inout CrewHandProfile) -> Void) {
        let id = profile.id
        model.updateCrewPair(pairID) { pair in
            guard let index = pair.handProfiles.firstIndex(where: { $0.id == id }) else { return }
            change(&pair.handProfiles[index])
        }
    }
}
