// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// A physical key and the modifiers held with it.
struct KeyChord: Codable, Hashable, Sendable {
    static let allowedModifiers: NSEvent.ModifierFlags = [.control, .option, .shift, .command]

    var keyCode: Int
    var modifiers: UInt

    init(keyCode: Int, modifiers: NSEvent.ModifierFlags) {
        self.keyCode = keyCode
        self.modifiers = modifiers.intersection(Self.allowedModifiers).rawValue
    }

    var modifierFlags: NSEvent.ModifierFlags {
        NSEvent.ModifierFlags(rawValue: modifiers).intersection(Self.allowedModifiers)
    }

    /// The chord as SwiftUI menus take it, or nil for a key a menu cannot hold.
    var keyboardShortcut: KeyboardShortcut? {
        guard let key = KeyNames.keyEquivalent(for: keyCode) else { return nil }
        var events: EventModifiers = []
        let flags = modifierFlags
        if flags.contains(.control) { events.insert(.control) }
        if flags.contains(.option) { events.insert(.option) }
        if flags.contains(.shift) { events.insert(.shift) }
        if flags.contains(.command) { events.insert(.command) }
        return KeyboardShortcut(key, modifiers: events)
    }

    var description: String {
        KeyNames.modifierSymbols(modifierFlags) + KeyNames.label(for: keyCode)
    }
}

/// Chords answered outside the menus, which `ShortcutStore.refusal` keeps every command off.
extension KeyChord {
    /// Command-Return: the chat box sends with it, and with nothing typed it approves the
    /// request the agent is waiting on. Plain Return adds a line.
    static let composerSend = KeyChord(keyCode: 36, modifiers: .command)
    /// Command-W: closes the windows beside the main one (`SecondaryWindow`). In the main
    /// window it belongs to Archive thread.
    static let closeWindow = KeyChord(keyCode: 13, modifiers: .command)
    /// The Tab key. The main window takes its chords before any menu sees them, for the
    /// thread switches alone (`ThreadWindow`).
    static let tabKeyCode = 48
}

/// Key labels and menu key equivalents for the US key positions.
enum KeyNames {
    static func isModifierKey(_ code: Int) -> Bool {
        [54, 55, 56, 57, 58, 59, 60, 61, 62, 63].contains(code)
    }

    static func modifierSymbols(_ flags: NSEvent.ModifierFlags) -> String {
        var text = ""
        if flags.contains(.control) { text += "⌃" }
        if flags.contains(.option) { text += "⌥" }
        if flags.contains(.shift) { text += "⇧" }
        if flags.contains(.command) { text += "⌘" }
        return text
    }

    private static let characters: [Int: String] = [
        0: "A", 1: "S", 2: "D", 3: "F", 4: "H", 5: "G", 6: "Z", 7: "X", 8: "C", 9: "V", 11: "B", 12: "Q",
        13: "W", 14: "E", 15: "R", 16: "Y", 17: "T", 18: "1", 19: "2", 20: "3", 21: "4", 22: "6", 23: "5",
        24: "=", 25: "9", 26: "7", 27: "-", 28: "8", 29: "0", 30: "]", 31: "O", 32: "U", 33: "[", 34: "I",
        35: "P", 37: "L", 38: "J", 39: "'", 40: "K", 41: ";", 42: "\\", 43: ",", 44: "/", 45: "N", 46: "M",
        47: ".", 50: "`",
    ]

    private static let specialLabels: [Int: String] = [
        36: "Return", 48: "Tab", 49: "Space", 51: "Delete", 53: "Esc", 117: "Del",
        123: "←", 124: "→", 125: "↓", 126: "↑", 115: "Home", 119: "End", 116: "PgUp", 121: "PgDn",
        122: "F1", 120: "F2", 99: "F3", 118: "F4", 96: "F5", 97: "F6", 98: "F7", 100: "F8",
        101: "F9", 109: "F10", 103: "F11", 111: "F12",
    ]

    private static let functionKeys: [Int: Int] = [
        122: 1, 120: 2, 99: 3, 118: 4, 96: 5, 97: 6, 98: 7, 100: 8, 101: 9, 109: 10, 103: 11, 111: 12,
    ]

    static func label(for code: Int) -> String {
        characters[code] ?? specialLabels[code] ?? "Key \(code)"
    }

    /// Keys that type or move the caret, which a chord without Command, Control or Option would steal from text.
    static func typesText(_ code: Int) -> Bool {
        characters[code] != nil || [36, 48, 49, 51, 53, 117, 123, 124, 125, 126].contains(code)
    }

    static func keyEquivalent(for code: Int) -> KeyEquivalent? {
        if let character = characters[code]?.lowercased().first { return KeyEquivalent(character) }
        switch code {
        case 36: return .return
        case 48: return .tab
        case 49: return .space
        case 51: return .delete
        case 53: return .escape
        case 117: return .deleteForward
        case 123: return .leftArrow
        case 124: return .rightArrow
        case 125: return .downArrow
        case 126: return .upArrow
        case 115: return .home
        case 119: return .end
        case 116: return .pageUp
        case 121: return .pageDown
        default:
            guard let number = functionKeys[code], let scalar = UnicodeScalar(NSF1FunctionKey + number - 1) else { return nil }
            return KeyEquivalent(Character(scalar))
        }
    }
}

/// Every command a shortcut can be recorded for, with the chord it ships with.
enum AppShortcut: String, CaseIterable, Identifiable {
    case newThread
    case newWorktreeThread
    case addProject
    case stopTurn
    /// Stops every hand of the selected chat that is still at work; the lead's own turn
    /// goes on.
    case stopAllHands
    case queueChat
    /// Stops the running turn and puts its message back in the chat box, the thread rewound
    /// to before it. Escape used to do this; it has no chord until one is recorded.
    case takeBackTurn
    case togglePlanMode
    case previousThread
    case nextThread
    case runScript
    /// Settles or archives the selected thread, as Settings chose. Stored under its old
    /// name, so a chord recorded for "Archive thread" still applies.
    case finishThread = "archiveThread"
    /// Asks whether to archive the selected thread or delete it.
    case archiveThread = "archiveThreadPrompt"
    /// Settles the selected thread, or reopens one that is settled.
    case settleThread
    case commandPalette
    case toggleSidebar
    case toggleTerminal
    case toggleChanges
    case toggleCrewPanel
    case toggleActivityView
    /// Folds every turn in the chat on screen back to its answer.
    case foldAllTurns
    /// Shows or hides the agent's thinking (`AppSettings.showReasoning`).
    case toggleReasoning
    /// Puts the caret in the sidebar's thread search.
    case focusSearch
    case showMainWindow
    case openSettings
    /// Moves to the next theme, round to the first after the last.
    case cycleTheme
    /// Light and dark in turn, and back to the theme the switch left.
    case toggleAppearance

    enum Section: String, CaseIterable, Identifiable {
        case threads
        case window

        var id: String { rawValue }

        var title: String {
            switch self {
            case .threads: "Threads"
            case .window: "Window and panels"
            }
        }

        var symbol: String {
            switch self {
            case .threads: "bubble.left.and.bubble.right.fill"
            case .window: "macwindow"
            }
        }

        var shortcuts: [AppShortcut] {
            AppShortcut.allCases.filter { $0.section == self }
        }
    }

    var id: String { rawValue }

    var section: Section {
        switch self {
        case .newThread, .newWorktreeThread, .addProject, .stopTurn, .stopAllHands, .queueChat, .takeBackTurn, .togglePlanMode, .previousThread, .nextThread, .runScript, .finishThread, .archiveThread, .settleThread:
            .threads
        case .commandPalette, .toggleSidebar, .toggleTerminal, .toggleChanges, .toggleCrewPanel, .toggleActivityView, .foldAllTurns, .toggleReasoning, .focusSearch, .showMainWindow, .openSettings:
            .window
        case .cycleTheme, .toggleAppearance:
            .window
        }
    }

    var title: String {
        switch self {
        case .newThread: "New thread"
        case .newWorktreeThread: "New thread in worktree"
        case .addProject: "Add project"
        case .stopTurn: "Stop the running turn"
        case .stopAllHands: "Stop all hands"
        case .queueChat: "Queue a chat"
        case .takeBackTurn: "Stop and take back"
        case .togglePlanMode: "Plan mode"
        case .previousThread: "Previous thread"
        case .nextThread: "Next thread"
        case .runScript: "Run top script"
        case .finishThread: "Finish thread"
        case .archiveThread: "Archive thread"
        case .settleThread: "Settle thread"
        case .commandPalette: "Command palette"
        case .toggleSidebar: "Toggle sidebar"
        case .toggleTerminal: "Toggle terminal"
        case .toggleChanges: "Toggle changes"
        case .toggleCrewPanel: "Toggle Crew"
        case .toggleActivityView: "Activity view"
        case .foldAllTurns: "Fold all turns"
        case .toggleReasoning: "Toggle thinking"
        case .focusSearch: "Search threads"
        case .showMainWindow: "Show Bottega"
        case .openSettings: "Settings"
        case .cycleTheme: "Next theme"
        case .toggleAppearance: "Toggle light and dark"
        }
    }

    var detail: String {
        switch self {
        case .newThread: "Starts a thread in the current project."
        case .newWorktreeThread: "Starts a thread in its own git worktree."
        case .addProject: "Opens a folder as a project."
        case .stopTurn: "Interrupts the agent while it works."
        case .stopAllHands: "Stops every Crew hand the chat has at work. The lead's own turn goes on."
        case .queueChat: "While a turn runs, lines the draft up behind it."
        case .takeBackTurn: "Stops the running turn and puts its message back in the chat box. The turn stays in the thread as stopped."
        case .togglePlanMode: "Plans before building."
        case .previousThread: "Selects the thread above in the sidebar."
        case .nextThread: "Selects the thread below in the sidebar."
        case .runScript: "Runs the first project script in the selected thread's terminal."
        case .finishThread: "Settles the selected thread, or archives it, whichever General chose. Reopens a settled one."
        case .archiveThread: "Asks to archive the selected thread; Return archives it, and Delete is a click away."
        case .settleThread: "Settles the selected thread. Reopens one that is settled."
        case .commandPalette: "Searches threads, projects, actions, scripts and commands."
        case .toggleSidebar: "Shows or hides the sidebar."
        case .toggleTerminal: "Opens the right sidebar on the thread's terminal, or closes it."
        case .toggleChanges: "Opens the right sidebar on the thread's changes, or closes it."
        case .toggleCrewPanel: "Opens the right sidebar on the Crew deck, or closes it."
        case .toggleActivityView: "Lists every thread by when it was last active."
        case .foldAllTurns: "Folds every turn in the chat back to its answer."
        case .toggleReasoning: "Shows or hides the agent's thinking."
        case .focusSearch: "Puts the caret in the sidebar's search, opening the sidebar if it is hidden."
        case .showMainWindow: "Brings the main window to the front."
        case .openSettings: "Opens this window."
        case .cycleTheme: "Moves to the next theme: Bottega, System, Light, Dark, then your own once theme.json has been read."
        case .toggleAppearance: "Switches to Light or Dark, whichever the window is not showing, and back to the theme it left."
        }
    }

    var defaultChord: KeyChord? {
        switch self {
        case .newThread: KeyChord(keyCode: 45, modifiers: .command)
        case .newWorktreeThread: KeyChord(keyCode: 45, modifiers: [.shift, .command])
        case .addProject: KeyChord(keyCode: 31, modifiers: .command)
        case .stopTurn: KeyChord(keyCode: 47, modifiers: .command)
        // ⌥⌘. beside Stop's ⌘.: ⇧⌘. is the open panel's own "show hidden files", which a
        // menu command could shadow in the app's folder pickers.
        case .stopAllHands: KeyChord(keyCode: 47, modifiers: [.option, .command])
        // Shift-Command-Return: Command-Return is the chat box's own send (`KeyChord.composerSend`).
        case .queueChat: KeyChord(keyCode: 36, modifiers: [.shift, .command])
        case .takeBackTurn: nil
        case .togglePlanMode: KeyChord(keyCode: 35, modifiers: [.shift, .command])
        case .previousThread: KeyChord(keyCode: 48, modifiers: [.control, .shift])
        case .nextThread: KeyChord(keyCode: 48, modifiers: .control)
        case .runScript: KeyChord(keyCode: 15, modifiers: .command)
        case .finishThread: KeyChord(keyCode: 51, modifiers: [.shift, .command])
        case .archiveThread: KeyChord(keyCode: 13, modifiers: .command)
        case .settleThread: KeyChord(keyCode: 1, modifiers: .command)
        case .commandPalette: KeyChord(keyCode: 40, modifiers: .command)
        case .toggleSidebar: KeyChord(keyCode: 11, modifiers: .command)
        case .toggleTerminal: KeyChord(keyCode: 38, modifiers: .command)
        case .toggleChanges: KeyChord(keyCode: 2, modifiers: .command)
        case .toggleCrewPanel: KeyChord(keyCode: 8, modifiers: [.shift, .command])
        case .toggleActivityView: KeyChord(keyCode: 32, modifiers: [.option, .command])
        case .foldAllTurns: KeyChord(keyCode: 123, modifiers: [.option, .command])
        case .toggleReasoning: KeyChord(keyCode: 17, modifiers: [.option, .command])
        case .focusSearch: KeyChord(keyCode: 3, modifiers: [.shift, .command])
        case .showMainWindow: KeyChord(keyCode: 29, modifiers: .command)
        case .openSettings: KeyChord(keyCode: 43, modifiers: .command)
        // ⌃⌘T and ⌃⌘L, for theme and light: nothing in macOS or the app answers to either.
        case .cycleTheme: KeyChord(keyCode: 17, modifiers: [.control, .command])
        case .toggleAppearance: KeyChord(keyCode: 37, modifiers: [.control, .command])
        }
    }
}

/// The user's shortcuts. Only changed chords are stored; a stored empty chord means the shortcut was cleared.
@MainActor
@Observable
final class ShortcutStore {
    static let shared = ShortcutStore()

    private struct Override: Codable, Hashable {
        var chord: KeyChord?
    }

    private static let defaultsKey = "keyboardShortcuts"

    /// Chords macOS and the app already answer to, which no command may take.
    private static let reserved: [KeyChord] = {
        var chords = [12, 4, 46, 8, 9, 7, 0, 6].map { KeyChord(keyCode: $0, modifiers: .command) }
        chords.append(KeyChord(keyCode: 6, modifiers: [.shift, .command]))
        chords += [18, 19, 20, 21, 23, 22, 26, 28, 25].map { KeyChord(keyCode: $0, modifiers: .command) }
        chords += ShortcutStore.scriptChords
        return chords
    }()

    /// ⌥⌘1 to ⌥⌘9: the first nine scripts, in the order the Workflow menu lists them.
    nonisolated static let scriptChords: [KeyChord] = [18, 19, 20, 21, 23, 22, 26, 28, 25].map {
        KeyChord(keyCode: $0, modifiers: [.option, .command])
    }

    private var overrides: [String: Override] {
        didSet {
            recordedChords = Self.chords(recordedIn: overrides)
            // Recorded chords that did not read, and could not be copied aside, stay stored
            // as they are this run (see `loadProblem`); the chords recorded now apply until quit.
            guard loadProblem?.blocksSaving != true, let data = try? JSONEncoder().encode(overrides) else { return }
            UserDefaults.standard.set(data, forKey: Self.defaultsKey)
        }
    }

    /// Every chord recorded for a command. Derived from `overrides` and changing with it, so
    /// not observed on its own.
    @ObservationIgnored private var recordedChords: Set<KeyChord> = []

    /// What went wrong reading the recorded chords at launch, if anything did: their bytes
    /// were copied into `recovery/` before the next recording could write over them, or, when
    /// even that failed, they are left as they are this run (see `StoredFile.readSetting`).
    @ObservationIgnored private(set) var loadProblem: StorageProblem?

    private init() {
        // Entry by entry: an entry that cannot be read puts its one command back on its
        // default chord, rather than every command the user recorded a chord for. A whole
        // record that does not read is kept aside rather than lost to the next recording.
        let read = StoredFile.readSetting(Self.defaultsKey, subject: "The keyboard shortcuts you recorded") { data in
            try JSONDecoder().decode([String: Lenient<Override>].self, from: data)
        }
        overrides = read.value?.compactMapValues(\.value) ?? [:]
        loadProblem = read.problem
        recordedChords = Self.chords(recordedIn: overrides)
    }

    private static func chords(recordedIn overrides: [String: Override]) -> Set<KeyChord> {
        Set(overrides.values.compactMap(\.chord))
    }

    /// A command's chord: the one recorded for it, else its default. A default the user has
    /// since recorded for another command stands aside, so a command that ships with a chord
    /// in a later build never doubles up one already put to use; the recorder then offers it
    /// as unset.
    func chord(for shortcut: AppShortcut) -> KeyChord? {
        if let override = overrides[shortcut.rawValue] { return override.chord }
        guard let chord = shortcut.defaultChord, !recordedChords.contains(chord) else { return nil }
        return chord
    }

    func keyboardShortcut(for shortcut: AppShortcut) -> KeyboardShortcut? {
        chord(for: shortcut)?.keyboardShortcut
    }

    /// A command's chord as it reads in a label, or nil when the command has none.
    /// One dictionary lookup and a short string: cheap enough for a view body, and
    /// read through the store, so a remap corrects every label that shows it.
    static func label(for shortcut: AppShortcut) -> String? {
        shared.chord(for: shortcut)?.description
    }

    /// The same chord as a tail for a help string: `"Hide terminal" + hint(for: .toggleTerminal)`
    /// reads "Hide terminal (⌘J)", and an empty string once the chord is cleared, so no
    /// help text ever promises a key the app no longer answers to.
    static func hint(for shortcut: AppShortcut) -> String {
        guard let label = label(for: shortcut) else { return "" }
        return " (\(label))"
    }

    func set(_ chord: KeyChord?, for shortcut: AppShortcut) {
        overrides[shortcut.rawValue] = chord == shortcut.defaultChord ? nil : Override(chord: chord)
    }

    func reset(_ shortcut: AppShortcut) {
        overrides[shortcut.rawValue] = nil
    }

    func reset(_ section: AppShortcut.Section) {
        for shortcut in section.shortcuts { overrides[shortcut.rawValue] = nil }
    }

    func isCustomized(_ shortcut: AppShortcut) -> Bool {
        overrides[shortcut.rawValue] != nil
    }

    func hasCustomizations(in section: AppShortcut.Section) -> Bool {
        section.shortcuts.contains(where: isCustomized)
    }

    /// Why a chord cannot be taken by a command, or nil when it can.
    func refusal(for chord: KeyChord, replacing shortcut: AppShortcut) -> String? {
        if chord == self.chord(for: shortcut) { return nil }
        let flags = chord.modifierFlags
        if KeyNames.isModifierKey(chord.keyCode) { return "Add a key to the modifiers." }
        if flags.subtracting(.shift).isEmpty, KeyNames.typesText(chord.keyCode) {
            return "Add ⌘, ⌃ or ⌥ so typing still works."
        }
        if chord.keyboardShortcut == nil { return "Menus cannot use this key." }
        if Self.reserved.contains(chord) { return "macOS or Bottega already uses this chord." }
        // Chords answered outside the menus, which a recorded command would otherwise take
        // or never get: the chat box's send, Command-W closing the windows beside the main
        // one, and the Tab chords the main window keeps for switching threads.
        if chord == .composerSend { return "The chat box sends with this chord." }
        if chord == .closeWindow, shortcut != .archiveThread { return "⌘W is kept for closing windows." }
        if chord.keyCode == KeyChord.tabKeyCode {
            if flags == .command || flags == [.shift, .command] { return "macOS switches apps with this chord." }
            if shortcut != .nextThread, shortcut != .previousThread { return "Only switching threads can use Tab." }
        }
        if let owner = AppShortcut.allCases.first(where: { $0 != shortcut && self.chord(for: $0) == chord }) {
            return "\(owner.title) already uses this chord."
        }
        return nil
    }
}
