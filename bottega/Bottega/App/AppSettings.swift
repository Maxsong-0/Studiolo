// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

enum TextGenerationChoice: String, CaseIterable, Identifiable {
    case automatic
    case claude
    case codex
    case off

    var id: String { rawValue }

    var title: String {
        switch self {
        case .automatic: "Automatic"
        case .claude: "Claude"
        case .codex: "Codex"
        case .off: "Off"
        }
    }
}

/// What the check on a thread row does once you are done with the thread.
enum ThreadFinishAction: String, CaseIterable, Identifiable {
    /// The thread stays in the sidebar, small and grey at the bottom of its list.
    case settle
    /// The thread leaves the sidebar for the Archive page.
    case archive

    var id: String { rawValue }

    var title: String {
        switch self {
        case .settle: "Settle"
        case .archive: "Archive"
        }
    }

    var detail: String {
        switch self {
        case .settle: "A settled thread drops to the bottom of the sidebar, small and grey, until you reopen it"
        case .archive: "An archived thread leaves the sidebar for the Archive page"
        }
    }
}

/// How the thread list is shown: as a column beside the chat, as a floating panel whenever the column is collapsed, or as the floating panel alone.
enum SidebarMode: String, CaseIterable, Identifiable {
    case column
    case floating
    case panelOnly

    var id: String { rawValue }

    var title: String {
        switch self {
        case .column: "Column"
        case .floating: "Floating"
        case .panelOnly: "Panel only"
        }
    }

    var detail: String {
        switch self {
        case .column: "A column beside the chat; the list button in the toolbar shows and hides it"
        case .floating: "With the column collapsed, the thread list floats as a panel over the chat and stays where you leave it"
        case .panelOnly: "No column and no toolbar button; the floating panel is the sidebar"
        }
    }
}

/// How the activity list shows which project a thread belongs to: the project's own mark
/// in front of the thread's name on one line, or the project's name under it.
enum ActivityThreadStyle: String, CaseIterable, Identifiable {
    /// The project's emoji or SF Symbol before the thread's name: one line per thread.
    case icon
    /// The project's name, with a folder mark, on a line under the thread's title.
    case projectName

    var id: String { rawValue }

    var title: String {
        switch self {
        case .icon: "Icon"
        case .projectName: "Project name"
        }
    }

    var detail: String {
        switch self {
        case .icon: "Each project's emoji or symbol before the thread's name, one line per thread"
        case .projectName: "The project's name, with a folder mark, on a line under the thread's title"
        }
    }
}

/// A model the composer's picker offers.
struct ModelPin: Codable, Hashable, Identifiable, Sendable {
    var provider: ProviderKind
    var modelID: String

    var id: String { "\(provider.rawValue)/\(modelID)" }
}

/// How a model starts in new chats.
struct ModelPreference: Codable, Hashable, Sendable {
    var effort: String?
    var fastMode = false
}

/// Preferences that belong to this Mac, stored in user defaults.
@MainActor
@Observable
final class AppSettings {
    /// The per-chat Crew default's key, owned by `ChatThread`, which seeds new
    /// threads from it before any settings object is in reach.
    nonisolated static let crewDefaultEnabledKey = ChatThread.crewDefaultKey

    private enum Key {
        static let defaultProvider = "defaultProvider"
        static let runtimeMode = "defaultRuntimeMode"
        static let workspaceMode = "defaultWorkspaceMode"
        static let notify = "notifyWhenFinished"
        static let chime = "chimeWhenFinished"
        static let confirmDelete = "confirmBeforeDeleting"
        static let autoContinueAfterLimit = "autoContinueAfterLimit"
        static let threadFinishAction = "threadFinishAction"
        static let settleSound = "settleSound"
        static let showReasoning = "showReasoning"
        static let chronologicalTimeline = "chronologicalTimeline"
        static let hidesReadSteps = "hidesReadSteps"
        static let hidesSearchSteps = "hidesSearchSteps"
        static let recentStepsShown = "recentStepsShown"
        static let antiSlopEnabled = "antiSlopEnabled"
        static let showsWorkingCard = "showsWorkingCard"
        /// The index earlier builds stored on their 80–135% scale: read until the percentage
        /// below has been written, and never written itself.
        static let chatZoom = "chatZoom"
        static let textZoomPercent = "textZoomPercent"
        static let textFontFamily = "textFontFamily"
        static let codeFontFamily = "codeFontFamily"
        static let terminalFontSize = "terminalFontSize"
        static let sidebarActivityView = "sidebarActivityView"
        static let activityThreadStyle = "activityThreadStyle"
        static let settledCollapsed = "settledSectionCollapsed"
        static let appTheme = "appTheme"
        static let backdropOpacity = "backdropOpacity"
        static let appearance = "appearance"
        static let binaryPaths = "providerBinaryPaths"
        static let disabledProviders = "disabledProviders"
        static let models = "lastModels"
        static let efforts = "lastEfforts"
        static let lastCrewPair = "lastCrewPairID"
        static let textGeneration = "textGeneration"
        static let commitInstructions = "commitInstructions"
        static let checkpointKeepTurns = "checkpointKeepTurns"
        static let archivedCheckpointsSwept = "archivedCheckpointsSwept"
        static let modelList = "modelList"
        static let modelPreferences = "modelPreferences"
        static let recentDownloadsPicker = "recentDownloadsPicker"
        static let lastProjectID = "lastProjectID"
        static let projectsEnabled = "projectsEnabled"
        static let projectActivationOverrides = "projectActivationOverrides"
        static let deepseekAPIKey = "deepseekAPIKey"
        static let metaAPIKey = "metaAPIKey"
        static let zaiAPIKey = "zaiAPIKey"
        static let commandcodeAPIKey = "commandcodeAPIKey"
        static let crewEnabled = "crewEnabled"
        static let panelSize = "floatingPanelSize"
        static let crewDefaultEnabled = AppSettings.crewDefaultEnabledKey
        static let crewQueueHands = "crewQueueHands"
        static let crewHoldsDelegations = "crewHoldsDelegations"
        static let panelsDockRight = "panelsDockRight"
        static let crewAllowedProjectRoots = "crewAllowedProjectRoots"
        static let crewIsolateHands = "crewIsolateHands"
        static let crewAutoMerge = "crewAutoMerge"
        static let crewMergeMode = "crewMergeMode"
        static let crewHandRuntimeMode = "crewHandRuntimeMode"
        static let crewCommitsLandings = "crewCommitsLandings"
        static let crewReviewHands = "crewReviewHands"
        static let crewStripsVerificationSteps = "crewStripsVerificationSteps"
        static let crewAutoClearFinished = "crewAutoClearFinished"
        static let crewAutoHidesIdleHands = "crewAutoHidesIdleHands"
        static let crewShowsHandDetails = "crewShowsHandDetails"
        static let terminalHeight = "terminalHeight"
        static let userTasks = "userTasks"
        static let crewTempersHandEffort = "crewTempersHandEffort"
        static let crewMaxHands = "crewMaxHands"
        static let crewLimits = "crewLimits"
        static let showsUsagePanel = "showsUsagePanel"
        static let sidebarFloats = "sidebarFloats"
        static let sidebarOnlyFloats = "sidebarOnlyFloats"
        static let sidebarPanelOrigin = "sidebarPanelOrigin"
        static let sidebarPanelSize = "sidebarPanelSize"
        static let usagePanelDock = "usagePanelDock"
        static let usagePanelHeight = "usagePanelHeight"
        static let crewPairs = "crewPairs"
        static let crewLeadInstructions = "crewLeadInstructions"
        static let crewLeadInstructionsFile = "crewLeadInstructionsFile"
        static let studioloVaultPath = "studioloVaultPath"
        static let studioloWebURL = "studioloWebURL"
    }

    static let modelListLimit = 15

    @ObservationIgnored private let defaults = UserDefaults.standard

    /// The Codable settings whose stored bytes did not read at launch, for the app to tell the
    /// user (see `StoredFile.readSetting`): each was copied into `recovery/` before anything
    /// could write over it, or, where the copy failed, is left alone this run.
    @ObservationIgnored private(set) var loadProblems: [StorageProblem] = []
    /// The keys of those whose copy failed: nothing is stored under them this run, so the
    /// bytes that did not read are still there to be read by the next launch.
    @ObservationIgnored private var unreadKeys: Set<String> = []

    /// The provider last chosen in a composer's model picker (or by a Crew pair): what a
    /// new thread starts on when there is no thread to follow. Not a setting of its own
    /// any more; Settings says as much under New threads.
    var defaultProvider: ProviderKind {
        didSet { defaults.set(defaultProvider.rawValue, forKey: Key.defaultProvider) }
    }

    var defaultRuntimeMode: RuntimeMode {
        didSet { defaults.set(defaultRuntimeMode.rawValue, forKey: Key.runtimeMode) }
    }

    var defaultWorkspaceMode: WorkspaceMode {
        didSet { defaults.set(defaultWorkspaceMode.rawValue, forKey: Key.workspaceMode) }
    }

    var notifyWhenFinished: Bool {
        didSet { defaults.set(notifyWhenFinished, forKey: Key.notify) }
    }

    /// A soft chime plays when a turn finishes, whether or not the thread is in view.
    var chimeWhenFinished: Bool {
        didSet { defaults.set(chimeWhenFinished, forKey: Key.chime) }
    }

    var confirmBeforeDeleting: Bool {
        didSet { defaults.set(confirmBeforeDeleting, forKey: Key.confirmDelete) }
    }

    /// A turn that stops on the provider's usage limit is picked back up by itself: the chat
    /// waits for the limit to reset, then tells the agent to continue where it left off.
    var autoContinueAfterLimit: Bool {
        didSet { defaults.set(autoContinueAfterLimit, forKey: Key.autoContinueAfterLimit) }
    }

    /// What the check on a thread row, ⇧⌘⌫ and the palette's finish action do.
    var threadFinishAction: ThreadFinishAction {
        didSet { defaults.set(threadFinishAction.rawValue, forKey: Key.threadFinishAction) }
    }

    /// A soft note plays as a thread settles.
    var settleSound: Bool {
        didSet { defaults.set(settleSound, forKey: Key.settleSound) }
    }

    var showReasoning: Bool {
        didSet { defaults.set(showReasoning, forKey: Key.showReasoning) }
    }

    /// The working line opens with every step in the order it arrived, rather than folding
    /// the turn into its summary. The rows read it through `RowContext.chronological`.
    var chronologicalTimeline: Bool {
        didSet { defaults.set(chronologicalTimeline, forKey: Key.chronologicalTimeline) }
    }

    /// Tool rows the timeline leaves out. Reads and searches are the bulk of a working
    /// turn and rarely what one comes back to read; the work summary still counts them.
    var hidesReadSteps: Bool {
        didSet { defaults.set(hidesReadSteps, forKey: Key.hidesReadSteps) }
    }

    var hidesSearchSteps: Bool {
        didSet { defaults.set(hidesSearchSteps, forKey: Key.hidesSearchSteps) }
    }

    /// How many of a work group's latest steps stay open before the rest fold under one line.
    var recentStepsShown: Int {
        didSet { defaults.set(recentStepsShown, forKey: Key.recentStepsShown) }
    }

    /// The kinds the two switches above take out, as the timeline reads them.
    var hiddenToolKinds: Set<ToolCall.Kind> {
        var kinds: Set<ToolCall.Kind> = []
        if hidesReadSteps { kinds.insert(.read) }
        if hidesSearchSteps { kinds.insert(.search) }
        return kinds
    }

    var antiSlopEnabled: Bool {
        didSet { defaults.set(antiSlopEnabled, forKey: Key.antiSlopEnabled) }
    }

    /// The running turn's line as the hand panel's progress card (the task, the stave bar
    /// of the turn's steps, the time); off, it is the one-line badge with the words and the time.
    var showsWorkingCard: Bool {
        didSet { defaults.set(showsWorkingCard, forKey: Key.showsWorkingCard) }
    }

    /// How large the main window reads: a step of `ChatZoom` (see its percentages), which the
    /// slider in Settings › General moves. The whole window follows it: the conversation, the
    /// sidebar, the chat box, the chrome row, the right sidebar and the panels. Stored as the
    /// step's percentage, so a scale with more steps never reads it as another size.
    var chatZoom: Int {
        didSet { defaults.set(ChatZoom.percentage(at: chatZoom), forKey: Key.textZoomPercent) }
    }

    /// The family the conversation's prose is set in; nil for the system's face.
    var textFontFamily: String? {
        didSet {
            if let textFontFamily {
                defaults.set(textFontFamily, forKey: Key.textFontFamily)
            } else {
                defaults.removeObject(forKey: Key.textFontFamily)
            }
        }
    }

    /// The family the conversation's code blocks and the terminal are set in; nil for the
    /// system's monospaced face.
    var codeFontFamily: String? {
        didSet {
            if let codeFontFamily {
                defaults.set(codeFontFamily, forKey: Key.codeFontFamily)
            } else {
                defaults.removeObject(forKey: Key.codeFontFamily)
            }
        }
    }

    /// The terminal's type at the stock text size, in points; the text size scales it with
    /// the rest of the window (see `TerminalHost`).
    var terminalFontSize: Double {
        didSet { defaults.set(terminalFontSize, forKey: Key.terminalFontSize) }
    }
    static let defaultTerminalFontSize = Double(ChatZoom.terminalBaseSize)
    static let terminalFontSizes: ClosedRange<Double> = 9...24

    /// The two families as the window reads them (see `WindowTypeEnvironment`): a family no
    /// longer installed reads as the system's face.
    var chatTypefaces: ChatTypefaces {
        ChatTypefaces(text: Typefaces.installed(textFontFamily), code: Typefaces.installed(codeFontFamily))
    }

    /// The text size as stored: the percentage this build keeps, else the index an earlier
    /// build kept on its shorter scale, else the stock step.
    private static func storedZoomIndex(_ defaults: UserDefaults) -> Int {
        if let percentage = defaults.object(forKey: Key.textZoomPercent) as? Int {
            return ChatZoom.index(forPercentage: percentage)
        }
        if let legacyIndex = defaults.object(forKey: Key.chatZoom) as? Int {
            return ChatZoom.index(forLegacyIndex: legacyIndex)
        }
        return ChatZoom.defaultIndex
    }

    /// The floating panels' size once one has been resized by hand; nil leaves them to
    /// size themselves to the pane. Every chat's panels share it, and the layout fits it
    /// to the room there is, so a panel never outgrows the pane it is in.
    var panelSize: CGSize? {
        didSet {
            if let panelSize {
                defaults.set([panelSize.width, panelSize.height], forKey: Key.panelSize)
            } else {
                defaults.removeObject(forKey: Key.panelSize)
            }
        }
    }

    /// The sidebar lists every thread by when it was last active, instead of by project.
    var sidebarActivityView: Bool {
        didSet { defaults.set(sidebarActivityView, forKey: Key.sidebarActivityView) }
    }

    /// How the activity list marks each thread's project: the project's own icon before
    /// the title on one line, or the project's name under it. The icon style is the default.
    var activityThreadStyle: ActivityThreadStyle {
        didSet { defaults.set(activityThreadStyle.rawValue, forKey: Key.activityThreadStyle) }
    }

    /// The sidebar's Settled section is folded away. Expanded by default.
    var settledCollapsed: Bool {
        didSet { defaults.set(settledCollapsed, forKey: Key.settledCollapsed) }
    }

    var theme: AppTheme {
        didSet {
            defaults.set(theme.rawValue, forKey: Key.appTheme)
            ThemeManager.current = theme
            // A theme picked any other way ends the light and dark switch's way back.
            if !isTogglingAppearance { themeBeforeToggle = nil }
        }
    }

    /// The theme the light and dark switch left, to go back to (see `toggleAppearance`).
    @ObservationIgnored private var themeBeforeToggle: AppTheme?
    @ObservationIgnored private var isTogglingAppearance = false

    /// How solid the window's backdrop is, 0 (clear glass, the desktop shows through) to
    /// 1 (a solid base). With a wallpaper the backdrop is the picture, drawn at this same
    /// value, so the desktop shows through the picture below the solid end.
    var backdropOpacity: Double {
        didSet { defaults.set(backdropOpacity, forKey: Key.backdropOpacity) }
    }
    static let defaultBackdropOpacity = 0.5

    var textGeneration: TextGenerationChoice {
        didSet { defaults.set(textGeneration.rawValue, forKey: Key.textGeneration) }
    }

    var commitInstructions: String {
        didSet { defaults.set(commitInstructions, forKey: Key.commitInstructions) }
    }

    /// How many of a thread's latest turns keep their file checkpoints, the two hidden git
    /// refs a turn leaves that let an edited message put its files back; nil keeps every
    /// turn's. Older turns let theirs go as each turn finishes, all but the thread's first
    /// start (see `ThreadRuntime.pruneCheckpoints`), and an archived thread lets go of all
    /// of them. Stored as 0 for every turn, so a Mac that never chose reads the default.
    var checkpointKeepTurns: Int? {
        didSet { defaults.set(checkpointKeepTurns ?? 0, forKey: Key.checkpointKeepTurns) }
    }
    static let defaultCheckpointKeepTurns = 20
    static let checkpointKeepTurnsChoices = [10, 20, 50, 100]

    /// Whether the one pass that lets go of the checkpoints still held by threads archived
    /// before archiving took them along has gone over every repository within reach (see
    /// `AppModel.sweepArchivedCheckpoints`). Not a preference: nothing shows it, and once it
    /// is true the pass never runs again.
    var archivedCheckpointsSwept: Bool {
        didSet { defaults.set(archivedCheckpointsSwept, forKey: Key.archivedCheckpointsSwept) }
    }

    /// The attach button offers recent downloads first instead of opening Finder straight away.
    var recentDownloadsPicker: Bool {
        didSet { defaults.set(recentDownloadsPicker, forKey: Key.recentDownloadsPicker) }
    }

    /// The folder a new thread opens in when no thread is selected. Updated whenever a thread
    /// is selected or created, so ⌘N stays in the last folder you used.
    var lastProjectID: UUID? {
        didSet {
            if let lastProjectID {
                defaults.set(lastProjectID.uuidString, forKey: Key.lastProjectID)
            } else {
                defaults.removeObject(forKey: Key.lastProjectID)
            }
        }
    }

    /// Whether projects are active app-wide. This is the default every project follows:
    /// with it on, all projects are active without any per-project step; a project the
    /// user explicitly switched off (or on) keeps its own choice.
    var projectsEnabled: Bool {
        didSet { defaults.set(projectsEnabled, forKey: Key.projectsEnabled) }
    }

    /// Per-project activation choices, by project id. Absent means the project follows
    /// `projectsEnabled`; present means the user explicitly overrode it.
    private(set) var projectActivationOverrides: [String: Bool] {
        didSet { store(projectActivationOverrides, forKey: Key.projectActivationOverrides) }
    }

    /// Whether a project is active: its own explicit choice, else the global switch.
    func isProjectActive(_ id: UUID) -> Bool {
        projectActivationOverrides[id.uuidString] ?? projectsEnabled
    }

    /// Explicitly switch a project on or off, overriding the global switch.
    func setProjectActive(_ active: Bool, for id: UUID) {
        projectActivationOverrides[id.uuidString] = active
    }

    /// Forget a project's own choice, so it follows the global switch again.
    func clearProjectActivationOverride(for id: UUID) {
        projectActivationOverrides[id.uuidString] = nil
    }

    /// Whether a project has its own explicit choice instead of following the switch.
    func hasProjectActivationOverride(for id: UUID) -> Bool {
        projectActivationOverrides[id.uuidString] != nil
    }

    /// DeepSeek talks to its cloud API directly, so it needs an API key instead of a CLI login.
    /// Stored in the Keychain when available, with a UserDefaults fallback for migration.
    /// Read once, on the first ask, and cached: a Keychain query on every render made
    /// Settings lag, and two of them before the window exists made launch wait on securityd.
    var deepseekAPIKeyInput: String {
        didSet {
            guard !isReadingKeychain, deepseekAPIKeyInput != oldValue else { return }
            let kept = DeepSeekKeychain.setAPIKey(deepseekAPIKeyInput)
            storeAPIKeyFallback(kept ? "" : deepseekAPIKeyInput, forKey: Key.deepseekAPIKey)
        }
    }

    /// Meta Model API (Muse Spark) talks to https://api.meta.ai/v1 directly,
    /// so it needs a MODEL_API_KEY instead of a CLI login.
    /// Stored in the Keychain when available, with a UserDefaults fallback for migration.
    var metaAPIKeyInput: String {
        didSet {
            guard !isReadingKeychain, metaAPIKeyInput != oldValue else { return }
            let kept = MetaKeychain.setAPIKey(metaAPIKeyInput)
            storeAPIKeyFallback(kept ? "" : metaAPIKeyInput, forKey: Key.metaAPIKey)
        }
    }

    /// Z.ai's GLM Coding Plan talks to https://api.z.ai/api/coding/paas/v4 directly,
    /// so it needs a ZAI_API_KEY instead of a CLI login.
    /// Stored in the Keychain when available, with a UserDefaults fallback for migration.
    /// Read once, on the first ask, and cached: a Keychain query on every render made
    /// Settings lag, and two of them before the window exists made launch wait on securityd.
    var zaiAPIKeyInput: String {
        didSet {
            guard !isReadingKeychain, zaiAPIKeyInput != oldValue else { return }
            let kept = ZaiKeychain.setAPIKey(zaiAPIKeyInput)
            storeAPIKeyFallback(kept ? "" : zaiAPIKeyInput, forKey: Key.zaiAPIKey)
        }
    }

    /// Command Code signs in with `cmd login`; a Studio API key here is the alternative,
    /// handed to the CLI as `COMMAND_CODE_API_KEY`. Stored in the Keychain when available,
    /// with a UserDefaults fallback for a Mac whose Keychain refused it.
    var commandcodeAPIKeyInput: String {
        didSet {
            guard !isReadingKeychain, commandcodeAPIKeyInput != oldValue else { return }
            let kept = CommandCodeKeychain.setAPIKey(commandcodeAPIKeyInput)
            storeAPIKeyFallback(kept ? "" : commandcodeAPIKeyInput, forKey: Key.commandcodeAPIKey)
        }
    }

    @ObservationIgnored private var didReadKeychain = false
    @ObservationIgnored private var isReadingKeychain = false

    /// One provider's key as read: the Keychain's, or the plaintext copy in the defaults
    /// for a Mac whose Keychain refused it.
    private struct StoredKey: Sendable {
        var value: String
        var inKeychain: Bool

        init(keychain: String, fallback: String) {
            inKeychain = !keychain.isEmpty
            value = inKeychain ? keychain : fallback
        }
    }

    private struct StoredKeys: Sendable {
        var deepseek: StoredKey
        var meta: StoredKey
        var zai: StoredKey
        var commandcode: StoredKey
    }

    private var keyFallbacks: (deepseek: String, meta: String, zai: String, commandcode: String) {
        (
            defaults.string(forKey: Key.deepseekAPIKey) ?? "",
            defaults.string(forKey: Key.metaAPIKey) ?? "",
            defaults.string(forKey: Key.zaiAPIKey) ?? "",
            defaults.string(forKey: Key.commandcodeAPIKey) ?? ""
        )
    }

    /// One Keychain round trip per key.
    private nonisolated static func readKeys(fallbacks: (deepseek: String, meta: String, zai: String, commandcode: String)) -> StoredKeys {
        StoredKeys(
            deepseek: StoredKey(keychain: DeepSeekKeychain.apiKey(fallback: ""), fallback: fallbacks.deepseek),
            meta: StoredKey(keychain: MetaKeychain.apiKey(fallback: ""), fallback: fallbacks.meta),
            zai: StoredKey(keychain: ZaiKeychain.apiKey(fallback: ""), fallback: fallbacks.zai),
            commandcode: StoredKey(keychain: CommandCodeKeychain.apiKey(fallback: ""), fallback: fallbacks.commandcode)
        )
    }

    /// The stored keys, from the Keychain the first time anything asks for one. The fill
    /// bypasses the observers above, which write keys back to the Keychain and the
    /// fallback: the fallback must never receive a key it did not already hold.
    private func readKeychainIfNeeded() {
        guard !didReadKeychain else { return }
        fillKeys(Self.readKeys(fallbacks: keyFallbacks))
    }

    /// Reads the keys off the main thread once the window is up, so neither the first
    /// frame nor the first ask waits on securityd. An ask that comes first reads for itself.
    func loadKeychainInBackground() async {
        guard !didReadKeychain else { return }
        let fallbacks = keyFallbacks
        let keys = await Task.detached(priority: .userInitiated) { Self.readKeys(fallbacks: fallbacks) }.value
        fillKeys(keys)
    }

    private func fillKeys(_ keys: StoredKeys) {
        guard !didReadKeychain else { return }
        didReadKeychain = true
        isReadingKeychain = true
        deepseekAPIKeyInput = keys.deepseek.value
        metaAPIKeyInput = keys.meta.value
        zaiAPIKeyInput = keys.zai.value
        commandcodeAPIKeyInput = keys.commandcode.value
        isReadingKeychain = false
        // Earlier builds kept a plaintext copy of every key in the defaults; one the
        // Keychain holds needs none.
        if keys.deepseek.inKeychain { defaults.removeObject(forKey: Key.deepseekAPIKey) }
        if keys.meta.inKeychain { defaults.removeObject(forKey: Key.metaAPIKey) }
        if keys.zai.inKeychain { defaults.removeObject(forKey: Key.zaiAPIKey) }
        if keys.commandcode.inKeychain { defaults.removeObject(forKey: Key.commandcodeAPIKey) }
    }

    /// The plaintext copy in defaults, for a Mac whose Keychain refused the key; an empty
    /// value removes it, so a key the Keychain holds never sits in the defaults too.
    private func storeAPIKeyFallback(_ value: String, forKey key: String) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            defaults.removeObject(forKey: key)
        } else {
            defaults.set(trimmed, forKey: key)
        }
    }

    private(set) var binaryPaths: [String: String] {
        didSet { defaults.set(binaryPaths, forKey: Key.binaryPaths) }
    }

    private(set) var disabledProviders: [String] {
        didSet { defaults.set(disabledProviders, forKey: Key.disabledProviders) }
    }

    private(set) var lastModels: [String: String] {
        didSet { defaults.set(lastModels, forKey: Key.models) }
    }

    private(set) var lastEfforts: [String: String] {
        didSet { defaults.set(lastEfforts, forKey: Key.efforts) }
    }

    /// The pair a chat was last put in from the model picker, so a new chat with no chat to
    /// carry from (after a relaunch, say) starts in it rather than on the bare last model.
    /// Cleared when a chat leaves its pair or the pair is removed.
    var lastCrewPairID: UUID? {
        didSet { defaults.set(lastCrewPairID?.uuidString, forKey: Key.lastCrewPair) }
    }

    private(set) var modelList: [ModelPin] {
        didSet { store(modelList, forKey: Key.modelList) }
    }

    private(set) var modelPreferences: [String: ModelPreference] {
        didSet { store(modelPreferences, forKey: Key.modelPreferences) }
    }

    /// Crew is on for the app from the first launch: what the app is for. Switched
    /// off in Settings › Crew, or per chat from the mark.
    var crewEnabled: Bool {
        didSet { defaults.set(crewEnabled, forKey: Key.crewEnabled) }
    }

    /// Whether a new chat starts leading a team: "New chats lead a team" in Settings ›
    /// Crew, and only there. A chat's own switch (a right-click on its mark) changes that
    /// chat alone, so a default set off stays off. On until the user turns it off.
    var crewDefaultEnabled: Bool {
        didSet { defaults.set(crewDefaultEnabled, forKey: Key.crewDefaultEnabled) }
    }

    /// The user's own standing rules for every Crew lead, typed in Settings › Crew: a
    /// coordination SOP, house rules. The lead's policy ends with them (see
    /// `crewLeadInstructionsText`). Empty adds nothing.
    var crewLeadInstructions: String {
        didSet { defaults.set(crewLeadInstructions, forKey: Key.crewLeadInstructions) }
    }

    /// A rules file whose contents follow the typed rules, kept wherever the user keeps
    /// them (a CLAUDE.md, a note in a vault), so the leads get its current version rather
    /// than a copy that goes stale. Empty is none. A newly chosen file is read at once.
    var crewLeadInstructionsFile: String {
        didSet {
            defaults.set(crewLeadInstructionsFile, forKey: Key.crewLeadInstructionsFile)
            guard crewLeadInstructionsFile != oldValue else { return }
            if crewLeadInstructionsFileRead != nil { crewLeadInstructionsFileRead = nil }
            Task { await self.refreshCrewLeadInstructionsFile() }
        }
    }

    /// The rules file as it was last read, for the leads and for Settings to show; nil
    /// with no file chosen, and until the chosen one has been read.
    private(set) var crewLeadInstructionsFileRead: CrewLeadInstructionsFile? = nil

    /// What a Crew lead is told after its policy (`CrewLaunch.leadInstructions`): the typed
    /// rules, then the rules file's text, or nil when there is neither. Built from the
    /// file's last read and never from the disk, since every Crew launch is made from this,
    /// some of them in a view's body; the read itself happens before each message a lead
    /// gets (`refreshCrewLeadInstructionsFile`).
    var crewLeadInstructionsText: String? {
        let typed = crewLeadInstructions.trimmingCharacters(in: .whitespacesAndNewlines)
        let file = crewLeadInstructionsFileRead.flatMap { $0.path == crewLeadInstructionsFile ? $0.text : nil }
        let parts = [typed, file].compactMap { $0 }.filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: "\n\n")
    }

    /// Reads the lead's rules file again, off the main thread, and keeps what it holds now;
    /// with no file chosen there is nothing to read. Run before each message a lead gets,
    /// so an edit made since reaches the lead with it, and whenever Settings shows the file.
    func refreshCrewLeadInstructionsFile() async {
        let path = crewLeadInstructionsFile
        guard !path.isEmpty else {
            if crewLeadInstructionsFileRead != nil { crewLeadInstructionsFileRead = nil }
            return
        }
        let read = await Task.detached(priority: .userInitiated) { CrewLeadInstructionsFile.read(path) }.value
        // Another file may have been chosen while this one was being read.
        guard path == crewLeadInstructionsFile, read != crewLeadInstructionsFileRead else { return }
        crewLeadInstructionsFileRead = read
    }

    /// With Crew on, a follow-up queued while a turn runs goes to a hand at once
    /// instead of waiting for the turn.
    /// Hands wait for the user's go-ahead instead of leaving the moment the lead asks for
    /// them. The lead is told to state the hand count and the rough cost first; this is
    /// what makes that a gate rather than a courtesy. `/go` releases them.
    var crewHoldsDelegations: Bool {
        didSet { defaults.set(crewHoldsDelegations, forKey: Key.crewHoldsDelegations) }
    }

    /// Every floating panel (the team's, a popped-out hand's, a helper's, the usage panel)
    /// stacks down the right of the chat instead of floating in the corner it was dragged
    /// to. The panel machinery is the same; only the corner every panel resolves to changes.
    var panelsDockRight: Bool {
        didSet { defaults.set(panelsDockRight, forKey: Key.panelsDockRight) }
    }

    var crewQueueHands: Bool {
        didSet { defaults.set(crewQueueHands, forKey: Key.crewQueueHands) }
    }

    /// Folders outside the sidebar that hands may be sent into, as standardized absolute
    /// paths (Settings › Crew): a lead naming a git repository that lies in one of them
    /// adds it to the sidebar and sends the hand there (see `AppModel.crewProject`), and
    /// the lead's policy lists them (`CrewLaunch.allowedProjectRoots`). Empty, the
    /// default, keeps hands to the projects already in the sidebar.
    var crewAllowedProjectRoots: [String] {
        didSet { defaults.set(crewAllowedProjectRoots, forKey: Key.crewAllowedProjectRoots) }
    }

    /// A hand Bottega runs gets a copy of the checkout of its own, so no hand ever
    /// sees another's half-done work; its changes land in the chat's checkout when it
    /// reports. Off, the hands work in the checkout itself.
    var crewIsolateHands: Bool {
        didSet { defaults.set(crewIsolateHands, forKey: Key.crewIsolateHands) }
    }

    /// The cap on hands at work at once for every chat, set on the Crew page; nil is
    /// off. A pair's own cap still counts where it is lower. A Mac that never chose starts
    /// at `defaultCrewMaxHands`, so no job fans out wider than that unasked; "No cap" is
    /// kept as a 0, which tells it apart from never having chosen.
    var crewMaxHands: Int? {
        didSet { defaults.set(crewMaxHands ?? 0, forKey: Key.crewMaxHands) }
    }
    static let defaultCrewMaxHands = 4

    /// The numbers the team runs to (Settings › Crew › Limits): rounds of hands per
    /// request, tasks per block, hands at once with no cap, the budget of a hand
    /// Bottega runs, and the watchdog over those hands.
    var crewLimits: CrewLimits {
        didSet { store(crewLimits, forKey: Key.crewLimits) }
    }

    /// Once the lead has finished and every hand is back, the team's work goes out as
    /// `crewMergeMode` says: by default as a commit in the checkout alone. Off unless asked.
    var crewAutoMerge: Bool {
        didSet { defaults.set(crewAutoMerge, forKey: Key.crewAutoMerge) }
    }

    /// What the auto-merge does with the team's work: a commit in the checkout alone, which
    /// needs no remote (the default: the user's repositories often have none, and their CI
    /// rule is to batch work into one push rather than open a request per job), a branch
    /// that is only pushed, or a merge request that is opened and merged (upstream's way).
    var crewMergeMode: CrewMergeMode {
        didSet { defaults.set(crewMergeMode.rawValue, forKey: Key.crewMergeMode) }
    }

    /// The most a hand Bottega runs as a thread of its own may work under (a native
    /// hand lives in its lead's session and takes the lead's). A ceiling, not a grant: the
    /// lead's own mode lowers it, and a plan lead's hands plan under Supervised (see
    /// `CrewHandAuthority`). Nobody sits at a hand's chat to answer a prompt, so anything
    /// short of full access lets it ask only in its card, and a hand left waiting there is
    /// stopped by the watchdog: Auto and Auto-accept edits trade that for keeping a hand's
    /// shell on a lead. Never offered as Supervised here, where its first tool would wait.
    var crewHandRuntimeMode: RuntimeMode {
        didSet { defaults.set(crewHandRuntimeMode.rawValue, forKey: Key.crewHandRuntimeMode) }
    }
    static let crewHandRuntimeModes: [RuntimeMode] = [.fullAccess, .auto, .autoAcceptEdits]

    /// A hand's work that lands cleanly in the checkout is committed there at once (see
    /// `AppModel.commitCrewLanding`), so a job stopped half way loses none of it. Off
    /// unless asked: it leaves a commit per hand on the checkout's branch.
    var crewCommitsLandings: Bool {
        didSet { defaults.set(crewCommitsLandings, forKey: Key.crewCommitsLandings) }
    }

    /// Whether clean landings are committed now: the switch, unless the auto-merge is about
    /// to build a merge request or a pushed branch out of the uncommitted work, which the
    /// commits would take from it (the merge would find the work already committed and
    /// send nothing). With a local-commit merge the two go together.
    var crewCommitsLandingsInEffect: Bool {
        crewCommitsLandings && (!crewAutoMerge || !crewMergeMode.pushes)
    }

    /// The lead gives every hand's landed work a quick audit before it finishes, reading
    /// the files the reports name and correcting what is wrong itself. On unless switched
    /// off: the user's own rules call nothing done before it has been read and built. The
    /// lead's one build of the whole change does not hang on it (see `CrewPrompts`).
    var crewReviewHands: Bool {
        didSet { defaults.set(crewReviewHands, forKey: Key.crewReviewHands) }
    }

    /// Every check stays with the lead: a line of a hosted hand's brief asking it to
    /// build, test or verify is taken out before the hand sees it, and the lead's chat
    /// names it (see `CrewPrompts.verificationSteps(in:)`). On unless switched off, the rule
    /// the user's projects keep: hands make the edits, the lead runs the one build. Off,
    /// the brief goes out whole and a hand runs the check it names, if that check writes
    /// no build output.
    var crewStripsVerificationSteps: Bool {
        didSet { defaults.set(crewStripsVerificationSteps, forKey: Key.crewStripsVerificationSteps) }
    }

    /// A hand that finishes leaves the Crew panel on its own, for the sidebar under its
    /// lead, instead of waiting for "Clear finished hands".
    var crewAutoClearFinished: Bool {
        didSet { defaults.set(crewAutoClearFinished, forKey: Key.crewAutoClearFinished) }
    }

    /// Finished hands leave the sidebar with their card: a chat's hands show there while one
    /// of them runs, and the card goes once every hand of that chat has finished. Off by
    /// default, so the hands stay in the sidebar until it is asked for.
    var crewAutoHidesIdleHands: Bool {
        didSet { defaults.set(crewAutoHidesIdleHands, forKey: Key.crewAutoHidesIdleHands) }
    }

    /// A hand's panel shows its whole conversation instead of the progress bar. Off unless
    /// asked: the bar says how far it is and how long it took, the steps are the sidebar's
    /// to show.
    var crewShowsHandDetails: Bool {
        didSet { defaults.set(crewShowsHandDetails, forKey: Key.crewShowsHandDetails) }
    }

    /// The terminal drawer's height, in points; the divider writes it on release.
    var terminalHeight: Double {
        didSet { defaults.set(terminalHeight, forKey: Key.terminalHeight) }
    }

    /// The reader's own list, on the right sidebar's task page. Kept for the app rather
    /// than a thread: it is a list of what to do next, not of what one chat did.
    var userTasks: [UserTask] {
        didSet { store(userTasks, forKey: Key.userTasks) }
    }

    /// Hands think at a working effort rather than the lead's own: a lead above medium
    /// sends out medium hands on the same model (a scale without a medium works out at
    /// its middle rung), unless the pair names an effort for
    /// them (see `AppModel.crewHandsEffort`). Off unless asked, so a lead thinking at max
    /// does not send its hands out at medium; a pair or one of its hand profiles that
    /// names an effort keeps it either way.
    var crewTempersHandEffort: Bool {
        didSet { defaults.set(crewTempersHandEffort, forKey: Key.crewTempersHandEffort) }
    }

    /// A floating panel with the chat's usage, the plan's limits and credits, beside
    /// every chat for as long as it is on: the General setting, the popover's pop-out
    /// button and the panel's own close button all set it, so it stays across threads
    /// and relaunches.
    var showsUsagePanel: Bool {
        didSet { defaults.set(showsUsagePanel, forKey: Key.showsUsagePanel) }
    }

    /// The sidebar as a floating panel over the chat whenever its column is collapsed, in
    /// place of the popover from the toolbar button; it stays where it was last left.
    var sidebarFloats: Bool {
        didSet { defaults.set(sidebarFloats, forKey: Key.sidebarFloats) }
    }

    /// Only the floating panel: the column and the toolbar button are gone; meaningful
    /// with `sidebarFloats` on.
    var sidebarOnlyFloats: Bool {
        didSet { defaults.set(sidebarOnlyFloats, forKey: Key.sidebarOnlyFloats) }
    }

    /// The two sidebar switches as one choice: the settings page offers Column, Floating and Panel only.
    var sidebarMode: SidebarMode {
        get {
            guard sidebarFloats else { return .column }
            return sidebarOnlyFloats ? .panelOnly : .floating
        }
        set {
            sidebarFloats = newValue != .column
            sidebarOnlyFloats = newValue == .panelOnly
        }
    }

    /// Where the floating sidebar was last left, from the chat area's top-left.
    var sidebarPanelOrigin: CGPoint? {
        didSet {
            if let sidebarPanelOrigin {
                defaults.set([sidebarPanelOrigin.x, sidebarPanelOrigin.y], forKey: Key.sidebarPanelOrigin)
            } else {
                defaults.removeObject(forKey: Key.sidebarPanelOrigin)
            }
        }
    }

    /// The size it was last dragged to.
    var sidebarPanelSize: CGSize? {
        didSet {
            if let sidebarPanelSize {
                defaults.set([sidebarPanelSize.width, sidebarPanelSize.height], forKey: Key.sidebarPanelSize)
            } else {
                defaults.removeObject(forKey: Key.sidebarPanelSize)
            }
        }
    }

    /// The corner the usage panel is docked in, for every chat; it goes where it was
    /// last dragged.
    var usagePanelDock: PanelDockCorner {
        didSet { defaults.set(usagePanelDock.rawValue, forKey: Key.usagePanelDock) }
    }

    /// The height the usage panel was dragged to, for every chat; nil fits the panel to its rows.
    var usagePanelHeight: CGFloat? {
        didSet {
            if let usagePanelHeight {
                defaults.set(Double(usagePanelHeight), forKey: Key.usagePanelHeight)
            } else {
                defaults.removeObject(forKey: Key.usagePanelHeight)
            }
        }
    }

    /// The lead-and-hands pairings, in the order they were added.
    private(set) var crewPairs: [CrewPair] {
        didSet { store(crewPairs, forKey: Key.crewPairs) }
    }

    /// The Studiolo vault on this Mac, whose `项目/` archives the sidebar shows beside the
    /// projects. A folder without `项目/` turns the whole feature off, quietly. Change it
    /// through `AppModel.setStudioloVault`, which also moves the watch.
    var studioloVaultPath: String {
        didSet { defaults.set(studioloVaultPath, forKey: Key.studioloVaultPath) }
    }
    nonisolated static let defaultStudioloVaultPath = NSHomeDirectory() + "/Documents/Max's Secret Place"

    /// Where the Studiolo web app is, for the archive card's Open in Studiolo.
    var studioloWebURL: String {
        didSet { defaults.set(studioloWebURL, forKey: Key.studioloWebURL) }
    }
    nonisolated static let defaultStudioloWebURL = "https://studiolo.maxsong.me"

    init() {
        let defaults = UserDefaults.standard
        // The Codable settings that did not read (see `load`), gathered as they are read.
        var unread = UnreadSettings()
        defaultProvider = ProviderKind(rawValue: defaults.string(forKey: Key.defaultProvider) ?? "") ?? .codex
        defaultRuntimeMode = RuntimeMode(rawValue: defaults.string(forKey: Key.runtimeMode) ?? "") ?? .fullAccess
        defaultWorkspaceMode = WorkspaceMode(rawValue: defaults.string(forKey: Key.workspaceMode) ?? "") ?? .local
        notifyWhenFinished = defaults.object(forKey: Key.notify) as? Bool ?? true
        chimeWhenFinished = defaults.object(forKey: Key.chime) as? Bool ?? true
        confirmBeforeDeleting = defaults.object(forKey: Key.confirmDelete) as? Bool ?? true
        autoContinueAfterLimit = defaults.object(forKey: Key.autoContinueAfterLimit) as? Bool ?? false
        threadFinishAction = ThreadFinishAction(rawValue: defaults.string(forKey: Key.threadFinishAction) ?? "") ?? .settle
        settleSound = defaults.object(forKey: Key.settleSound) as? Bool ?? true
        showReasoning = defaults.object(forKey: Key.showReasoning) as? Bool ?? false
        chronologicalTimeline = defaults.object(forKey: Key.chronologicalTimeline) as? Bool ?? false
        hidesReadSteps = defaults.object(forKey: Key.hidesReadSteps) as? Bool ?? true
        hidesSearchSteps = defaults.object(forKey: Key.hidesSearchSteps) as? Bool ?? true
        recentStepsShown = min(10, max(1, defaults.object(forKey: Key.recentStepsShown) as? Int ?? 2))
        antiSlopEnabled = defaults.object(forKey: Key.antiSlopEnabled) as? Bool ?? false
        showsWorkingCard = defaults.object(forKey: Key.showsWorkingCard) as? Bool ?? true
        chatZoom = Self.storedZoomIndex(defaults)
        textFontFamily = defaults.string(forKey: Key.textFontFamily)
        codeFontFamily = defaults.string(forKey: Key.codeFontFamily)
        terminalFontSize = min(Self.terminalFontSizes.upperBound, max(Self.terminalFontSizes.lowerBound, defaults.object(forKey: Key.terminalFontSize) as? Double ?? Self.defaultTerminalFontSize))
        sidebarActivityView = defaults.bool(forKey: Key.sidebarActivityView)
        activityThreadStyle = ActivityThreadStyle(rawValue: defaults.string(forKey: Key.activityThreadStyle) ?? "") ?? .icon
        settledCollapsed = defaults.object(forKey: Key.settledCollapsed) as? Bool ?? true
        backdropOpacity = defaults.object(forKey: Key.backdropOpacity) as? Double ?? Self.defaultBackdropOpacity
        // A theme only an earlier build had (the named palettes that were dropped) falls back
        // to the default and is written back, so the stale name goes and every other setting
        // stays as it was. The old System/Light/Dark appearance, from before there were themes,
        // still maps straight onto the same themes.
        let storedTheme = defaults.string(forKey: Key.appTheme)
        let initialTheme = storedTheme.map { AppTheme(rawValue: $0) ?? .bottega }
            ?? AppTheme(rawValue: defaults.string(forKey: Key.appearance) ?? "")
            ?? .bottega
        if let storedTheme, storedTheme != initialTheme.rawValue {
            defaults.set(initialTheme.rawValue, forKey: Key.appTheme)
        }
        // The custom theme's file is read before the theme is applied, so a launch in it
        // starts in its colours rather than the brass it shows until then.
        CustomThemeStore.shared.start()
        theme = initialTheme
        ThemeManager.current = initialTheme
        textGeneration = TextGenerationChoice(rawValue: defaults.string(forKey: Key.textGeneration) ?? "") ?? .automatic
        commitInstructions = defaults.string(forKey: Key.commitInstructions) ?? ""
        recentDownloadsPicker = defaults.object(forKey: Key.recentDownloadsPicker) as? Bool ?? true
        let keptTurns = defaults.object(forKey: Key.checkpointKeepTurns) as? Int ?? Self.defaultCheckpointKeepTurns
        checkpointKeepTurns = keptTurns > 0 ? keptTurns : nil
        archivedCheckpointsSwept = defaults.object(forKey: Key.archivedCheckpointsSwept) as? Bool ?? false
        if let stored = defaults.string(forKey: Key.lastProjectID) {
            lastProjectID = UUID(uuidString: stored)
        } else {
            lastProjectID = nil
        }
        projectsEnabled = defaults.object(forKey: Key.projectsEnabled) as? Bool ?? true
        projectActivationOverrides = Self.load([String: Bool].self, forKey: Key.projectActivationOverrides, subject: "The projects switched on and off", unread: &unread) ?? [:]
        // Read from the Keychain later (see `loadKeychainInBackground`), not before the first frame.
        deepseekAPIKeyInput = ""
        metaAPIKeyInput = ""
        zaiAPIKeyInput = ""
        commandcodeAPIKeyInput = ""
        binaryPaths = defaults.dictionary(forKey: Key.binaryPaths) as? [String: String] ?? [:]
        disabledProviders = defaults.stringArray(forKey: Key.disabledProviders) ?? []
        lastModels = defaults.dictionary(forKey: Key.models) as? [String: String] ?? [:]
        lastEfforts = defaults.dictionary(forKey: Key.efforts) as? [String: String] ?? [:]
        lastCrewPairID = defaults.string(forKey: Key.lastCrewPair).flatMap(UUID.init(uuidString:))
        // Element by element, like the pairs below: a pin naming a provider this build no
        // longer has, or a preference that cannot be read, goes alone rather than emptying
        // its list.
        modelList = Self.load([Lenient<ModelPin>].self, forKey: Key.modelList, subject: "The model picker's list", unread: &unread)?.compactMap(\.value) ?? []
        modelPreferences = Self.load([String: Lenient<ModelPreference>].self, forKey: Key.modelPreferences, subject: "The models' own preferences", unread: &unread)?.compactMapValues(\.value) ?? [:]
        crewEnabled = defaults.object(forKey: Key.crewEnabled) as? Bool ?? true
        crewDefaultEnabled = defaults.object(forKey: Key.crewDefaultEnabled) as? Bool ?? true
        crewLeadInstructions = defaults.string(forKey: Key.crewLeadInstructions) ?? ""
        // Read on the first message a lead gets, or when Settings shows it, not at launch.
        crewLeadInstructionsFile = defaults.string(forKey: Key.crewLeadInstructionsFile) ?? ""
        if let stored = defaults.array(forKey: Key.panelSize) as? [Double], stored.count == 2 {
            panelSize = CGSize(width: stored[0], height: stored[1])
        }
        crewQueueHands = defaults.object(forKey: Key.crewQueueHands) as? Bool ?? true
        crewHoldsDelegations = defaults.object(forKey: Key.crewHoldsDelegations) as? Bool ?? true
        panelsDockRight = defaults.object(forKey: Key.panelsDockRight) as? Bool ?? true
        crewAllowedProjectRoots = defaults.stringArray(forKey: Key.crewAllowedProjectRoots) ?? []
        crewIsolateHands = defaults.object(forKey: Key.crewIsolateHands) as? Bool ?? true
        // Nothing stored is a Mac that never chose; 0 is its "No cap".
        if let storedCap = defaults.object(forKey: Key.crewMaxHands) as? Int {
            crewMaxHands = storedCap > 0 ? CrewPair.clampedCap(storedCap) : nil
        } else {
            crewMaxHands = Self.defaultCrewMaxHands
        }
        crewLimits = Self.load(CrewLimits.self, forKey: Key.crewLimits, subject: "The Crew limits", unread: &unread) ?? .default
        crewAutoMerge = defaults.object(forKey: Key.crewAutoMerge) as? Bool ?? false
        crewMergeMode = CrewMergeMode(rawValue: defaults.string(forKey: Key.crewMergeMode) ?? "") ?? .localCommit
        crewHandRuntimeMode = RuntimeMode(rawValue: defaults.string(forKey: Key.crewHandRuntimeMode) ?? "")
            .flatMap { Self.crewHandRuntimeModes.contains($0) ? $0 : nil } ?? .fullAccess
        crewCommitsLandings = defaults.object(forKey: Key.crewCommitsLandings) as? Bool ?? false
        crewReviewHands = defaults.object(forKey: Key.crewReviewHands) as? Bool ?? true
        crewStripsVerificationSteps = defaults.object(forKey: Key.crewStripsVerificationSteps) as? Bool ?? true
        crewAutoClearFinished = defaults.object(forKey: Key.crewAutoClearFinished) as? Bool ?? false
        crewAutoHidesIdleHands = defaults.object(forKey: Key.crewAutoHidesIdleHands) as? Bool ?? false
        crewShowsHandDetails = defaults.object(forKey: Key.crewShowsHandDetails) as? Bool ?? false
        terminalHeight = defaults.object(forKey: Key.terminalHeight) as? Double ?? 260
        userTasks = Self.load([Lenient<UserTask>].self, forKey: Key.userTasks, subject: "Your task list", unread: &unread)?.compactMap(\.value) ?? []
        crewTempersHandEffort = defaults.object(forKey: Key.crewTempersHandEffort) as? Bool ?? false
        showsUsagePanel = defaults.object(forKey: Key.showsUsagePanel) as? Bool ?? false
        sidebarFloats = defaults.object(forKey: Key.sidebarFloats) as? Bool ?? false
        sidebarOnlyFloats = defaults.object(forKey: Key.sidebarOnlyFloats) as? Bool ?? false
        if let stored = defaults.array(forKey: Key.sidebarPanelOrigin) as? [Double], stored.count == 2 {
            sidebarPanelOrigin = CGPoint(x: stored[0], y: stored[1])
        } else {
            sidebarPanelOrigin = nil
        }
        if let stored = defaults.array(forKey: Key.sidebarPanelSize) as? [Double], stored.count == 2 {
            sidebarPanelSize = CGSize(width: stored[0], height: stored[1])
        } else {
            sidebarPanelSize = nil
        }
        usagePanelDock = defaults.string(forKey: Key.usagePanelDock).flatMap(PanelDockCorner.init(rawValue:)) ?? .bottomLeading
        usagePanelHeight = defaults.object(forKey: Key.usagePanelHeight).flatMap { $0 as? Double }.map { CGFloat($0) }
        crewPairs = Self.load([Lenient<CrewPair>].self, forKey: Key.crewPairs, subject: "The Crew pairs", unread: &unread)?.compactMap(\.value) ?? []
        studioloVaultPath = defaults.string(forKey: Key.studioloVaultPath) ?? Self.defaultStudioloVaultPath
        studioloWebURL = defaults.string(forKey: Key.studioloWebURL) ?? Self.defaultStudioloWebURL
        loadProblems = unread.problems
        unreadKeys = unread.keys
    }

    // MARK: - Theme switches

    /// The next theme in the menus' order, round to the first after the last: the View menu's
    /// Next theme, its shortcut and the command palette's row.
    func cycleTheme() {
        theme = theme.next(in: AppTheme.available(current: theme))
    }

    /// Light and dark in turn: the built-in Light or Dark, whichever the window is not
    /// showing. A theme of any other kind is kept aside as the switch leaves it, and the
    /// switch goes back to it once that theme would show the other side again, so the brass
    /// default is one press back rather than a trip to Settings. With only four themes built
    /// in there is nothing left to pair per side: System already is Light and Dark following
    /// the Mac, and a theme of one's own does it with its own `light` and `dark` blocks.
    func toggleAppearance() {
        let macIsDark = AppTheme.macIsDark
        let showsDark = theme.showsDark(macIsDark: macIsDark)
        isTogglingAppearance = true
        defer { isTogglingAppearance = false }
        if theme == .light || theme == .dark,
           let previous = themeBeforeToggle,
           previous.showsDark(macIsDark: macIsDark) != showsDark {
            theme = previous
            themeBeforeToggle = nil
            return
        }
        if theme != .light, theme != .dark { themeBeforeToggle = theme }
        theme = showsDark ? .light : .dark
    }

    // MARK: - Crew pairs

    func addCrewPair(_ pair: CrewPair) {
        crewPairs.append(pair)
    }

    func updateCrewPair(_ id: UUID, _ change: (inout CrewPair) -> Void) {
        guard let index = crewPairs.firstIndex(where: { $0.id == id }) else { return }
        var pair = crewPairs[index]
        change(&pair)
        pair.maxHands = CrewPair.clampedCap(pair.maxHands)
        guard pair != crewPairs[index] else { return }
        crewPairs[index] = pair
    }

    func removeCrewPair(_ id: UUID) {
        crewPairs.removeAll { $0.id == id }
    }

    func crewPair(_ id: UUID?) -> CrewPair? {
        guard let id else { return nil }
        return crewPairs.first { $0.id == id }
    }

    /// The pair a chat leads with: the one for its provider whose lead model is the chat's,
    /// else one for any model on the provider, else the provider's first pair.
    func crewPair(for provider: ProviderKind, model: String?) -> CrewPair? {
        let candidates = crewPairs.filter { $0.provider == provider }
        if let model, let exact = candidates.first(where: { $0.orchestratorModel == model }) { return exact }
        if let any = candidates.first(where: { $0.orchestratorModel == nil }) { return any }
        return candidates.first
    }

    // MARK: - Model picker

    func isInModelList(_ pin: ModelPin) -> Bool {
        modelList.contains(pin)
    }

    func addToModelList(_ pin: ModelPin) {
        guard !modelList.contains(pin), modelList.count < Self.modelListLimit else { return }
        modelList.append(pin)
    }

    func removeFromModelList(_ pin: ModelPin) {
        modelList.removeAll { $0 == pin }
    }

    /// Moves `pin` to sit right before or after `target`; returns whether the list changed.
    @discardableResult
    func moveModel(_ pin: ModelPin, to target: ModelPin, placeAfter: Bool) -> Bool {
        guard pin != target,
              let from = modelList.firstIndex(of: pin),
              let to = modelList.firstIndex(of: target) else { return false }
        var dest = to + (placeAfter ? 1 : 0)
        if from < dest { dest -= 1 }
        guard from != dest else { return false }
        let moved = modelList.remove(at: from)
        modelList.insert(moved, at: dest)
        return true
    }

    func preference(for provider: ProviderKind, model: String?) -> ModelPreference {
        guard let model else { return ModelPreference() }
        return modelPreferences[ModelPin(provider: provider, modelID: model).id] ?? ModelPreference()
    }

    func setPreference(_ preference: ModelPreference, for provider: ProviderKind, model: String) {
        let key = ModelPin(provider: provider, modelID: model).id
        modelPreferences[key] = preference == ModelPreference() ? nil : preference
    }

    /// Writes a Codable setting, unless its stored bytes did not read at launch and could not
    /// be copied aside (`unreadKeys`): those stay as they are this run, for the next launch.
    private func store<Value: Encodable>(_ value: Value, forKey key: String) {
        guard !unreadKeys.contains(key) else { return }
        if let data = try? JSONEncoder().encode(value) { defaults.set(data, forKey: key) }
    }

    /// What `load` found wrong at launch: the problems to tell the user, and the keys `store`
    /// leaves alone this run.
    private struct UnreadSettings {
        var problems: [StorageProblem] = []
        var keys: Set<String> = []
    }

    /// Reads a Codable setting. Nothing stored reads as nil, the setting's default. Bytes that
    /// do not decode read as nil too, but are not lost to the next write: they are copied into
    /// `recovery/` first, and when that fails the key goes into `unread` and is not written
    /// over this run (see `StoredFile.readSetting`). `subject` names the setting for the user.
    private static func load<Value: Decodable>(_ type: Value.Type, forKey key: String, subject: String, unread: inout UnreadSettings) -> Value? {
        let read = StoredFile.readSetting(key, subject: subject) { data in
            try JSONDecoder().decode(type, from: data)
        }
        if let problem = read.problem {
            unread.problems.append(problem)
            if problem.blocksSaving { unread.keys.insert(key) }
        }
        return read.value
    }

    func binaryPath(for provider: ProviderKind) -> String {
        binaryPaths[provider.rawValue] ?? ""
    }

    func setBinaryPath(_ path: String, for provider: ProviderKind) {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        binaryPaths[provider.rawValue] = trimmed.isEmpty ? nil : trimmed
    }

    func isEnabled(_ provider: ProviderKind) -> Bool {
        !disabledProviders.contains(provider.rawValue)
    }

    func setEnabled(_ enabled: Bool, for provider: ProviderKind) {
        disabledProviders.removeAll { $0 == provider.rawValue }
        if !enabled { disabledProviders.append(provider.rawValue) }
    }

    func lastModel(for provider: ProviderKind) -> String? {
        lastModels[provider.rawValue]
    }

    func lastEffort(for provider: ProviderKind) -> String? {
        lastEfforts[provider.rawValue]
    }

    func remember(model: String?, effort: String?, for provider: ProviderKind) {
        lastModels[provider.rawValue] = model
        lastEfforts[provider.rawValue] = effort
    }

    /// The API key Bottega sends to an API-key provider: the value from Settings,
    /// falling back to the provider's env var from the login environment
    /// (`DEEPSEEK_API_KEY` for DeepSeek, `MODEL_API_KEY` for Meta). Command Code's is
    /// optional: the CLI's own `cmd login` serves when it is empty.
    func apiKey(for provider: ProviderKind) -> String {
        readKeychainIfNeeded()
        let stored: String = switch provider {
        case .deepseek: deepseekAPIKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        case .meta: metaAPIKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        case .zai: zaiAPIKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        case .commandcode: commandcodeAPIKeyInput.trimmingCharacters(in: .whitespacesAndNewlines)
        default: ""
        }
        if !stored.isEmpty { return stored }
        guard provider.acceptsAPIKey, let envVar = provider.apiKeyEnvVar else { return "" }
        return (LoginEnvironment.current[envVar] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func hasAPIKey(for provider: ProviderKind) -> Bool {
        !apiKey(for: provider).isEmpty
    }

    /// Settings-bound value for the Providers page, per API-key provider.
    func apiKeyInput(for provider: ProviderKind) -> String {
        readKeychainIfNeeded()
        return switch provider {
        case .deepseek: deepseekAPIKeyInput
        case .meta: metaAPIKeyInput
        case .zai: zaiAPIKeyInput
        case .commandcode: commandcodeAPIKeyInput
        default: ""
        }
    }

    func setAPIKeyInput(_ value: String, for provider: ProviderKind) {
        readKeychainIfNeeded()
        switch provider {
        case .commandcode: commandcodeAPIKeyInput = value
        case .deepseek: deepseekAPIKeyInput = value
        case .meta: metaAPIKeyInput = value
        case .zai: zaiAPIKeyInput = value
        default: break
        }
    }
}

/// The registry's view of the settings; the methods are the ones above.
extension AppSettings: ProviderSettings {}
