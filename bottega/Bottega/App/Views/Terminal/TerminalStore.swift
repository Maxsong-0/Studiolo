// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftTerm

/// A shell running in a pseudo terminal, kept alive while its thread is open.
@MainActor
@Observable
final class TerminalSession: Identifiable {
    let id = UUID()
    let threadID: UUID
    let directory: String
    var title: String
    /// The script the tab was opened to run, if one was: running that script again starts
    /// it over in this tab rather than opening another (see `TerminalStore.run`).
    let scriptID: UUID?
    private(set) var isRunning = true

    @ObservationIgnored let view: LocalProcessTerminalView
    @ObservationIgnored private var bridge: TerminalBridge?

    init(threadID: UUID, directory: String, title: String, command: String?, scriptID: UUID? = nil) {
        self.threadID = threadID
        self.directory = directory
        self.title = title
        self.scriptID = scriptID
        view = LocalProcessTerminalView(frame: NSRect(x: 0, y: 0, width: 640, height: 240))
        view.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        view.optionAsMetaKey = true
        let bridge = TerminalBridge(session: self)
        self.bridge = bridge
        view.processDelegate = bridge

        let shell = LoginEnvironment.userShell()
        var environment = LoginEnvironment.current
        environment["TERM"] = "xterm-256color"
        environment["COLORTERM"] = "truecolor"
        environment["TERM_PROGRAM"] = "Bottega"
        // The folder the tab works in: the thread's worktree, or its project's root. A script
        // that runs in every project (a global one) finds its project here. The upstream
        // app's name for it stays, for scripts written against that app.
        environment["BOTTEGA_PROJECT_ROOT"] = directory
        environment["DROPPY_CODE_PROJECT_ROOT"] = directory
        let pairs = environment.map { "\($0.key)=\($0.value)" }
        let shellName = "-" + (shell as NSString).lastPathComponent
        var arguments: [String] = []
        if let command {
            arguments = ["-l", "-i", "-c", "\(command); exec \(shell) -l"]
        }
        view.startProcess(executable: shell, args: arguments, environment: pairs, execName: shellName, currentDirectory: directory)
    }

    /// The colours last applied: setting them again clears SwiftTerm's attribute caches and
    /// redraws the whole screen, so the host applies them only on a change. The whole
    /// appearance is compared, not just its side: two themes on the same side still differ
    /// in accent, and a theme of one's own in every colour.
    @ObservationIgnored private var appliedAppearance: TerminalAppearance?

    func applyAppearance(_ appearance: TerminalAppearance) {
        guard appliedAppearance != appearance else { return }
        let previousANSI = appliedAppearance?.palette.ansi
        appliedAppearance = appearance
        view.nativeBackgroundColor = appearance.palette.background.nsColor
        view.nativeForegroundColor = appearance.palette.foreground.nsColor
        // The caret and the selection follow the theme's accent, like every other
        // insertion point in the app.
        let accent = appearance.accent.nsColor
        view.caretColor = accent
        view.selectedTextBackgroundColor = accent.withAlphaComponent(0.3)
        // A theme's own sixteen colours, or SwiftTerm's again once a theme without them takes
        // over. Installing a palette redraws the screen too, so only on a real change.
        if appearance.palette.ansi != previousANSI {
            view.installColors(appearance.palette.ansi.map { $0.map(Self.terminalColor) } ?? SwiftTerm.Color.terminalAppColors)
        }
    }

    /// A theme's colour as SwiftTerm's own: the package keeps its bridge from AppKit internal.
    private nonisolated static func terminalColor(_ rgb: ThemeRGB) -> SwiftTerm.Color {
        SwiftTerm.Color(red8: UInt16(rgb.red), green8: UInt16(rgb.green), blue8: UInt16(rgb.blue))
    }

    func terminate() {
        guard isRunning else { return }
        view.terminate()
        isRunning = false
    }

    /// Whether the shell owns the foreground of its terminal: true at the prompt, false while
    /// a job it started (vim, claude, less) has the terminal. Read from the kernel's view of
    /// the shell, since the pty's master side answers no `tcgetpgrp` to a process it is not
    /// the controlling terminal of.
    nonisolated static func foregroundJobIsTheShell(_ shell: pid_t) -> Bool {
        guard shell > 0 else { return false }
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var name: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, shell]
        guard sysctl(&name, 4, &info, &size, nil, 0) == 0, size > 0 else { return false }
        return info.kp_eproc.e_tpgid == info.kp_eproc.e_pgid
    }

    fileprivate func processEnded() {
        isRunning = false
    }

    /// Sets the terminal's type: the code face at the terminal's size, from the window (see
    /// `TerminalHost`). A new font makes SwiftTerm measure its cells again and re-wrap the
    /// whole screen, so it is set only when it differs from the one in use.
    func applyFont(_ font: NSFont) {
        guard view.font != font else { return }
        view.font = font
    }
}

/// Receives SwiftTerm callbacks, which arrive on the main thread.
private final class TerminalBridge: NSObject, LocalProcessTerminalViewDelegate {
    weak var session: TerminalSession?

    init(session: TerminalSession) {
        self.session = session
    }

    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}

    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {}

    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        let session = self.session
        Task { @MainActor in session?.processEnded() }
    }
}

@MainActor
@Observable
final class TerminalStore {
    private(set) var sessions: [UUID: [TerminalSession]] = [:]
    var selection: [UUID: UUID] = [:]

    func sessions(for threadID: UUID) -> [TerminalSession] {
        sessions[threadID] ?? []
    }

    func selected(for threadID: UUID) -> TerminalSession? {
        let list = sessions(for: threadID)
        return list.first { $0.id == selection[threadID] } ?? list.last
    }

    @discardableResult
    func open(threadID: UUID, directory: String, title: String? = nil, command: String? = nil, scriptID: UUID? = nil) -> TerminalSession {
        let number = sessions(for: threadID).count + 1
        let session = TerminalSession(threadID: threadID, directory: directory, title: title ?? "Terminal \(number)", command: command, scriptID: scriptID)
        sessions[threadID, default: []].append(session)
        selection[threadID] = session.id
        return session
    }

    /// Runs a script in a tab of its own. The tab it ran in last time, while it is still
    /// open, starts over with the script's command, whatever it was doing: a script run
    /// again and again keeps to one tab instead of filling the row. The first run opens it.
    func run(_ script: ProjectScript, threadID: UUID, directory: String) {
        if let tab = sessions(for: threadID).first(where: { $0.scriptID == script.id }) {
            restart(tab, title: script.name, directory: directory, command: script.command)
        } else {
            open(threadID: threadID, directory: directory, title: script.name, command: script.command, scriptID: script.id)
        }
    }

    func ensureTerminal(threadID: UUID, directory: String) {
        if sessions(for: threadID).isEmpty {
            open(threadID: threadID, directory: directory)
        }
    }

    /// Replaces an exited shell with a fresh one in the same place in the row, keeping the
    /// tab's name and its directory, so a shell that quit is one click from working again.
    /// A script run again passes its current name, folder and command: the tab starts the
    /// command over and stays that script's.
    @discardableResult
    func restart(_ session: TerminalSession, title: String? = nil, directory: String? = nil, command: String? = nil) -> TerminalSession {
        session.terminate()
        let fresh = TerminalSession(
            threadID: session.threadID,
            directory: directory ?? session.directory,
            title: title ?? session.title,
            command: command,
            scriptID: session.scriptID
        )
        if let index = sessions[session.threadID]?.firstIndex(where: { $0.id == session.id }) {
            sessions[session.threadID]?[index] = fresh
        } else {
            sessions[session.threadID, default: []].append(fresh)
        }
        selection[session.threadID] = fresh.id
        return fresh
    }

    func close(_ session: TerminalSession) {
        session.terminate()
        sessions[session.threadID]?.removeAll { $0.id == session.id }
        if selection[session.threadID] == session.id {
            selection[session.threadID] = sessions[session.threadID]?.last?.id
        }
    }

    func closeAll(for threadID: UUID) {
        for session in sessions(for: threadID) { session.terminate() }
        sessions[threadID] = nil
        selection[threadID] = nil
    }

    func terminateAll() {
        for list in sessions.values {
            for session in list { session.terminate() }
        }
    }
}
