// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
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
/// whole checkout. A command that only reads is not worth them. The lists below are what
/// is known to write nothing; anything else counts as writing, so an edit is never
/// missed: a writer taken for a reader loses its row's edits, a reader taken for a writer
/// costs a snapshot.
///
/// Command Code's Auto mode asks a different question of the same command: whether it can
/// run with nothing to approve (`runsUnasked`). That is the reading answer with more taken
/// away: whatever can make a reader run a program of the command's choosing (a variable in
/// front of it, `env` setting one, git's `-c` and config writes, a pager, the shell builtins
/// that change what a later name runs) needs asking, though it writes no file itself.
enum ShellCommandKind {
    /// Tools that only ever read. Whatever is not here makes its whole command count as
    /// writing, and so does a chain with one such part in it.
    private static let readers: Set<String> = [
        "ls", "cat", "bat", "head", "tail", "wc", "grep", "egrep", "fgrep", "ag", "ack",
        "pwd", "echo", "printf", "which", "whereis", "type", "stat", "du", "df", "ps",
        "pgrep", "lsof", "printenv", "date", "whoami", "id", "hostname", "uname", "sw_vers",
        "nproc", "uptime", "basename", "dirname", "realpath", "readlink", "column", "cut",
        "tr", "nl", "rev", "tac", "paste", "comm", "fold", "expand", "diff", "cmp", "jq",
        "od", "hexdump", "strings", "md5", "md5sum", "shasum", "sha1sum", "sha256sum",
        "sha512sum", "cksum", "otool", "nm", "objdump", "mdls", "mdfind", "eza", "exa",
        "man", "true", "false", ":", "test", "[", "[[", "seq", "expr", "sleep",
        "cd", "pushd", "popd",
        // Shell builtins that set, read or leave something in the shell and touch no file.
        "read", "wait", "export", "unset", "set", "local", "declare", "typeset", "readonly",
        "shopt", "alias", "unalias", "umask", "hash", "exit", "return", "break", "continue",
        "shift",
    ]

    /// Git subcommands that leave the working tree alone. Anything else git can do stays a
    /// writer.
    private static let gitReaders: Set<String> = [
        "log", "status", "diff", "show", "branch", "rev-parse", "rev-list", "ls-files",
        "ls-tree", "blame", "annotate", "remote", "describe", "shortlog", "cat-file", "tag",
        "config", "grep", "whatchanged", "reflog", "for-each-ref", "symbolic-ref",
        "count-objects", "var", "version", "help", "merge-base", "name-rev", "check-ignore",
        "check-attr", "cherry", "range-diff", "diff-tree", "diff-index", "diff-files",
        "show-ref", "show-branch", "ls-remote", "verify-commit", "verify-tag",
    ]

    /// Git subcommands that only read in the forms named, by the first word after them:
    /// `git stash list` reads, where a bare `git stash` puts the changes away.
    private static let gitReadingForms: [String: Set<String>] = [
        "stash": ["list", "show"],
        "worktree": ["list"],
        "submodule": ["status", "summary"],
    ]

    /// Git's own options that take the next word as their value, ahead of the subcommand.
    private static let gitValuedOptions: Set<String> = ["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env"]

    /// Git's own options that point it at configuration or programs of the command's
    /// choosing: `-c core.fsmonitor=…` runs a program on the next read, a repository
    /// elsewhere brings its own config. None of them runs unasked.
    private static let gitRedirectingOptions = ["-c", "--config-env", "--git-dir", "--work-tree", "--exec-path"]

    /// Options of any git subcommand that name a program for git to run.
    private static let gitProgramOptions = ["--upload-pack", "--receive-pack", "--exec"]

    /// Shell builtins that write no file but change what a later name in the line runs or
    /// what it sees: an alias, a hashed path, an exported variable (`GIT_EXTERNAL_DIFF`,
    /// `PAGER`), `set -a` exporting every assignment after it. Readers for a snapshot;
    /// never run unasked.
    private static let resolutionBuiltins: Set<String> = [
        "export", "unset", "set", "local", "declare", "typeset", "readonly", "shopt", "alias", "unalias", "hash",
        // Each assigns a variable (`read PATH`), which a later name in the line then runs by.
        "read",
    ]

    /// Readers that hand their output to a pager of the command's naming.
    private static let pagingReaders: Set<String> = ["bat", "ag", "ack"]

    /// Readers no argument of which runs a program or writes a file, whatever the argument
    /// turns out to be: the only tools a word the shell still expands (`$f`, a brace list)
    /// may reach unasked, since that word can become any option at all.
    private static let inertReaders: Set<String> = [
        "ls", "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "pwd", "echo", "which",
        "whereis", "type", "stat", "du", "df", "printenv", "date", "whoami", "id", "uname",
        "basename", "dirname", "realpath", "readlink", "column", "cut", "tr", "nl", "rev",
        "tac", "paste", "comm", "fold", "expand", "diff", "cmp", "jq", "od", "hexdump",
        "strings", "md5", "md5sum", "shasum", "sha1sum", "sha256sum", "sha512sum", "cksum",
        "true", "false", ":", "test", "[", "[[", "seq", "expr", "sleep",
    ]

    /// `set` options that change how the rest of the line fails or reads, and nothing it runs:
    /// `set -euo pipefail` and its kin. Anything else (`-a` exporting every assignment, `-k`
    /// taking them from arguments) needs asking.
    private static let harmlessSetFlags: Set<Character> = ["e", "u", "x", "v", "o"]
    private static let harmlessSetOptions: Set<String> = ["pipefail", "errexit", "nounset", "xtrace", "verbose"]

    /// What a search runs of its own, which may well write, or the file its findings go to.
    private static let searchActions: Set<String> = [
        "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf", "-fls",
        "-x", "-X", "--exec", "--exec-batch",
    ]

    /// Words of the shell's own grammar that run nothing themselves: what follows one is
    /// judged as it stands.
    private static let shellWords: Set<String> = ["if", "then", "elif", "else", "fi", "do", "done", "while", "until", "!"]

    /// Tools that report their version and do nothing else when that is all they are asked.
    private static let versionReporters: Set<String> = [
        "swift", "swiftc", "node", "npm", "python", "python3", "ruby", "rustc", "cargo",
        "clang", "clang++", "gcc", "g++", "make", "cmake", "java", "bun", "deno", "pnpm",
        "yarn", "pip", "pip3", "brew",
    ]

    private static let versionFlags: Set<String> = ["--version", "-version", "-v", "-V"]

    /// `defaults` verbs that only read.
    private static let defaultsReaders: Set<String> = ["read", "read-type", "domains", "find", "help"]

    /// A word without the subshell and group brackets around it, which run nothing themselves.
    private static func unbracketed(_ word: String) -> String {
        word.trimmingCharacters(in: CharacterSet(charactersIn: "(){}"))
    }

    /// Whether the command may change a file, so its row needs the working tree's snapshots.
    static func mayWriteFiles(_ text: String) -> Bool {
        mayWriteFiles(text, depth: 3, strict: false)
    }

    /// Whether Command Code's Auto mode may run the command without asking: it only reads,
    /// and nothing in it can make a reader run a program of the command's choosing (see the
    /// type's note). Anything this cannot take apart asks.
    static func runsUnasked(_ text: String) -> Bool {
        !mayWriteFiles(text, depth: 3, strict: true)
    }

    /// `depth` is how much deeper substitutions may nest before one counts as writing.
    /// `strict` is the Auto mode's question (`runsUnasked`) rather than the snapshot's.
    private static func mayWriteFiles(_ text: String, depth: Int, strict: Bool) -> Bool {
        // A line continuation makes one command of its two lines.
        let command = text.replacingOccurrences(of: "\\\n", with: " ").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !command.isEmpty else { return false }
        // A substitution runs a command of its own, judged here like any other; one that
        // cannot be taken apart counts as writing.
        guard let flat = withoutSubstitutions(command, depth: depth, strict: strict) else { return true }
        // Output thrown away or shown lands in no file; any other redirection writes
        // whatever the tool in front of it does.
        let bare = withoutHarmlessRedirections(flat)
        guard !bare.contains(">") else { return true }
        for part in parts(of: bare) where !onlyReads(part, strict: strict) { return true }
        return false
    }

    /// The command with each `$(…)`, `<(…)` and backquoted command in it stood in for by a
    /// plain word, once each has been judged to only read; nil when one may write, is left
    /// open, or nests deeper than `depth`. Quoting is not taken apart: a bracket inside a
    /// quoted argument leaves the count uneven, which is the safe answer anyway.
    private static func withoutSubstitutions(_ command: String, depth: Int, strict: Bool) -> String? {
        let characters = Array(command)
        var result = ""
        var index = 0
        while index < characters.count {
            let character = characters[index]
            let opensGroup = (character == "$" || character == "<") && index + 1 < characters.count && characters[index + 1] == "("
            guard character == "`" || opensGroup else {
                result.append(character)
                index += 1
                continue
            }
            guard depth > 0 else { return nil }
            let inner: String
            if character == "`" {
                guard let close = characters[(index + 1)...].firstIndex(of: "`") else { return nil }
                inner = String(characters[(index + 1)..<close])
                // An escaped backquote nests a substitution this cannot follow.
                guard !inner.contains("\\") else { return nil }
                index = close + 1
            } else {
                var level = 0
                var close: Int?
                for position in (index + 1)..<characters.count {
                    if characters[position] == "(" {
                        level += 1
                    } else if characters[position] == ")" {
                        level -= 1
                        if level == 0 {
                            close = position
                            break
                        }
                    }
                }
                guard let close else { return nil }
                inner = String(characters[(index + 2)..<close])
                index = close + 1
            }
            guard !mayWriteFiles(inner, depth: depth - 1, strict: strict) else { return nil }
            result += "x"
        }
        return result
    }

    /// The command without the redirections that land in no file: output thrown away or
    /// shown (`2>/dev/null`, `&> /dev/null`, `>/dev/stderr`), and one stream joined to or
    /// closing another (`2>&1`, `>&2`, `2>&-`).
    private static func withoutHarmlessRedirections(_ command: String) -> String {
        command
            .replacingOccurrences(of: #"(?:[0-9]+|&)?>{1,2}&?[ \t]*/dev/(?:null|stdout|stderr|tty)(?=$|[\s;&|)])"#, with: " ", options: .regularExpression)
            .replacingOccurrences(of: #"[0-9]*>&(?:[0-9]+|-)(?=$|[\s;&|)])"#, with: " ", options: .regularExpression)
    }

    /// A command line as the separate commands it runs, split at `&&`, `||`, `;`, `|`, `&`
    /// and line breaks outside quotes: `rg "TODO|FIXME"` is one command. A quote left open
    /// keeps the rest of the line in its part, which then reads as whatever it can.
    private static func parts(of command: String) -> [String] {
        var parts: [String] = []
        var current = ""
        var quote: Character?
        var escaped = false
        for character in command {
            if escaped {
                current.append(character)
                escaped = false
                continue
            }
            if let open = quote {
                if character == open { quote = nil }
                // A backslash escapes inside double quotes, never inside single ones.
                if character == "\\", open == "\"" { escaped = true }
                current.append(character)
                continue
            }
            switch character {
            case "'", "\"":
                quote = character
                current.append(character)
            case "\\":
                escaped = true
                current.append(character)
            case ";", "|", "&", "\n":
                parts.append(current)
                current = ""
            default:
                current.append(character)
            }
        }
        parts.append(current)
        return parts.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    private static func onlyReads(_ part: String, strict: Bool) -> Bool {
        // The snapshot's answer reads the words as they stand: a quote or an escape it misses
        // costs at worst a snapshot or an edit row.
        guard strict else { return onlyReads(words: part.split(whereSeparator: \.isWhitespace).map(String.init), strict: false) }
        // Unasked, the words are judged as the program will receive them, quotes and escapes
        // resolved: `'-exec'` and `-\exec` are find's `-exec`. A word the shell still expands
        // can become any option (`-c$IFS…` becomes git's `-c` and its value), so it may only
        // reach a tool none of whose arguments runs anything; and a part this cannot read
        // (an unclosed quote) asks.
        guard let arguments = shellArguments(part) else { return false }
        if arguments.contains(where: \.expands) {
            let toolIndex = arguments.firstIndex { word in
                let bare = unbracketed(word.text)
                return !bare.isEmpty && !shellWords.contains(bare) && !isAssignment(bare)
            }
            guard let toolIndex, !arguments[toolIndex].expands else { return false }
            let tool = (unbracketed(arguments[toolIndex].text) as NSString).lastPathComponent
            guard inertReaders.contains(tool) || expandsOnlyAsOperands(Array(arguments[(toolIndex + 1)...]), tool: tool) else { return false }
        }
        return onlyReads(words: arguments.map(\.text), strict: true)
    }

    /// Whether every word the shell still expands stands where the tool can only take it as
    /// an operand, never as an option: past git's or rg's own `--`, or as the folder git's
    /// `-C` names, quoted so it stays one word. `git log "$ref"` still asks: the ref could
    /// be `--output=…`.
    private static func expandsOnlyAsOperands(_ words: [ShellArgument], tool: String) -> Bool {
        guard tool == "git" || tool == "rg" else { return false }
        var pastOptions = false
        var beforeSubcommand = tool == "git"
        var folderNext = false
        for word in words {
            if folderNext {
                folderNext = false
                if word.splits { return false }
                continue
            }
            if pastOptions { continue }
            if word.expands { return false }
            if word.text == "--" {
                pastOptions = true
            } else if beforeSubcommand {
                if word.text == "-C" {
                    folderNext = true
                } else if !word.text.hasPrefix("-") {
                    beforeSubcommand = false
                }
            }
        }
        return true
    }

    /// A word of a command line as the shell hands it to the program, and whether the shell
    /// still changes it on the way (see `shellArguments`).
    private struct ShellArgument {
        var text: String
        /// A `$` expansion (outside single quotes, so `$'…'` too) or a brace list: what the
        /// word becomes is not known here.
        var expands: Bool
        /// Expanded outside double quotes, so it may also come apart into several words.
        var splits = false
    }

    /// A command's words with their quotes taken off and their backslash escapes resolved,
    /// as the program will receive them: `'--pre'=sh` is `--pre=sh`. Nil for a line with an
    /// unclosed quote. Globs are left as written: they expand to names already on disk. A
    /// comment ends the words.
    private static func shellArguments(_ part: String) -> [ShellArgument]? {
        let characters = Array(part)
        var words: [ShellArgument] = []
        var text = ""
        var expands = false
        var splits = false
        var inWord = false
        var quote: Character?
        var index = 0
        func finishWord() {
            if inWord { words.append(ShellArgument(text: text, expands: expands, splits: splits)) }
            text = ""
            expands = false
            splits = false
            inWord = false
        }
        while index < characters.count {
            let character = characters[index]
            index += 1
            if quote == "'" {
                if character == "'" { quote = nil } else { text.append(character) }
                continue
            }
            if quote == "\"" {
                if character == "\"" {
                    quote = nil
                } else if character == "\\", index < characters.count, "$`\"\\\n".contains(characters[index]) {
                    text.append(characters[index])
                    index += 1
                } else {
                    if character == "$" || character == "`" { expands = true }
                    text.append(character)
                }
                continue
            }
            if character.isWhitespace {
                finishWord()
                continue
            }
            // A `#` starting a word starts a comment, which runs nothing.
            if character == "#", !inWord { break }
            inWord = true
            switch character {
            case "'", "\"":
                quote = character
            case "\\":
                if index < characters.count {
                    text.append(characters[index])
                    index += 1
                }
            case "$", "`":
                expands = true
                splits = true
                text.append(character)
            case "{", "}":
                // A brace standing alone groups commands; one inside a word is a list the
                // shell expands (`{--pre=sh,x}`).
                let alone = text.isEmpty && (index == characters.count || characters[index].isWhitespace)
                if !alone {
                    expands = true
                    splits = true
                }
                text.append(character)
            default:
                text.append(character)
            }
        }
        guard quote == nil else { return nil }
        finishWord()
        return words
    }

    /// Whether the command these words make only reads. A wrapper (`env`, `time`, `xargs`
    /// and the like) is judged by the command it runs. `strict` also turns away what could
    /// make a reader run a program of the command's choosing (see `runsUnasked`).
    private static func onlyReads(words all: [String], strict: Bool) -> Bool {
        var words = withoutInputRedirections(all)[...]
        // The shell's own words, brackets and variable assignments ahead of the command run
        // nothing. An assignment still reaches the command (`GIT_EXTERNAL_DIFF`, `PAGER`,
        // `GIT_CONFIG_*` make git or a reader run a program), so under `strict` it asks.
        while let first = words.first {
            let bare = unbracketed(first)
            if isAssignment(bare) {
                if strict { return false }
            } else if !bare.isEmpty, !shellWords.contains(bare) {
                break
            }
            words = words.dropFirst()
        }
        guard let first = words.first else { return true }
        let tool = (unbracketed(first) as NSString).lastPathComponent
        let arguments = Array(words.dropFirst())
        if strict {
            if tool == "set" { return setIsHarmless(arguments) }
            if resolutionBuiltins.contains(tool) { return false }
            if pagingReaders.contains(tool), arguments.contains(where: { $0.hasPrefix("--pager") }) { return false }
            // `printf -v` assigns a variable, as `read` does.
            if tool == "printf", arguments.contains(where: { $0.hasPrefix("-v") }) { return false }
        }
        switch tool {
        case "sed":
            // Printing what it is told to, never editing in place (`-i`, or BSD's `-I`), and
            // from scripts that only print: a `w` command or flag writes a file of its naming,
            // and GNU sed's `e` runs one.
            let prints = arguments.contains { ($0.hasPrefix("-") && !$0.hasPrefix("--") && $0.contains("n")) || $0 == "--quiet" || $0 == "--silent" }
            let inPlace = arguments.contains { ($0.hasPrefix("-") && !$0.hasPrefix("--") && ($0.contains("i") || $0.contains("I"))) || $0.hasPrefix("--in-place") }
            guard prints, !inPlace, let scripts = sedScripts(arguments) else { return false }
            return scripts.allSatisfy(sedScriptOnlyPrints)
        case "awk", "gawk", "mawk", "nawk":
            // A program in a language with its own pipes to commands (`print | "cmd"`,
            // `"cmd" | getline`), `system` and writes: nothing this can read the whole of, so
            // it never runs unasked. For the snapshot, any trace of those in the program, a
            // program from a file this cannot read, or gawk's in-place editor makes it a writer.
            guard !strict else { return false }
            let program = words.joined(separator: " ")
            return !program.contains("system") && !program.contains("getline") && !program.contains("|")
                && !arguments.contains { $0.hasPrefix("-i") || $0.hasPrefix("-f") || $0.hasPrefix("-E") || $0.hasPrefix("--include") || $0.hasPrefix("--file") || $0.hasPrefix("--exec") }
        case "find", "fd":
            return !arguments.contains { searchActions.contains($0) || $0.hasPrefix("--exec") }
        case "sort":
            // `-o` writes; `--compress-program` runs a program of its naming.
            return !arguments.contains { ($0.hasPrefix("-") && !$0.hasPrefix("--") && $0.contains("o")) || $0.hasPrefix("--output") || $0.hasPrefix("--compress-program") }
        case "uniq":
            // `uniq in out` writes out.
            return positionals(in: arguments, valued: ["-f", "-s", "-w"]).count <= 1
        case "xxd":
            // `xxd in out` writes out.
            return positionals(in: arguments, valued: ["-c", "-cols", "-g", "-groupsize", "-l", "-len", "-n", "-name", "-o", "-offset", "-s", "-seek", "-R"]).count <= 1
        case "tree":
            // `-o` writes the listing to a file.
            return !arguments.contains { $0.hasPrefix("-") && !$0.hasPrefix("--") && $0.contains("o") }
        case "yq":
            // `-i` edits in place and `-s` splits into files.
            return !arguments.contains {
                ($0.hasPrefix("-") && !$0.hasPrefix("--") && ($0.contains("i") || $0.contains("s")))
                    || $0.hasPrefix("--inplace") || $0.hasPrefix("--in-place") || $0.hasPrefix("--split-exp")
            }
        case "rg":
            // A preprocessor is a command of its own, and so is the program `--hostname-bin`
            // names for its hyperlinks.
            return !arguments.contains { $0.hasPrefix("--pre") || $0.hasPrefix("--hostname-bin") }
        case "file":
            // `-C` compiles a magic file into the working directory.
            return !arguments.contains { $0.hasPrefix("-C") || $0 == "--compile" }
        case "plutil":
            // Printing or checking a property list (checking is what it does bare); every
            // other verb writes one.
            return arguments.allSatisfy { !$0.hasPrefix("-") || ["-p", "-lint", "-s", "-help"].contains($0) }
        case "defaults":
            guard let verb = arguments.first(where: { !$0.hasPrefix("-") }) else { return false }
            return defaultsReaders.contains(verb)
        case "git":
            return gitOnlyReads(arguments, strict: strict)
        case "xcodebuild":
            // Listing a project resolves its packages, which runs their manifests: nothing
            // to run unasked, though the working tree is left alone.
            guard !strict else { return arguments == ["-version"] || arguments == ["-showsdks"] }
            return arguments.contains { $0 == "-showBuildSettings" || $0 == "-list" || $0 == "-version" || $0 == "-showsdks" }
        case "man":
            // `-P` and `--pager` name the program the page goes through, `-H` a browser.
            guard strict else { return true }
            return !arguments.contains { $0.hasPrefix("-P") || $0.hasPrefix("--pager") || $0.hasPrefix("-H") || $0.hasPrefix("--html") }
        case "env":
            // Bare env prints the environment; with a command after it, that command runs.
            guard !arguments.contains(where: { $0.hasPrefix("-S") || $0.hasPrefix("--split-string") }),
                  let inner = wrappedCommand(in: arguments, valued: ["-u", "-C", "-P", "--unset", "--chdir"]) else { return false }
            return onlyReads(words: inner, strict: strict)
        case "time":
            guard let inner = wrappedCommand(in: arguments, allowed: ["-p", "-l"]) else { return false }
            return onlyReads(words: inner, strict: strict)
        case "nice":
            guard let inner = wrappedCommand(in: arguments, valued: ["-n"]) else { return false }
            return onlyReads(words: inner, strict: strict)
        case "timeout":
            // The first word past the options is how long; the command follows it.
            guard let inner = wrappedCommand(in: arguments, valued: ["-k", "-s", "--kill-after", "--signal"]) else { return false }
            return onlyReads(words: Array(inner.dropFirst()), strict: strict)
        case "command":
            // `command -v` says where a tool is; otherwise the tool after it runs.
            if arguments.contains(where: { $0 == "-v" || $0 == "-V" }) { return true }
            guard let inner = wrappedCommand(in: arguments, allowed: ["-p"]) else { return false }
            return onlyReads(words: inner, strict: strict)
        case "xargs":
            // With no command of its own, xargs echoes what it is given.
            guard let inner = wrappedCommand(in: arguments, valued: ["-n", "-I", "-P", "-L", "-E", "-J", "-s", "-R", "-S", "-d", "-a"]) else { return false }
            return onlyReads(words: inner, strict: strict)
        case "arch":
            // Bare arch names the architecture; with arguments it runs a command.
            return arguments.isEmpty
        case "for":
            // A loop's head, `for name` or `for name in words`: the words it goes over are
            // only read (a substitution among them was judged already), and the body, past a
            // `;` or a line, is a part of its own. zsh's short loops keep the body in the same
            // part (`for f (src/*) rm $f`, `for f in *; rm $f` aside), so any other shape of
            // head is judged as the writer it may hold.
            guard let name = arguments.first, !name.contains("("), !name.contains(")") else { return false }
            return arguments.count == 1
                || (arguments[1] == "in" && !arguments.contains { $0.contains("(") || $0.contains(")") })
        default:
            if versionReporters.contains(tool) { return arguments.count == 1 && versionFlags.contains(arguments[0]) }
            return readers.contains(tool)
        }
    }

    /// Git reads with the subcommands that leave the working tree alone, past git's own
    /// options, and never when it writes a patch to a file (`--output`) or opens matches in
    /// an editor (`grep -O`).
    private static func gitOnlyReads(_ arguments: [String], strict: Bool) -> Bool {
        var index = 0
        while index < arguments.count, arguments[index].hasPrefix("-") {
            let option = arguments[index]
            // `-c` with its value run on (`-cname=value`) is refused by git itself, and asked
            // about here all the same.
            if strict, option.hasPrefix("-c") || gitRedirectingOptions.contains(where: { option == $0 || option.hasPrefix($0 + "=") }) { return false }
            index += gitValuedOptions.contains(option) ? 2 : 1
        }
        // Options alone: `git --version`, `git -h`.
        guard index < arguments.count else { return true }
        let subcommand = arguments[index]
        let rest = Array(arguments[(index + 1)...])
        if rest.contains(where: { $0.hasPrefix("--output") || $0.hasPrefix("-O") || $0.hasPrefix("--open-files-in-pager") }) { return false }
        if strict, rest.contains(where: { word in gitProgramOptions.contains { word == $0 || word.hasPrefix($0 + "=") } }) { return false }
        if let forms = gitReadingForms[subcommand] {
            guard let form = rest.first(where: { !$0.hasPrefix("-") }) else { return false }
            return forms.contains(form)
        }
        guard gitReaders.contains(subcommand) else { return false }
        return !strict || gitReadsUnasked(subcommand, rest)
    }

    /// Whether a `set` only changes how the rest of the line fails or reads (see
    /// `harmlessSetFlags`): `set -e`, `set -euo pipefail`, `set +x`. Bare `set` prints the
    /// variables. `set -- …` resets the positional parameters, which nothing here reads.
    private static func setIsHarmless(_ arguments: [String]) -> Bool {
        var index = 0
        while index < arguments.count {
            let word = arguments[index]
            index += 1
            if word == "--" { return true }
            guard let sign = word.first, sign == "-" || sign == "+", word.count > 1 else { return false }
            let flags = word.dropFirst()
            guard flags.allSatisfy(harmlessSetFlags.contains) else { return false }
            // `o` takes the next word as the option's long name.
            if flags.contains("o") {
                guard index < arguments.count, harmlessSetOptions.contains(arguments[index]) else { return false }
                index += 1
            }
        }
        return true
    }

    /// The listing forms of the git subcommands that also write refs or config: what Auto
    /// mode runs unasked. `git config core.fsmonitor …` would have the next read run a
    /// program, `git branch -D` drops a branch, and `ls-remote --upload-pack` runs one
    /// outright, so outside their listing forms they ask. The working tree is untouched
    /// either way, so the snapshot's reading answer keeps them.
    private static func gitReadsUnasked(_ subcommand: String, _ rest: [String]) -> Bool {
        switch subcommand {
        case "ls-remote":
            // It talks to a remote, through whatever transport the config names.
            return false
        case "config":
            let reads: Set<String> = ["--get", "--get-all", "--get-regexp", "--get-urlmatch", "--list", "-l"]
            if rest.contains(where: { reads.contains($0) }) { return true }
            return ["get", "list"].contains(rest.first(where: { !$0.hasPrefix("-") }) ?? "")
        case "remote":
            guard let verb = rest.first(where: { !$0.hasPrefix("-") }) else { return true }
            return verb == "show" || verb == "get-url"
        case "branch", "tag":
            let writes: Set<String> = subcommand == "branch"
                ? ["-d", "-D", "--delete", "-m", "-M", "--move", "-c", "-C", "--copy", "-f", "--force", "-u",
                   "--set-upstream-to", "--unset-upstream", "--edit-description", "-t", "--track", "--no-track", "--create-reflog"]
                : ["-d", "--delete", "-a", "--annotate", "-s", "--sign", "-u", "--local-user", "-f", "--force",
                   "-m", "--message", "-F", "--file", "-e", "--edit", "--create-reflog"]
            if rest.contains(where: { word in writes.contains(word) || writes.contains { $0.hasPrefix("--") && word.hasPrefix($0 + "=") } }) { return false }
            // A name on its own makes one; with a listing option it filters the list.
            let filters: Set<String> = ["--contains", "--no-contains", "--merged", "--no-merged", "--points-at", "--sort", "--format", "--color", "--column"]
            let named = positionals(in: rest, valued: filters)
            return named.isEmpty || rest.contains("--list") || rest.contains("-l")
        case "reflog":
            let verb = rest.first(where: { !$0.hasPrefix("-") })
            return !["expire", "delete", "drop"].contains(verb ?? "")
        case "symbolic-ref":
            // With a second name it points the first at it; `-d` deletes it.
            if rest.contains(where: { $0 == "-d" || $0 == "--delete" }) { return false }
            return positionals(in: rest, valued: ["-m"]).count <= 1
        default:
            return true
        }
    }

    /// Every script a sed command runs: the `-e` and `--expression` ones, else the first
    /// word past the options. Nil when a script comes from a file (`-f`), which this cannot
    /// read, or when there is none.
    private static func sedScripts(_ arguments: [String]) -> [String]? {
        var scripts: [String] = []
        var index = 0
        var positional: String?
        while index < arguments.count {
            let word = arguments[index]
            let isShortCluster = word.hasPrefix("-") && !word.hasPrefix("--") && word.count > 1
            if word.hasPrefix("--file") || (isShortCluster && word.dropFirst().contains("f")) {
                return nil
            }
            // `-e script`, `--expression script`, or `-e` closing a cluster (`-ne script`);
            // a script run on from the cluster (`-e1p`) is the rest of the word.
            if word == "--expression" || (isShortCluster && word.dropFirst().contains("e")) {
                let rest = isShortCluster ? word.firstIndex(of: "e").map { String(word[word.index(after: $0)...]) } ?? "" : ""
                if !rest.isEmpty {
                    scripts.append(rest)
                    index += 1
                    continue
                }
                guard index + 1 < arguments.count else { return nil }
                scripts.append(arguments[index + 1])
                index += 2
                continue
            }
            if word.hasPrefix("--expression=") {
                scripts.append(String(word.dropFirst("--expression=".count)))
            } else if !word.hasPrefix("-"), positional == nil {
                positional = word
            }
            index += 1
        }
        if scripts.isEmpty, let positional { scripts.append(positional) }
        return scripts.isEmpty ? nil : scripts
    }

    /// Whether a sed script only picks lines out and prints them: addresses (line numbers,
    /// `$`, ranges, `/regex/`), the commands that print, hold or move along (`p`, `=`, `l`,
    /// `n`, `d`, `q`, the hold space), and `s` and `y` with no `w` or `e` flag. Everything
    /// else (`w` and `r` with their files, `e`, text and labels whose words this cannot
    /// bound) counts as more than printing. Quoting is only stripped at the ends; a script
    /// the shell split on a space reads as unfinished, which is the safe answer.
    private static func sedScriptOnlyPrints(_ script: String) -> Bool {
        var text = script
        for quote in ["'", "\""] where text.hasPrefix(quote) && text.hasSuffix(quote) && text.count >= 2 {
            text = String(text.dropFirst().dropLast())
        }
        let characters = Array(text)
        var index = 0
        // Past the delimited text that starts at `index`, escapes and all; nil if unclosed.
        func skipDelimited(_ delimiter: Character, from start: Int) -> Int? {
            var position = start
            while position < characters.count {
                if characters[position] == "\\" {
                    position += 2
                    continue
                }
                if characters[position] == delimiter { return position + 1 }
                position += 1
            }
            return nil
        }
        let quiet: Set<Character> = [" ", "\t", ";", "{", "}", "!", ",", "$", "~", "+", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
        let printing: Set<Character> = ["p", "P", "=", "l", "q", "Q", "n", "N", "d", "D", "g", "G", "h", "H", "x", "z"]
        let commandEnds: Set<Character> = [" ", "\t", ";", "}"]
        while index < characters.count {
            let character = characters[index]
            if quiet.contains(character) || printing.contains(character) {
                index += 1
            } else if character == "/" {
                guard let next = skipDelimited("/", from: index + 1) else { return false }
                index = next
                // `I` and `M` after an address regex change how it matches.
                while index < characters.count, characters[index] == "I" || characters[index] == "M" { index += 1 }
            } else if character == "\\", index + 1 < characters.count {
                guard let next = skipDelimited(characters[index + 1], from: index + 2) else { return false }
                index = next
            } else if character == "s" || character == "y", index + 1 < characters.count {
                let delimiter = characters[index + 1]
                guard delimiter != "\\", delimiter != "\n",
                      let middle = skipDelimited(delimiter, from: index + 2),
                      let end = skipDelimited(delimiter, from: middle) else { return false }
                index = end
                // An `s` command's flags, up to the end of the command: `w` writes the pattern
                // space to a file, `e` runs it.
                while index < characters.count, !commandEnds.contains(characters[index]) {
                    if characters[index] == "w" || characters[index] == "e" { return false }
                    index += 1
                }
            } else {
                return false
            }
        }
        return true
    }

    /// The command a wrapper runs: the words past its options, where a `valued` option takes
    /// the next word along; nil when it has an option outside `allowed`, when that is given.
    private static func wrappedCommand(in arguments: [String], valued: Set<String> = [], allowed: Set<String>? = nil) -> [String]? {
        var index = 0
        while index < arguments.count, arguments[index].hasPrefix("-"), arguments[index].count > 1 {
            let option = arguments[index]
            if valued.contains(option) {
                index += 2
                continue
            }
            if let allowed, !allowed.contains(option) { return nil }
            index += 1
        }
        return index < arguments.count ? Array(arguments[index...]) : []
    }

    /// The words that are neither an option nor an option's value.
    private static func positionals(in arguments: [String], valued: Set<String>) -> [String] {
        var found: [String] = []
        var index = 0
        while index < arguments.count {
            let word = arguments[index]
            if valued.contains(word) {
                index += 2
                continue
            }
            if !word.hasPrefix("-") || word == "-" { found.append(word) }
            index += 1
        }
        return found
    }

    /// The words without the input redirections among them: `< file` reads it, and
    /// `<<EOF` and `<<<` feed text in.
    private static func withoutInputRedirections(_ words: [String]) -> [String] {
        var kept: [String] = []
        kept.reserveCapacity(words.count)
        var skipsNext = false
        for word in words {
            if skipsNext {
                skipsNext = false
            } else if word == "<" || word == "<<" || word == "<<-" || word == "<<<" {
                skipsNext = true
            } else if !word.hasPrefix("<") {
                kept.append(word)
            }
        }
        return kept
    }

    /// `NAME=value` in front of a command sets a variable for it and runs nothing.
    private static func isAssignment(_ word: String) -> Bool {
        guard let equals = word.firstIndex(of: "="), equals != word.startIndex,
              let first = word.first, !first.isNumber else { return false }
        return word[..<equals].allSatisfy { $0 == "_" || ($0.isASCII && ($0.isLetter || $0.isNumber)) }
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
