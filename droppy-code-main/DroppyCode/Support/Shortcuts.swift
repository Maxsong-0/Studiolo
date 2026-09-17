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

    static func isFunctionKey(_ code: Int) -> Bool {
        functionKeys[code] != nil
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
    case queueChat
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
    case toggleActivityView
    case showMainWindow
    case openSettings

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

        var tint: Color {
            switch self {
            case .threads: Chrome.blue
            case .window: Color(red: 0.686, green: 0.322, blue: 0.871)
            }
        }

        var shortcuts: [AppShortcut] {
            AppShortcut.allCases.filter { $0.section == self }
        }
    }

    var id: String { rawValue }

    var section: Section {
        switch self {
        case .newThread, .newWorktreeThread, .addProject, .stopTurn, .queueChat, .togglePlanMode, .previousThread, .nextThread, .runScript, .finishThread, .archiveThread, .settleThread:
            .threads
        case .commandPalette, .toggleSidebar, .toggleTerminal, .toggleChanges, .toggleActivityView, .showMainWindow, .openSettings:
            .window
        }
    }

    var title: String {
        switch self {
        case .newThread: "New thread"
        case .newWorktreeThread: "New thread in worktree"
        case .addProject: "Add project"
        case .stopTurn: "Stop the running turn"
        case .queueChat: "Queue a chat"
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
        case .toggleActivityView: "Activity view"
        case .showMainWindow: "Show Droppy Code"
        case .openSettings: "Settings"
        }
    }

    var detail: String {
        switch self {
        case .newThread: "Starts a thread in the current project."
        case .newWorktreeThread: "Starts a thread in its own git worktree."
        case .addProject: "Opens a folder as a project."
        case .stopTurn: "Interrupts the agent while it works."
        case .queueChat: "While a turn runs, lines the draft up behind it."
        case .togglePlanMode: "Plans before building."
        case .previousThread: "Selects the thread above in the sidebar."
        case .nextThread: "Selects the thread below in the sidebar."
        case .runScript: "Runs the first project script in the selected thread's terminal."
        case .finishThread: "Settles the selected thread, or archives it, whichever General chose. Reopens a settled one."
        case .archiveThread: "Asks to archive the selected thread; Return archives it, and Delete is a click away."
        case .settleThread: "Settles the selected thread. Reopens one that is settled."
        case .commandPalette: "Searches threads, projects and actions."
        case .toggleSidebar: "Shows or hides the sidebar."
        case .toggleTerminal: "Shows or hides the thread's terminal."
        case .toggleChanges: "Shows or hides the changes panel."
        case .toggleActivityView: "Lists every thread by when it was last active."
        case .showMainWindow: "Brings the main window to the front."
        case .openSettings: "Opens this window."
        }
    }

    var defaultChord: KeyChord? {
        switch self {
        case .newThread: KeyChord(keyCode: 45, modifiers: .command)
        case .newWorktreeThread: KeyChord(keyCode: 45, modifiers: [.shift, .command])
        case .addProject: KeyChord(keyCode: 31, modifiers: .command)
        case .stopTurn: KeyChord(keyCode: 47, modifiers: .command)
        case .queueChat: KeyChord(keyCode: 36, modifiers: .command)
        case .togglePlanMode: KeyChord(keyCode: 35, modifiers: [.shift, .command])
        case .previousThread: KeyChord(keyCode: 48, modifiers: [.control, .shift])
        case .nextThread: KeyChord(keyCode: 48, modifiers: .control)
        case .runScript: KeyChord(keyCode: 15, modifiers: .command)
        case .finishThread: KeyChord(keyCode: 51, modifiers: [.shift, .command])
        case .archiveThread: KeyChord(keyCode: 13, modifiers: .command)
        case .settleThread: KeyChord(keyCode: 1, modifiers: .command)
        case .commandPalette: KeyChord(keyCode: 40, modifiers: .command)
        case .toggleSidebar: KeyChord(keyCode: 1, modifiers: [.control, .command])
        case .toggleTerminal: KeyChord(keyCode: 38, modifiers: .command)
        case .toggleChanges: KeyChord(keyCode: 2, modifiers: .command)
        case .toggleActivityView: KeyChord(keyCode: 32, modifiers: [.option, .command])
        case .showMainWindow: KeyChord(keyCode: 29, modifiers: .command)
        case .openSettings: KeyChord(keyCode: 43, modifiers: .command)
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
        return chords
    }()

    private var overrides: [String: Override] {
        didSet {
            if let data = try? JSONEncoder().encode(overrides) {
                UserDefaults.standard.set(data, forKey: Self.defaultsKey)
            }
        }
    }

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.defaultsKey),
           let stored = try? JSONDecoder().decode([String: Override].self, from: data) {
            overrides = stored
        } else {
            overrides = [:]
        }
    }

    func chord(for shortcut: AppShortcut) -> KeyChord? {
        if let override = overrides[shortcut.rawValue] { return override.chord }
        return shortcut.defaultChord
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
        if Self.reserved.contains(chord) { return "macOS or Droppy Code already uses this chord." }
        if let owner = AppShortcut.allCases.first(where: { $0 != shortcut && self.chord(for: $0) == chord }) {
            return "\(owner.title) already uses this chord."
        }
        return nil
    }
}
