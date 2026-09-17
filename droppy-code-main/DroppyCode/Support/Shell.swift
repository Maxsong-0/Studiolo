import Foundation
import Synchronization

struct ShellResult: Sendable {
    var status: Int32
    var stdout: Data
    var stderr: Data

    var succeeded: Bool { status == 0 }
    var output: String { String(decoding: stdout, as: UTF8.self) }
    var errorOutput: String { String(decoding: stderr, as: UTF8.self) }
    var trimmedOutput: String { output.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The most useful text to show when a command fails.
    var failureMessage: String {
        let error = errorOutput.trimmingCharacters(in: .whitespacesAndNewlines)
        let message = error.isEmpty ? trimmedOutput : error
        return message.isEmpty ? "The command failed with exit code \(status)." : message
    }
}

struct ShellError: LocalizedError, Sendable {
    var message: String
    var errorDescription: String? { message }

    init(_ message: String) {
        self.message = message
    }
}

enum Shell {
    /// Runs a process to completion off the main actor and collects its output. With an
    /// `outputLimit`, only that many bytes of each stream's tail are kept (a command a model
    /// runs can print without end for the whole timeout; the app's own commands keep it all).
    @concurrent
    static func run(
        _ executable: URL,
        _ arguments: [String],
        in directory: URL? = nil,
        environment: [String: String]? = nil,
        input: Data? = nil,
        timeout: TimeInterval = 120,
        outputLimit: Int? = nil
    ) async throws -> ShellResult {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        if let directory { process.currentDirectoryURL = directory }
        process.environment = environment ?? LoginEnvironment.current

        let stdout = Pipe()
        let stderr = Pipe()
        let stdin: Pipe? = input == nil ? nil : Pipe()
        process.standardOutput = stdout
        process.standardError = stderr
        process.standardInput = stdin ?? FileHandle.nullDevice

        let collector = OutputCollector(limit: outputLimit)
        stdout.fileHandleForReading.readabilityHandler = collector.reader(for: .stdout)
        stderr.fileHandleForReading.readabilityHandler = collector.reader(for: .stderr)

        let cancelled = Mutex(false)
        var deadline: Task<Void, Never>?
        defer {
            deadline?.cancel()
            stdout.fileHandleForReading.readabilityHandler = nil
            stderr.fileHandleForReading.readabilityHandler = nil
        }
        let status: Int32 = try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                let once = OnceContinuation(continuation)
                process.terminationHandler = { process in
                    once.resume(returning: process.terminationStatus)
                }
                do {
                    try cancelled.withLock { isCancelled in
                        if isCancelled { throw CancellationError() }
                        try process.run()
                    }
                    deadline = Task {
                        do { try await Task.sleep(for: .seconds(timeout)) }
                        catch { return }
                        terminate(process)
                    }
                } catch {
                    once.resume(throwing: error)
                    return
                }
                if let input, let stdin {
                    DispatchQueue.global(qos: .userInitiated).async {
                        try? stdin.fileHandleForWriting.write(contentsOf: input)
                        try? stdin.fileHandleForWriting.close()
                    }
                }
            }
        } onCancel: {
            cancelled.withLock { isCancelled in
                isCancelled = true
                terminate(process)
            }
        }
        deadline?.cancel()
        try Task.checkCancellation()
        let output = await collector.finished(within: 3)
        try Task.checkCancellation()
        return ShellResult(status: status, stdout: output.stdout, stderr: output.stderr)
    }

    private static func terminate(_ process: Process) {
        guard process.isRunning else { return }
        process.terminate()
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 1) {
            guard process.isRunning else { return }
            let pid = process.processIdentifier
            kill(getpgid(pid) == pid ? -pid : pid, SIGKILL)
        }
    }

    /// Runs a named tool found on the login PATH.
    static func run(
        tool name: String,
        _ arguments: [String],
        in directory: URL? = nil,
        environment: [String: String]? = nil,
        input: Data? = nil,
        timeout: TimeInterval = 120
    ) async throws -> ShellResult {
        guard let executable = LoginEnvironment.which(name) else {
            throw ShellError("\(name) was not found on your PATH.")
        }
        return try await run(executable, arguments, in: directory, environment: environment, input: input, timeout: timeout)
    }
}

/// Whether a shell command could change a file. Agents edit from the shell often enough
/// that a command's timeline row takes its edits from a snapshot of the working tree
/// before and after it, and each of those snapshots costs several git processes over the
/// whole checkout. A command that only reads is not worth them. The list below is what is
/// known to read; anything else counts as writing, so an edit is never missed.
enum ShellCommandKind {
    /// Tools that only ever read. Whatever is not here makes its whole command count as
    /// writing, and so does a chain with one such part in it.
    private static let readers: Set<String> = [
        "ls", "cat", "bat", "head", "tail", "wc", "grep", "egrep", "fgrep", "rg", "tree",
        "pwd", "echo", "printf", "which", "command", "type", "stat", "file", "du", "df",
        "ps", "printenv", "date", "whoami", "hostname", "uname", "basename", "dirname",
        "realpath", "readlink", "column", "uniq", "cut", "tr", "nl", "diff", "cmp", "jq",
        "yq", "xxd", "od", "md5", "shasum", "cksum", "man", "true", "false", "sleep",
        "cd", "pushd", "popd",
    ]

    /// Git subcommands that only report. Anything else git can do stays a writer.
    private static let gitReaders: Set<String> = [
        "log", "status", "diff", "show", "branch", "rev-parse", "rev-list", "ls-files",
        "ls-tree", "blame", "remote", "describe", "shortlog", "cat-file", "tag", "config",
        "grep", "whatchanged", "reflog", "for-each-ref", "symbolic-ref", "count-objects",
        "var", "version", "help",
    ]

    /// Searches that run something of their own, which may well write.
    private static let searchActions: Set<String> = ["-delete", "-exec", "-execdir", "-ok", "-okdir", "-x", "-X"]

    static func mayWriteFiles(_ text: String) -> Bool {
        let command = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !command.isEmpty else { return false }
        // Redirection writes whatever the tool in front of it does, and a substitution
        // hides a command this cannot see.
        guard !command.contains(">"), !command.contains("$("), !command.contains("`") else { return true }
        for part in parts(of: command) where !onlyReads(part) { return true }
        return false
    }

    /// A command line as the separate commands it runs. Quoting is not taken apart: a
    /// separator inside a quoted argument splits the line into parts that read as
    /// unfamiliar tools, which is the safe answer anyway.
    private static func parts(of command: String) -> [String] {
        var text = command
        for separator in ["&&", "||", ";", "|", "\n", "&"] {
            text = text.replacingOccurrences(of: separator, with: "\u{1}")
        }
        return text.split(separator: "\u{1}").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    private static func onlyReads(_ part: String) -> Bool {
        let tokens = part.split(whereSeparator: \.isWhitespace).map(String.init)
        guard let first = tokens.first else { return true }
        let tool = (first as NSString).lastPathComponent
        let arguments = Array(tokens.dropFirst())
        switch tool {
        case "sed":
            let prints = arguments.contains { $0.hasPrefix("-") && !$0.hasPrefix("--") && $0.contains("n") }
            let inPlace = arguments.contains { $0.hasPrefix("-i") || $0.hasPrefix("--in-place") }
            return prints && !inPlace
        case "awk", "gawk", "mawk":
            // Redirection is already ruled out; awk can still shell out through system().
            return !part.contains("system(")
        case "find", "fd":
            return !arguments.contains { searchActions.contains($0) }
        case "sort":
            return !arguments.contains { $0.hasPrefix("-o") }
        case "git":
            guard let subcommand = arguments.first(where: { !$0.hasPrefix("-") }) else { return true }
            return gitReaders.contains(subcommand)
        case "xcodebuild":
            return arguments.contains { $0 == "-showBuildSettings" || $0 == "-list" || $0 == "-version" || $0 == "-showsdks" }
        case "swift", "swiftc", "node", "npm", "python", "python3":
            return arguments == ["--version"] || arguments == ["-version"] || arguments == ["-v"]
        case "env":
            // Bare env prints the environment; with a command after it, that command runs.
            return arguments.allSatisfy { $0.hasPrefix("-") }
        default:
            return readers.contains(tool)
        }
    }
}

/// Resumes a continuation exactly once, however many callbacks race to finish it.
final class OnceContinuation<Value: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Value, Error>?

    init(_ continuation: CheckedContinuation<Value, Error>) {
        self.continuation = continuation
    }

    func resume(returning value: Value) {
        take()?.resume(returning: value)
    }

    func resume(throwing error: Error) {
        take()?.resume(throwing: error)
    }

    private func take() -> CheckedContinuation<Value, Error>? {
        lock.withLock {
            let continuation = self.continuation
            self.continuation = nil
            return continuation
        }
    }
}

private final class OutputCollector: @unchecked Sendable {
    enum Stream: Hashable {
        case stdout
        case stderr
    }

    private let lock = NSLock()
    private var stdout = Data()
    private var stderr = Data()
    private var closed: Set<Stream> = []
    private var waiters: [OnceContinuation<Void>] = []
    private let limit: Int?

    init(limit: Int?) {
        self.limit = limit
    }

    func reader(for stream: Stream) -> @Sendable (FileHandle) -> Void {
        { [self] handle in
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
                close(stream)
            } else {
                lock.withLock {
                    switch stream {
                    case .stdout: append(data, to: &stdout)
                    case .stderr: append(data, to: &stderr)
                    }
                }
            }
        }
    }

    /// Keeps the tail within the limit, trimming by whole chunks once the buffer holds twice
    /// the limit, so a stream that never ends costs O(limit) memory and amortized O(1) per byte.
    private func append(_ data: Data, to buffer: inout Data) {
        buffer.append(data)
        guard let limit, buffer.count > limit * 2 else { return }
        buffer = Data(buffer.suffix(limit))
    }

    func finished(within timeout: TimeInterval) async -> (stdout: Data, stderr: Data) {
        if let output = lock.withLock({ closed.count == 2 ? (stdout, stderr) : nil }) { return output }
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(timeout)) }
            catch { return }
            close(.stdout)
            close(.stderr)
        }
        defer { deadline.cancel() }
        await withTaskCancellationHandler {
            try? await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                let once = OnceContinuation(continuation)
                let isDone = lock.withLock {
                    if closed.count == 2 { return true }
                    waiters.append(once)
                    return false
                }
                if isDone { once.resume(returning: ()) }
            }
        } onCancel: {
            close(.stdout)
            close(.stderr)
        }
        return lock.withLock { (stdout, stderr) }
    }

    private func close(_ stream: Stream) {
        let ready: [OnceContinuation<Void>] = lock.withLock {
            guard closed.insert(stream).inserted, closed.count == 2 else { return [] }
            let ready = waiters
            waiters.removeAll()
            return ready
        }
        for waiter in ready { waiter.resume(returning: ()) }
    }
}

/// The user's login shell environment. Apps launched from Finder inherit a
/// minimal PATH that cannot find provider CLIs, so this is captured once at launch.
enum LoginEnvironment {
    private static let cached = Mutex<[String: String]?>(nil)
    /// The one read of the login shell. Launch starts it; anything that resolves a CLI before
    /// it lands waits on the same task, so a codex under nvm is never reported missing.
    private static let loading = Mutex<Task<Void, Never>?>(nil)

    private static let excludedKeys: Set<String> = [
        "_", "SHLVL", "PWD", "OLDPWD", "TERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION",
        "TERM_SESSION_ID", "COLORTERM", "ITERM_SESSION_ID", "XPC_SERVICE_NAME",
        "__CFBundleIdentifier", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT",
    ]

    static var current: [String: String] {
        cached.withLock { $0 } ?? fallbackEnvironment
    }

    /// The process's own environment, for the moments before the login shell has answered:
    /// built once, since every reader in that window asked for a fresh copy.
    private static let fallbackEnvironment = fallback()

    static var isLoaded: Bool {
        cached.withLock { $0 != nil }
    }

    static var homeDirectory: String {
        FileManager.default.homeDirectoryForCurrentUser.path
    }

    static func load() async {
        let task = loading.withLock { current -> Task<Void, Never> in
            if let current { return current }
            let started = Task { await read() }
            current = started
            return started
        }
        await task.value
    }

    @concurrent
    private static func read() async {
        let marker = "__DROPPY_CODE_ENVIRONMENT__"
        let script = "printf '\(marker)'; /usr/bin/env -0; printf '\(marker)'"
        var environment = fallbackEnvironment
        let shell = URL(fileURLWithPath: userShell())
        if let result = try? await Shell.run(shell, ["-l", "-i", "-c", script], environment: environment, timeout: 8) {
            let text = result.output
            if let start = text.range(of: marker),
               let end = text.range(of: marker, options: .backwards),
               start.upperBound < end.lowerBound {
                for entry in text[start.upperBound..<end.lowerBound].split(separator: "\0") {
                    guard let equals = entry.firstIndex(of: "=") else { continue }
                    let key = String(entry[..<equals])
                    guard !key.isEmpty, !excludedKeys.contains(key) else { continue }
                    environment[key] = String(entry[entry.index(after: equals)...])
                }
            }
        }
        environment["PATH"] = augmentedPath(environment["PATH"])
        let resolved = environment
        cached.withLock { $0 = resolved }
        let git = await developerGit(environment: resolved)
        resolvedGit.withLock { $0 = git }
    }

    /// The git binary every Git call runs, found once. PATH usually gives `/usr/bin/git`,
    /// Apple's shim that looks up the active developer directory and execs the real binary
    /// on every call: measured at 11.5 ms per call against 4.5 ms for the binary itself, on
    /// the dozens of calls a turn makes. The shim's own target is used instead where it can
    /// be found; without developer tools, the shim stays so its install prompt still shows.
    static var git: URL {
        resolvedGit.withLock { $0 } ?? which("git") ?? URL(fileURLWithPath: "/usr/bin/git")
    }

    private static let resolvedGit = Mutex<URL?>(nil)

    private static func developerGit(environment: [String: String]) async -> URL {
        let onPath = which("git", in: environment) ?? URL(fileURLWithPath: "/usr/bin/git")
        guard onPath.path == "/usr/bin/git",
              let result = try? await Shell.run(URL(fileURLWithPath: "/usr/bin/xcode-select"), ["-p"], environment: environment, timeout: 8),
              result.succeeded else { return onPath }
        let developer = result.trimmedOutput
        let candidate = URL(fileURLWithPath: developer).appendingPathComponent("usr/bin/git")
        guard !developer.isEmpty, FileManager.default.isExecutableFile(atPath: candidate.path) else { return onPath }
        return candidate
    }

    static func which(_ name: String, in environment: [String: String]? = nil) -> URL? {
        let fileManager = FileManager.default
        if name.contains("/") {
            let path = (name as NSString).expandingTildeInPath
            return fileManager.isExecutableFile(atPath: path) ? URL(fileURLWithPath: path) : nil
        }
        let path = (environment ?? current)["PATH"] ?? augmentedPath(nil)
        for directory in path.split(separator: ":") {
            let candidate = URL(fileURLWithPath: String(directory)).appendingPathComponent(name)
            if fileManager.isExecutableFile(atPath: candidate.path) { return candidate }
        }
        return nil
    }

    static func userShell() -> String {
        let fileManager = FileManager.default
        if let shell = ProcessInfo.processInfo.environment["SHELL"], fileManager.isExecutableFile(atPath: shell) {
            return shell
        }
        if let entry = getpwuid(getuid()), let shell = entry.pointee.pw_shell {
            let path = String(cString: shell)
            if fileManager.isExecutableFile(atPath: path) { return path }
        }
        return "/bin/zsh"
    }

    static func augmentedPath(_ path: String?) -> String {
        let home = homeDirectory
        var entries = (path ?? "").split(separator: ":").map(String.init)
        let common = [
            "/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "\(home)/.local/bin",
            "\(home)/.bun/bin", "\(home)/.cargo/bin", "\(home)/.npm-global/bin", "\(home)/.volta/bin",
            "/usr/bin", "/bin", "/usr/sbin", "/sbin",
        ]
        for directory in common where !entries.contains(directory) {
            entries.append(directory)
        }
        return entries.joined(separator: ":")
    }

    private static func fallback() -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        for key in excludedKeys { environment.removeValue(forKey: key) }
        environment["PATH"] = augmentedPath(environment["PATH"])
        if environment["LANG"] == nil { environment["LANG"] = "en_US.UTF-8" }
        return environment
    }
}
