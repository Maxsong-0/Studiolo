// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit

// A thread's scripts come from two lists. Its project's is kept in the library: the scripts
// added in the app, and the ones the project's `bottega.json` (or the upstream app's
// `droppy-code.json`) lists. The global list, which every thread offers whatever its project,
// is read from `bottega.json` in the app's Application Support folder. Both files stay
// their scripts' source: a look at a list checks its file's stamp and reads the file again
// only once it has changed, so an edit made in any editor shows the next time the list is
// looked at: the thread coming on screen, the chat's ▶ menu opening, the app coming to the
// front, a script about to run. The global scripts come first wherever the lists are shown,
// so each keeps its ⌥⌘ number in every project.

extension AppModel {
    /// The scripts a thread runs, in the order the Workflow menu and the chat's ▶ menu list
    /// and number them: the global ones, then the thread's project's.
    func scripts(for threadID: UUID) -> [ProjectScript] {
        globalScripts + projectScripts(for: threadID)
    }

    /// The thread's project's scripts, or none for a thread that is gone.
    func projectScripts(for threadID: UUID) -> [ProjectScript] {
        guard let thread = thread(threadID), let project = project(thread.projectID) else { return [] }
        return project.scripts
    }

    /// Reads the global scripts' file and the thread's project's file again where either has
    /// changed since it was last read. Cheap enough for every look: an unchanged file costs
    /// one stat.
    func refreshScripts(for threadID: UUID?) {
        refreshGlobalScripts()
        guard let threadID, let thread = thread(threadID) else { return }
        refreshProjectScripts(thread.projectID)
    }

    /// Folds the project's scripts file into its list when the file has changed since it was
    /// last read (see `ProjectFile.merge`). A file gone takes its scripts with it; one that
    /// does not read as it stands (caught half-way through a save, or not JSON yet) leaves
    /// the list as it was until it is saved again, and so does a project folder that is not
    /// there at all (on a drive that is not mounted, say).
    func refreshProjectScripts(_ id: UUID) {
        guard let project = project(id), FileManager.default.fileExists(atPath: project.path) else { return }
        let file = ProjectFile.file(in: project.url)
        let stamp = ScriptFileStamp(file: file)
        guard projectScriptStamps[id] != stamp else { return }
        projectScriptStamps[id] = stamp
        let listed: [ProjectScript]
        if let file {
            guard let read = ProjectFile.scripts(inFile: file) else { return }
            listed = read
        } else {
            listed = []
        }
        updateProject(id) { $0.scripts = ProjectFile.merge(listed, into: $0.scripts) }
    }

    /// The same for the global list, every script of which is the file's.
    func refreshGlobalScripts() {
        let file = ProjectFile.globalFileIfPresent
        let stamp = ScriptFileStamp(file: file)
        guard globalScriptStamp != stamp else { return }
        globalScriptStamp = stamp
        let listed: [ProjectScript]
        if let file {
            guard let read = ProjectFile.scripts(inFile: file) else { return }
            listed = read
        } else {
            listed = []
        }
        let merged = ProjectFile.merge(listed, into: globalScripts)
        if merged != globalScripts { globalScripts = merged }
    }

    /// Runs the thread's script at `number`, counted from 1 in the order the Workflow menu
    /// numbers them, as the files have it now: what ⌥⌘N runs. The files are read again first,
    /// so a list changed since the menu was built runs its own N-th script. Nothing for a
    /// number past the list.
    func runScript(number: Int, threadID: UUID) {
        refreshScripts(for: threadID)
        let list = scripts(for: threadID)
        guard list.indices.contains(number - 1) else { return }
        runScript(list[number - 1], threadID: threadID)
    }

    /// Reads every scripts file again whether or not it looks changed: the Workflow menu's
    /// Reload scripts.
    func reloadScripts() {
        globalScriptStamp = nil
        projectScriptStamps = [:]
        refreshScripts(for: selectedThreadID)
    }

    /// Writes the global list to its file and reads the file back, so every menu shows what
    /// the file now holds. It is read back through the list as saved, so a script renamed in
    /// the editor keeps its id, and with it its terminal tab.
    func saveGlobalScripts(_ scripts: [ProjectScript]) throws {
        // The names the file knew them by, so a script renamed in the editor keeps whatever
        // else its entry held.
        let formerNames = Dictionary(globalScripts.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
        try ProjectFile.write(scripts, to: ProjectFile.globalFile, formerNames: formerNames)
        globalScripts = scripts
        globalScriptStamp = nil
        refreshGlobalScripts()
    }

    /// Opens the scripts editor away from the chat's ▶ menu, for the Workflow menu's Edit
    /// items: the chat on screen takes the request up and shows the editor over itself. With
    /// nothing on screen to show it (no thread open, say), the list's file opens instead.
    func requestScriptsEditor(_ target: ScriptsEditorTarget) {
        let request = ScriptsEditorRequest(target: target)
        scriptsEditorRequest = request
        Task {
            try? await Task.sleep(for: .milliseconds(80))
            guard scriptsEditorRequest == request else { return }
            scriptsEditorRequest = nil
            openScriptsFile(target)
        }
    }

    /// Opens a list's scripts file in whatever edits JSON on this Mac. The global file is
    /// made, with no scripts in it, when it is not there yet; a project's opens only when the
    /// project has one, since a file made there would land in its checkout.
    func openScriptsFile(_ target: ScriptsEditorTarget) {
        switch target {
        case .global:
            let file = ProjectFile.globalFile
            if ProjectFile.globalFileIfPresent == nil {
                do {
                    try ProjectFile.write([], to: file)
                } catch {
                    alert = AppAlert(title: "Could not create the global scripts file", message: error.localizedDescription)
                    return
                }
            }
            NSWorkspace.shared.open(file)
        case .project(let id):
            guard let project = project(id) else { return }
            guard let file = ProjectFile.file(in: project.url) else {
                alert = AppAlert(
                    title: "No scripts file",
                    message: "\(project.name) has no \(ProjectFile.names[0]). Add its scripts from the ▶ menu at the top of one of its chats."
                )
                return
            }
            NSWorkspace.shared.open(file)
        }
    }
}

/// A scripts file as it was when last read: where it is, when it last changed and how big
/// it is. Two stamps alike mean the file has not changed, or is still not there.
struct ScriptFileStamp: Equatable {
    var path: String?
    var modified: Date?
    var size: Int?

    init(file: URL?) {
        path = file?.path
        guard let file, let attributes = try? FileManager.default.attributesOfItem(atPath: file.path) else { return }
        modified = attributes[.modificationDate] as? Date
        size = attributes[.size] as? Int
    }
}

/// The list the scripts editor edits: one project's, or the global one every thread offers.
enum ScriptsEditorTarget: Hashable, Identifiable {
    case project(UUID)
    case global

    var id: Self { self }
}

/// The scripts editor asked for from the Workflow menu, for the chat on screen to take up.
struct ScriptsEditorRequest: Equatable {
    let id = UUID()
    let target: ScriptsEditorTarget
}

enum ScriptFileError: LocalizedError {
    /// The file is there but does not read as a JSON object, so it is not written over.
    case unreadable(URL)
    /// The file's `scripts` is not a list (`count` nil), or holds entries with no name or
    /// command that no menu shows: saving over them would lose them.
    case unreadableScripts(URL, count: Int?)

    var errorDescription: String? {
        switch self {
        case .unreadable(let file):
            "\(file.lastPathComponent) is there but does not read as JSON. Fix it or remove it, then save again."
        case .unreadableScripts(let file, let count?):
            "\(count == 1 ? "A script" : "\(count) scripts") in \(file.lastPathComponent) \(count == 1 ? "has" : "have") no name or command, so the menus leave \(count == 1 ? "it" : "them") out and saving would drop \(count == 1 ? "it" : "them"). Fix \(count == 1 ? "it" : "them") in the file, then save again."
        case .unreadableScripts(let file, nil):
            "The scripts in \(file.lastPathComponent) are not a list, so saving would drop them. Fix the file, then save again."
        }
    }
}

/// The scripts' keys: ⌥⌘1 to ⌥⌘9 for the first nine scripts in the order
/// `AppModel.scripts(for:)` lists them. Fixed rather than recorded, so the Shortcuts page's
/// recorder refuses them (see `ShortcutStore.scriptChords`).
@MainActor
enum ScriptShortcuts {
    /// The chord for the script at this place in the list, counted from 1. None past the
    /// ninth, and none while a command holds the chord: one recorded for it before the
    /// scripts took these keys stays the command's.
    static func chord(forNumber number: Int) -> KeyChord? {
        let chords = ShortcutStore.scriptChords
        guard chords.indices.contains(number - 1) else { return nil }
        let chord = chords[number - 1]
        let store = ShortcutStore.shared
        guard !AppShortcut.allCases.contains(where: { store.chord(for: $0) == chord }) else { return nil }
        return chord
    }
}

extension ProjectScript {
    /// Whether this Mac draws an SF Symbol by this name.
    static func isSymbolName(_ name: String) -> Bool {
        !name.isEmpty && NSImage(systemSymbolName: name, accessibilityDescription: nil) != nil
    }

    /// The SF Symbol a script's icon text stands for: the name itself, trimmed, when this Mac
    /// has that symbol, and the default mark when it has not.
    static func resolvedSymbol(_ name: String) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return isSymbolName(trimmed) ? trimmed : defaultSymbol
    }
}
