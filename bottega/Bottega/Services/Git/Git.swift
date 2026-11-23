// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Synchronization

struct GitStatus: Equatable, Sendable {
    var branch: String?
    var upstream: String?
    var ahead = 0
    var behind = 0
    var changedFiles = 0
}

struct GitBranch: Identifiable, Hashable, Sendable {
    var name: String
    var isCurrent: Bool
    var upstream: String?

    var id: String { name }
}

/// A thin wrapper around the git command line. Every call runs off the main actor.
struct Git: Sendable {
    let directory: URL

    init(_ path: String) {
        directory = URL(fileURLWithPath: path)
    }

    private static var executable: URL { LoginEnvironment.git }

    @discardableResult
    func run(
        _ arguments: [String],
        environment extra: [String: String] = [:],
        input: Data? = nil,
        timeout: TimeInterval = 120
    ) async throws -> ShellResult {
        var environment = LoginEnvironment.current
        environment["GIT_TERMINAL_PROMPT"] = "0"
        environment["GIT_OPTIONAL_LOCKS"] = "0"
        environment.merge(extra) { _, new in new }
        return try await Shell.run(
            Self.executable,
            ["-c", "core.quotepath=false", "-c", "color.ui=false"] + arguments,
            in: directory,
            environment: environment,
            input: input,
            timeout: timeout
        )
    }

    func output(_ arguments: [String]) async throws -> String {
        let result = try await run(arguments)
        try Self.check(result)
        return result.output
    }

    static func check(_ result: ShellResult) throws {
        if !result.succeeded { throw ShellError(result.failureMessage) }
    }

    // MARK: - Repository

    func isRepository() async -> Bool {
        (try? await run(["rev-parse", "--is-inside-work-tree"]))?.trimmedOutput == "true"
    }

    func repositoryRoot() async throws -> URL {
        let prefix = try await output(["rev-parse", "--show-prefix"])
        var root = directory
        for _ in prefix.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: "/") {
            root.deleteLastPathComponent()
        }
        return root
    }

    /// Where this directory sits in its repository, as git names it: nothing at the root,
    /// `sub/dir/` (slash and all) below it.
    func checkoutPrefix() async throws -> String {
        try await output(["rev-parse", "--show-prefix"]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func hasCommits() async -> Bool {
        (try? await run(["rev-parse", "--verify", "--quiet", "HEAD"]))?.succeeded ?? false
    }

    /// HEAD's tree, nil on an unborn branch, and a throw where git could not say: unlike
    /// `hasCommits`, which reads any failure as no commits. A snapshot must not take that
    /// for an answer, since an index started empty for want of HEAD writes a tree without
    /// every tracked file the snapshot does not walk.
    func headTree() async throws -> String? {
        let result = try await run(["rev-parse", "--verify", "--quiet", "HEAD^{tree}"])
        if result.succeeded { return result.trimmedOutput }
        // `--verify --quiet` exits 1, saying nothing, for a name that resolves to nothing;
        // anything else (128 outside a repository) is git failing.
        guard result.status == 1 else { throw ShellError(result.failureMessage) }
        return nil
    }

    /// The names `write-tree` gives a tree with nothing in it, in SHA-1 and SHA-256
    /// repositories. It is also what `write-tree` answers, successfully, over an index
    /// file that does not exist.
    static let emptyTrees: Set<String> = [
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
        "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321",
    ]

    func status() async -> GitStatus? {
        guard let result = try? await run(["status", "--porcelain=v2", "--branch"]), result.succeeded else { return nil }
        var status = GitStatus()
        for line in result.output.split(separator: "\n") {
            if let head = line.value(after: "# branch.head ") {
                status.branch = head == "(detached)" ? nil : head
            } else if let upstream = line.value(after: "# branch.upstream ") {
                status.upstream = upstream
            } else if let counts = line.value(after: "# branch.ab ") {
                for token in counts.split(separator: " ") {
                    if token.hasPrefix("+") { status.ahead = Int(token.dropFirst()) ?? 0 }
                    if token.hasPrefix("-") { status.behind = Int(token.dropFirst()) ?? 0 }
                }
            } else if !line.hasPrefix("#") {
                status.changedFiles += 1
            }
        }
        return status
    }

    func branches() async -> [GitBranch] {
        let format = "%(HEAD)%09%(refname:short)%09%(upstream:short)"
        guard let text = try? await output(["for-each-ref", "--sort=-committerdate", "--format=\(format)", "refs/heads"]) else {
            return []
        }
        return text.split(separator: "\n").compactMap { line in
            let parts = line.split(separator: "\t", omittingEmptySubsequences: false)
            guard parts.count >= 2 else { return nil }
            let upstream = parts.count > 2 && !parts[2].isEmpty ? String(parts[2]) : nil
            return GitBranch(name: String(parts[1]), isCurrent: parts[0] == "*", upstream: upstream)
        }
    }

    func switchBranch(_ name: String) async throws {
        try Self.check(await run(["switch", name]))
    }

    /// Puts HEAD on the branch `name` without touching the index or the working tree: for a
    /// branch made from HEAD plus what the working tree holds, which the checkout then
    /// stands on as it is (the caller brings the index of those paths along).
    func pointHead(at name: String) async throws {
        try Self.check(await run(["symbolic-ref", "HEAD", "refs/heads/\(name)"]))
    }

    /// Detaches HEAD at `commit`, whatever branch it stands on, moving no branch: undoes
    /// `pointHead` for a checkout that was detached before it.
    func detachHead(at commit: String) async throws {
        try Self.check(await run(["update-ref", "--no-deref", "HEAD", commit]))
    }

    func createBranch(_ name: String) async throws {
        try Self.check(await run(["switch", "-c", name]))
    }

    func addWorktree(at path: String, branch: String, base: String?) async throws {
        var arguments = ["worktree", "add", "-b", branch, path]
        if let base { arguments.append(base) }
        try Self.check(await run(arguments))
    }

    func removeWorktree(at path: String) async throws {
        try Self.check(await run(["worktree", "remove", "--force", path]))
    }

    func remoteURL() async -> String? {
        guard let text = try? await output(["remote", "get-url", "origin"]) else { return nil }
        return text.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
    }

    /// The https page for the origin remote, when it is a hosted repository.
    func remoteWebURL() async -> URL? {
        guard var remote = await remoteURL() else { return nil }
        if remote.hasSuffix(".git") { remote.removeLast(4) }
        if remote.hasPrefix("git@"), let colon = remote.firstIndex(of: ":") {
            let host = remote[remote.index(remote.startIndex, offsetBy: 4)..<colon]
            remote = "https://\(host)/\(remote[remote.index(after: colon)...])"
        } else if remote.hasPrefix("ssh://git@") {
            remote = "https://" + remote.dropFirst("ssh://git@".count)
        }
        return remote.hasPrefix("http") ? URL(string: remote) : nil
    }

    func listFiles() async -> [String] {
        guard let text = try? await output(["ls-files", "--cached", "--others", "--exclude-standard"]) else { return [] }
        return text.split(separator: "\n").map(String.init)
    }

    // MARK: - Commit and publish

    func workingChanges() async -> (summary: String, patch: String) {
        let summary = (try? await output(["status", "--short"])) ?? ""
        var patch = (try? await output(["diff", "HEAD", "--no-ext-diff"])) ?? ""
        if patch.utf8.count > 40_000 { patch = String(patch.prefix(40_000)) }
        return (summary, patch)
    }

    func commitAll(message: String) async throws {
        try Self.check(await run(["add", "-A"]))
        try Self.check(await run(["commit", "-F", "-"], input: Data(message.utf8)))
    }

    func push() async throws {
        try Self.check(await run(["push", "-u", "origin", "HEAD"], timeout: 300))
    }

    /// Opens a pull request (GitHub) or merge request (GitLab) for the current branch, or
    /// for `source` into `target` when those are given.
    func createPullRequest(title: String, body: String, source: String? = nil, target: String? = nil) async throws -> URL? {
        let result: ShellResult
        switch await forge() {
        case .gitlab:
            var arguments = ["mr", "create", "--title", title, "--description", body, "--yes"]
            if let source { arguments += ["--source-branch", source] }
            if let target { arguments += ["--target-branch", target] }
            result = try await Shell.run(tool: "glab", arguments, in: directory, timeout: 300)
        case .github:
            var arguments = ["pr", "create", "--title", title, "--body", body]
            if let source { arguments += ["--head", source] }
            if let target { arguments += ["--base", target] }
            result = try await Shell.run(tool: "gh", arguments, in: directory, timeout: 300)
        case .gitea:
            var arguments = ["pulls", "create", "--title", title, "--description", body]
            if let source { arguments += ["--head", source] }
            if let target { arguments += ["--base", target] }
            result = try await Shell.run(tool: "tea", arguments, in: directory, timeout: 300)
        }
        try Self.check(result)
        let text = result.output + "\n" + result.errorOutput
        let match = text.firstMatch(of: #/https://\S+/#)
        return match.flatMap { URL(string: String($0.output)) }
    }

    /// Which forge the origin remote is on, by its host: GitLab and GitHub by name, and
    /// Gitea's family (tea serves them all) for anything else.
    func forge() async -> MergeRequestLink.Forge {
        let remote = (await remoteURL() ?? "").lowercased()
        if remote.contains("gitlab") { return .gitlab }
        if remote.contains("github") { return .github }
        return .gitea
    }

    /// Lands a request with the forge's own CLI, squashed into one commit on GitLab and
    /// GitHub alike. GitLab answers 405 while it is still checking a fresh request, so that
    /// is tried again a few times. `deleteSourceBranch` removes the request's branch once
    /// it is in: meant for a branch made for the request alone. On GitHub it also deletes
    /// the local branch, and moves the checkout off it first when the checkout is on it,
    /// so a branch someone is working on passes false.
    /// Merges a request: squashed into one commit, or with a merge commit when `squash` is
    /// false (the caller's call, for a branch whose history must stay as it is).
    func mergePullRequest(_ link: MergeRequestLink, deleteSourceBranch: Bool = true, squash: Bool = true) async throws {
        let number = String(link.number)
        var lastFailure = "The request could not be merged."
        for attempt in 0..<6 {
            let result: ShellResult
            switch link.forge {
            case .gitlab: result = try await Shell.run(tool: "glab", ["mr", "merge", number, "--yes"] + (squash ? ["--squash"] : []) + (deleteSourceBranch ? ["--remove-source-branch"] : []), in: directory, timeout: 300)
            case .github: result = try await Shell.run(tool: "gh", ["pr", "merge", number, squash ? "--squash" : "--merge"] + (deleteSourceBranch ? ["--delete-branch"] : []), in: directory, timeout: 300)
            case .gitea: result = try await Shell.run(tool: "tea", ["pulls", "merge", number], in: directory, timeout: 300)
            }
            if result.succeeded { return }
            lastFailure = result.failureMessage
            let text = (result.output + result.errorOutput).lowercased()
            guard link.forge == .gitlab, text.contains("405") || text.contains("not allowed") || text.contains("checking") else { break }
            try? await Task.sleep(for: .seconds(3 + attempt * 2))
        }
        throw ShellError(lastFailure)
    }

    // MARK: - Landing a team's work

    /// The branch the remote treats as its default: what origin's HEAD points at, else
    /// main or master where one exists.
    func defaultBranch() async -> String {
        if let head = try? await output(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).trimmingCharacters(in: .whitespacesAndNewlines),
           !head.isEmpty {
            return head.hasPrefix("origin/") ? String(head.dropFirst("origin/".count)) : head
        }
        for candidate in ["main", "master"] where (try? await run(["rev-parse", "--verify", "--quiet", "refs/remotes/origin/\(candidate)"]))?.succeeded == true {
            return candidate
        }
        return "main"
    }

    /// A tree of HEAD with only `paths` taken from the working tree, additions and
    /// deletions included: what one thread's work amounts to, with nothing else that
    /// happens to be uncommitted alongside it. HEAD is read in only once git has said
    /// whether there is one: read as no commits on a failure, the index started empty and
    /// a commit of the tree deleted every file outside `paths`.
    func captureTree(paths: [String]) async throws -> String {
        let index = FileManager.default.temporaryDirectory.appendingPathComponent("bottega-index-\(UUID().uuidString)")
        defer { Self.discardIndex(index) }
        let environment = ["GIT_INDEX_FILE": index.path]
        if try await headTree() != nil { try Self.check(await run(["read-tree", "HEAD"], environment: environment)) }
        try Self.check(await run(["add", "-A", "--ignore-errors", "--pathspec-from-file=-", "--pathspec-file-nul"], environment: environment, input: Self.nulSeparated(paths), timeout: 300))
        let result = try await run(["write-tree"], environment: environment)
        try Self.check(result)
        return result.trimmedOutput
    }

    func treeHash(of ref: String) async throws -> String {
        try await output(["rev-parse", "\(ref)^{tree}"]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func commitHash(of ref: String = "HEAD") async throws -> String {
        try await output(["rev-parse", "--verify", ref]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// A commit of `tree` on `parent` in the user's own name, for a branch to carry.
    func commitWork(_ tree: String, parent: String, message: String) async throws -> String {
        let result = try await run(["commit-tree", tree, "-p", parent, "-F", "-"], input: Data(message.utf8))
        try Self.check(result)
        return result.trimmedOutput
    }

    func updateRef(_ ref: String, to commit: String) async throws {
        try Self.check(await run(["update-ref", ref, commit]))
    }

    /// Moves `ref` only while it still points at `old`, so a branch someone else moved in
    /// the meantime is left where it is rather than run over. `old` nil means the ref must
    /// not exist yet.
    func updateRef(_ ref: String, to commit: String, expecting old: String?) async throws {
        try Self.check(await run(["update-ref", ref, commit, old ?? ""]))
    }

    /// Commits `paths` (relative to this directory) as they stand in the working tree, on
    /// top of HEAD, and moves HEAD's branch to the commit. Built the way a team merge is:
    /// a tree of HEAD with only `paths` taken from the working tree, so nothing else that
    /// is staged or changed goes along, and the move names the commit HEAD was read at, so
    /// a commit made meanwhile (in a terminal, by another chat) makes git refuse rather
    /// than be run over. The index of `paths` then catches up with the commit; the working
    /// tree is never touched. Paths that are ignored, or neither on disk nor in HEAD, are
    /// left out: git cannot stage them. Returns the commit, or nil when `paths` hold
    /// nothing HEAD does not already have.
    func commitPaths(_ paths: [String], message: String) async throws -> String? {
        let ignored = await ignoredPaths(among: paths)
        let tracked = await trackedPaths(among: paths)
        let usable = paths.filter { path in
            !ignored.contains(path)
                && (tracked.contains(path) || FileManager.default.fileExists(atPath: directory.appendingPathComponent(path).path))
        }
        guard !usable.isEmpty else { return nil }
        let head = try await commitHash()
        let tree = try await captureTree(paths: usable)
        guard try await tree != treeHash(of: head) else { return nil }
        let commit = try await commitWork(tree, parent: head, message: message)
        try await updateRef("HEAD", to: commit, expecting: head)
        do {
            try await resetIndex(paths: usable)
        } catch {
            throw ShellError("Committed \(commit.prefix(7)), but the index did not catch up: \(error.localizedDescription) `git reset -q -- <the files>` settles it.")
        }
        return commit
    }

    func pushBranch(_ name: String) async throws {
        try Self.check(await run(["push", "-u", "origin", "refs/heads/\(name):refs/heads/\(name)"], timeout: 300))
    }

    func fetch() async throws {
        try Self.check(await run(["fetch", "--quiet", "origin"], timeout: 300))
    }

    func isAncestor(_ commit: String, of other: String) async -> Bool {
        (try? await run(["merge-base", "--is-ancestor", commit, other]))?.succeeded ?? false
    }

    /// Paths that differ between two commits or trees, as they are on disk: NUL-separated,
    /// so a name with a quote or a tab in it comes back unquoted and usable as a path.
    func changedPaths(from: String, to: String) async throws -> [String] {
        try await output(["diff", "--name-only", "-z", from, to]).split(separator: "\0").map(String.init)
    }

    /// Paths with uncommitted changes, staged or not, untracked ones included.
    func dirtyPaths() async -> [String] {
        guard let text = try? await output(["status", "--porcelain=v1", "-z", "--untracked-files=all"]) else { return [] }
        var records = text.split(separator: "\0", omittingEmptySubsequences: false)[...]
        var paths: [String] = []
        while let entry = records.popFirst() {
            guard entry.count > 3 else { continue }
            let code = entry.prefix(2)
            paths.append(String(entry.dropFirst(3)))
            // A rename or a copy is two records: the new name, then the name it came from.
            // That second record is not an entry of its own, and both names are dirty.
            if code.contains("R") || code.contains("C"), let origin = records.popFirst(), !origin.isEmpty {
                paths.append(String(origin))
            }
        }
        return paths
    }

    /// Paths from `paths` that the checkout ignores.
    func ignoredPaths(among paths: [String]) async -> Set<String> {
        guard !paths.isEmpty, let result = try? await run(["check-ignore", "-z", "--stdin"], input: Self.nulSeparated(paths)), result.succeeded || result.status == 1 else { return [] }
        return Set(result.output.split(separator: "\0", omittingEmptySubsequences: true).map(String.init))
    }

    /// Paths from `paths` that HEAD holds. That is what `captureTree(paths:)` reads its
    /// index from, so a path in HEAD can be staged even once it is gone from disk (the
    /// deletion is the work), while a path in neither is a pathspec that matches nothing
    /// and takes the whole `git add` down. `ls-files` reads no pathspec file, so the
    /// check goes through `cat-file --batch-check`, one `HEAD:<path>` per line on stdin,
    /// which answers in order with `missing` for what HEAD does not have.
    func trackedPaths(among paths: [String]) async -> Set<String> {
        await presentPaths(among: paths, in: "HEAD")
    }

    /// Paths from `paths` that `ref` holds, answered by one `cat-file --batch-check` over
    /// the lot rather than a `cat-file -e` per path: a branch that moved on by thousands
    /// of files (a build's output merged once by mistake, say) used to mean thousands of
    /// git processes one after another, with the main actor picking each one up in turn.
    func presentPaths(among paths: [String], in ref: String) async -> Set<String> {
        guard !paths.isEmpty else { return [] }
        let input = paths.reduce(into: Data()) { data, path in
            data.append(contentsOf: "\(ref):\(path)".utf8)
            data.append(0)
        }
        guard let result = try? await run(["cat-file", "--batch-check=%(objecttype)", "-Z"], input: input), result.succeeded else { return [] }
        let answers = result.output.split(separator: "\0", omittingEmptySubsequences: false)
        var tracked = Set<String>()
        for (path, answer) in zip(paths, answers) where !answer.hasSuffix(" missing") {
            tracked.insert(path)
        }
        return tracked
    }

    private static func nulSeparated(_ paths: [String]) -> Data {
        paths.reduce(into: Data()) { data, path in
            data.append(contentsOf: path.utf8)
            data.append(0)
        }
    }

    /// Whether a rebase, merge or cherry-pick is underway: nothing may move then.
    ///
    /// Every marker is per-worktree, so one `rev-parse --git-dir` names where all of them
    /// live — a linked worktree's own git directory, where git writes them, not the shared
    /// one — and the rest is six `fileExists` calls. Asking `--git-path` per marker was six
    /// git processes on every landing and every merge. A directory git cannot be asked
    /// about, or one that is no repository at all, counts as busy: callers use the answer
    /// to keep out of a checkout's way, and a state nobody can read is no reason to write
    /// into it. (`GIT_DIR`, when the environment sets it, is honoured by `--git-dir` too.)
    func hasOperationInProgress() async -> Bool {
        guard let gitDirectory = try? await output(["rev-parse", "--path-format=absolute", "--git-dir"]).trimmingCharacters(in: .whitespacesAndNewlines),
              !gitDirectory.isEmpty else { return true }
        for marker in ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "BISECT_LOG"] {
            if FileManager.default.fileExists(atPath: gitDirectory + "/" + marker) { return true }
        }
        return false
    }

    /// Moves the current branch to `commit`; index and working tree stay as they are.
    func resetSoft(to commit: String) async throws {
        try Self.check(await run(["reset", "--soft", "--quiet", commit]))
    }

    /// The index takes HEAD's version of `paths`; the working tree stays as it is.
    func resetIndex(paths: [String]) async throws {
        guard !paths.isEmpty else { return }
        try Self.check(await run(["reset", "--quiet", "--pathspec-from-file=-", "--pathspec-file-nul"], input: Self.nulSeparated(paths)))
    }

    /// Index and working tree take `ref`'s version of `paths`.
    func checkoutPaths(from ref: String, _ paths: [String]) async throws {
        guard !paths.isEmpty else { return }
        try Self.check(await run(["checkout", "--quiet", ref, "--"] + paths))
    }

    func removePaths(_ paths: [String]) async throws {
        guard !paths.isEmpty else { return }
        try Self.check(await run(["rm", "--quiet", "--"] + paths))
    }

    func pullFastForward() async throws {
        try Self.check(await run(["pull", "--ff-only", "--quiet"], timeout: 300))
    }

    // MARK: - Checkpoints

    /// Where every thread's checkpoints live, a folder of refs per thread.
    static let checkpointRoot = "refs/bottega/checkpoints/"

    /// Where a thread's checkpoints live: two hidden refs per turn, `<turn>-start` and
    /// `<turn>-end`, each keeping its snapshot commit from being collected.
    static func checkpointPrefix(thread: UUID) -> String {
        checkpointRoot + "\(thread.uuidString.lowercased())/"
    }

    static func checkpointRef(thread: UUID, turn: Int, phase: String) -> String {
        checkpointPrefix(thread: thread) + "\(turn)-\(phase)"
    }

    /// The turn a checkpoint ref was taken for, read off its name; nil for a name that
    /// carries none.
    static func checkpointTurn(of ref: String) -> Int? {
        guard let name = ref.split(separator: "/").last, let dash = name.firstIndex(of: "-") else { return nil }
        return Int(name[..<dash])
    }

    /// The thread a checkpoint ref was taken for, read off its name; nil for a ref outside
    /// the checkpoints or one whose folder names no thread.
    static func checkpointThread(of ref: String) -> UUID? {
        guard ref.hasPrefix(checkpointRoot) else { return nil }
        let rest = ref.dropFirst(checkpointRoot.count)
        guard let slash = rest.firstIndex(of: "/") else { return nil }
        return UUID(uuidString: String(rest[..<slash]))
    }

    /// The git directory every worktree of this repository shares, as an absolute path: it
    /// names the repository whichever of its checkouts or folders git is asked from, so two
    /// projects in one repository answer alike. Nil when git finds no repository here;
    /// throws when git could not answer at all.
    func commonDirectory() async throws -> String? {
        let result = try await run(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        // A folder in no repository, or in one git refuses to read, is a fatal error to
        // git, which exits 128 then; any other failure is git itself not working.
        if result.status == 128 { return nil }
        try Self.check(result)
        let path = result.trimmedOutput
        return path.isEmpty ? nil : path
    }

    /// Whether `ref` still names a commit here. A checkpoint pruned with its turn, or let go
    /// with its archived thread, does not, whatever the turn's record still says.
    func resolves(_ ref: String) async -> Bool {
        (try? await run(["rev-parse", "--verify", "--quiet", "\(ref)^{commit}"]))?.succeeded ?? false
    }

    /// Snapshots the working tree into a hidden ref without touching the user's index or
    /// branch. A tree already checkpointed since anything changed reuses its commit, so a
    /// turn starting on an untouched tree costs the `update-ref` alone.
    func captureCheckpoint(_ ref: String) async throws {
        let tree = try await currentTree()
        let watch = await watch()
        let commit: String
        if let known = watch?.commit(of: tree) {
            commit = known
        } else {
            let result = try await run(["commit-tree", tree, "-m", "Bottega checkpoint"], environment: Self.identity)
            try Self.check(result)
            commit = result.trimmedOutput
            watch?.remember(commit: commit, of: tree)
        }
        try Self.check(await run(["update-ref", ref, commit]))
    }

    /// The working tree as a tree object, from the directory's watch when it can vouch
    /// that nothing changed since the last capture, else captured now. A capture the watch
    /// runs that fails, or whose tree cannot be vouched for (`vouch`), is taken again whole
    /// into an index of its own (see `WorkingTreeWatch.currentTree`); only that failing too
    /// throws, and the caller goes without the tree rather than use a wrong one.
    func currentTree() async throws -> String {
        guard let indexPath = await indexPath(), let watch = await watch() else { return try await captureTree() }
        return try await watch.currentTree(indexPath: indexPath) { index, paths in
            guard let index else { return try await captureTree() }
            return try await captureTree(into: index, paths: paths)
        }
    }

    /// A capture into `index`, an index file of the caller's own that nothing else writes
    /// or deletes meanwhile. With `paths`, the directories changed since the capture that
    /// wrote the file (the watch hands over a copy of its scratch index), only they are
    /// walked and the rest of the index stands. Without them, or when that walk fails (a
    /// path vanished with nothing of it in the index), the file is started afresh
    /// (`seedIndex`) and the whole directory walked. The tree is vouched for (`vouch`)
    /// before it is handed back.
    private func captureTree(into index: URL, paths: [String]?) async throws -> String {
        let environment = ["GIT_INDEX_FILE": index.path]
        var walkedChanges = false
        if let paths, !paths.isEmpty, FileManager.default.fileExists(atPath: index.path) {
            walkedChanges = (try? await run(["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"], environment: environment, input: Self.nulSeparated(paths), timeout: 300))?.succeeded == true
        }
        if !walkedChanges {
            Self.discardIndex(index)
            try await seedIndex(index, environment: environment)
            try Self.check(await run(["add", "-A", "--", "."], environment: environment, timeout: 300))
        }
        let result = try await run(["write-tree"], environment: environment)
        try Self.check(result)
        let tree = result.trimmedOutput
        try await vouch(for: tree)
        return tree
    }

    /// Starts `index`, a fresh index file of a capture's own: from the checkout's index,
    /// whose stat cache spares git re-hashing every unchanged file, else from HEAD's tree,
    /// and empty only on an unborn branch. Whatever git or the copy cannot do throws
    /// rather than leave the file empty: a capture from an empty index writes a tree
    /// without every tracked file it does not walk (the ignored ones, and those outside
    /// the directory), and a revert to that tree deletes them as if a turn had made them.
    private func seedIndex(_ index: URL, environment: [String: String]) async throws {
        guard let indexPath = await indexPath() else {
            throw ShellError("git could not say where this checkout's index is.")
        }
        if FileManager.default.fileExists(atPath: indexPath) {
            try FileManager.default.copyItem(atPath: indexPath, toPath: index.path)
        } else if try await headTree() != nil {
            try Self.check(await run(["read-tree", "HEAD"], environment: environment))
        }
    }

    /// Throws when `tree`, just written from a capture's own index, is the empty tree
    /// although the capture would have put files in it: there are working files git adds,
    /// or the index the capture started from (`seedIndex`: the checkout's, else HEAD's
    /// tree) lists a file still on disk, one git keeps without looking (skip-worktree), or
    /// one outside the directory, which the capture never walks and so keeps. `write-tree`
    /// over an index file that went missing answers with the empty tree and succeeds;
    /// taken for a turn's start checkpoint, that tree once had undoing the turn delete
    /// files that had been there all along. A checkout emptied for real holds none of
    /// these, and its empty tree stands: a turn that deleted every file then records the
    /// deletions, and undoing it brings them back. Costs nothing unless the tree is empty.
    private func vouch(for tree: String) async throws {
        guard Self.emptyTrees.contains(tree) else { return }
        let others = try await run(["ls-files", "-z", "--others", "--exclude-standard"])
        try Self.check(others)
        guard others.stdout.isEmpty else { throw ShellError(Self.implausiblyEmptyMessage) }
        guard let indexPath = await indexPath() else {
            throw ShellError("git could not say where this checkout's index is.")
        }
        // Each name is full (from the top of the checkout), with a tag of its own ahead of
        // it where git gives one ("S" for skip-worktree).
        let seeded: [(tag: Substring?, name: Substring)]
        if FileManager.default.fileExists(atPath: indexPath) {
            let listed = try await run(["ls-files", "-z", "-t", "--cached", "--full-name", "--", ":/"])
            try Self.check(listed)
            seeded = listed.output.split(separator: "\0").map { record in
                guard let space = record.firstIndex(of: " ") else { return (nil, record) }
                return (record[..<space], record[record.index(after: space)...])
            }
        } else if try await headTree() != nil {
            let listed = try await run(["ls-tree", "-r", "-z", "--name-only", "--full-tree", "HEAD"])
            try Self.check(listed)
            seeded = listed.output.split(separator: "\0").map { (nil, $0) }
        } else {
            return
        }
        guard !seeded.isEmpty else { return }
        let root = try await repositoryRoot()
        let walked = directory.path.hasSuffix("/") ? directory.path : directory.path + "/"
        for (tag, name) in seeded {
            let file = root.appendingPathComponent(String(name)).path
            // `attributesOfItem` does not follow a symlink: a tracked link whose target is
            // gone is still a file git keeps.
            if tag == "S" || !file.hasPrefix(walked) || (try? FileManager.default.attributesOfItem(atPath: file)) != nil {
                throw ShellError(Self.implausiblyEmptyMessage)
            }
        }
    }

    private static let implausiblyEmptyMessage = "A snapshot of the checkout came back empty although the checkout holds files, so it was not used."

    /// Removes a capture's index file, and the lock a git killed mid-write leaves beside it.
    static func discardIndex(_ index: URL) {
        try? FileManager.default.removeItem(at: index)
        try? FileManager.default.removeItem(atPath: index.path + ".lock")
    }

    private func watch() async -> WorkingTreeWatch? {
        guard let indexPath = await indexPath() else { return nil }
        return WorkingTreeWatch.shared(for: directory.path, gitDirectory: (indexPath as NSString).deletingLastPathComponent)
    }

    /// The working tree as a tree object: a cheap before/after marker for a
    /// diff, with no commit and nothing referenced. Respects .gitignore. Walks the whole
    /// directory into an index of its own, used once and thrown away.
    func captureTree() async throws -> String {
        let index = FileManager.default.temporaryDirectory.appendingPathComponent("bottega-index-\(UUID().uuidString)")
        defer { Self.discardIndex(index) }
        return try await captureTree(into: index, paths: nil)
    }

    /// Whether `ref`, a checkpoint, holds the empty tree while HEAD's tree is not empty:
    /// a start checkpoint written over a snapshot index deleted under it (see
    /// `WorkingTreeWatch.tearDown`; checkpoints taken since are vouched for as they are
    /// taken). It knows of no file the checkout had, so it cannot tell a file a turn made
    /// from one that was there all along, and nothing may be deleted or swept on its word.
    /// Throws when git could not say.
    func isEmptyWhileHeadIsNot(_ ref: String) async throws -> Bool {
        let checkpoint = try await treeHash(of: ref)
        guard Self.emptyTrees.contains(checkpoint) else { return false }
        guard let head = try await headTree() else { return false }
        return !Self.emptyTrees.contains(head)
    }

    /// Why files are not put back from a checkpoint that holds nothing while HEAD holds files.
    static let emptyCheckpointMessage = "This turn's file checkpoint is empty although the current commit holds files, so it cannot tell which files the turn created; the files are left as they are."

    /// What a worktree's index path was resolved from: its `.git` file, which names the
    /// git directory, and the environment overrides git would honour over it.
    struct GitFileStamp: Equatable {
        var identifier: UInt64?
        var modified: Date?
        var size: Int?
        var overrides: String

        init(gitFile: String, environment: [String: String]) {
            let attributes = try? FileManager.default.attributesOfItem(atPath: gitFile)
            identifier = (attributes?[.systemFileNumber] as? NSNumber)?.uint64Value
            modified = attributes?[.modificationDate] as? Date
            size = attributes?[.size] as? Int
            overrides = (environment["GIT_DIR"] ?? "") + "\u{0}" + (environment["GIT_INDEX_FILE"] ?? "")
        }
    }

    /// Resolved index paths of worktrees by working directory. A session works in a
    /// handful of directories; past the bound the cache is simply dropped.
    static let worktreeIndexPaths = Mutex<[String: (stamp: GitFileStamp, path: String?)]>([:])

    /// The repository's index file. A checkout with its own `.git` directory keeps it right
    /// there, which saves the git process that asking would cost on every snapshot. A
    /// worktree (`.git` is a file naming the git directory) asks once and keeps the answer
    /// while that file stands unchanged: a snapshot asks three or four times, and every
    /// command tool takes two snapshots. A directory inside a repository, with no `.git`
    /// of its own, still asks every time, since the answer depends on its ancestors.
    private func indexPath() async -> String? {
        var isDirectory: ObjCBool = false
        let dotGit = directory.appendingPathComponent(".git").path
        let environment = LoginEnvironment.current
        let exists = FileManager.default.fileExists(atPath: dotGit, isDirectory: &isDirectory)
        if exists, isDirectory.boolValue, environment["GIT_DIR"] == nil, environment["GIT_INDEX_FILE"] == nil {
            return dotGit + "/index"
        }
        let stamp = exists ? GitFileStamp(gitFile: dotGit, environment: environment) : nil
        if let stamp, let cached = Self.worktreeIndexPaths.withLock({ $0[directory.path] }), cached.stamp == stamp {
            return cached.path
        }
        let path = try? await output(["rev-parse", "--path-format=absolute", "--git-path", "index"])
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let stamp {
            Self.worktreeIndexPaths.withLock { paths in
                if paths.count >= 64 { paths.removeAll() }
                paths[directory.path] = (stamp, path)
            }
        }
        return path
    }

    /// Paths per `git diff` call when a caller names many: well under the argument limit.
    private static let diffPathChunk = 400

    /// The patch between two trees or refs. `binary` puts whole binary blobs in it, so a
    /// picture a hand added applies elsewhere. Over `paths` alone when given: a diff of a
    /// tree that holds build output runs to hundreds of megabytes, so the caller names
    /// the files it wants rather than cutting the rest out afterwards.
    func diff(from: String, to: String, binary: Bool = false, paths: [String]? = nil) async throws -> String {
        // `diff` takes no pathspec file, so the paths go on the command line, and as
        // plain names: a `?` or `[` in one is a character, not a pattern. Long lists go in
        // chunks: a command that touched thousands of files would otherwise blow the
        // argument limit, and one diff per chunk joins into the same patch.
        var arguments = paths == nil ? [] : ["--literal-pathspecs"]
        arguments += ["diff", "--no-ext-diff", "-M"]
        if binary { arguments.append("--binary") }
        arguments += [from, to]
        guard let paths, paths.count > Self.diffPathChunk else {
            if let paths { arguments += ["--"] + paths }
            let result = try await run(arguments, timeout: 300)
            try Self.check(result)
            return result.output
        }
        var output = ""
        var start = 0
        while start < paths.count {
            let chunk = Array(paths[start..<min(start + Self.diffPathChunk, paths.count)])
            let result = try await run(arguments + ["--"] + chunk, timeout: 300)
            try Self.check(result)
            if !output.isEmpty, !output.hasSuffix("\n") { output += "\n" }
            output += result.output
            start += chunk.count
        }
        return output
    }

    /// What undoing a stretch of a thread's turns would do to each of `paths`, the files the
    /// thread's agent changed in the stretch as its turns recorded them (from the checkout,
    /// or from the repository's root), or to those of them `only` names, as git names them.
    /// `base` is the checkpoint before the stretch, `end` the one after it. Only the
    /// stretch's own changes come out: a file changed again since keeps those later changes
    /// where a three-way merge can tell them apart (see `MergeFile`), and is left exactly as
    /// it is where it cannot. Other threads' files never appear: only `paths` are looked at.
    /// A `base` that holds nothing while HEAD holds files throws (see `isEmptyWhileHeadIsNot`):
    /// every file would read as the stretch's own creation, due for deletion. The files come
    /// back as git names them, from the root (see `RevertPreview.names`).
    func previewFileRevert(base: String, end: String, paths: [String], only: [String]? = nil) async throws -> RevertPreview {
        try await planFileRevert(base: base, end: end, paths: paths, only: only).preview
    }

    /// Undoes the stretch's changes to `paths`, or to those `only` names (see
    /// `previewFileRevert`), judged again against the files as they are now, and answers
    /// with what was done to each: a file that could not be written is reported as left
    /// alone. The index follows where it still holds what the stretch left (see `resetIndex`).
    func revertFiles(base: String, end: String, paths: [String], only: [String]? = nil) async throws -> RevertPreview {
        var plan = try await planFileRevert(base: base, end: end, paths: paths, only: only)
        let root = plan.root
        var restoredIndex: [String: RevertSide?] = [:]
        for index in plan.preview.files.indices where plan.preview.files[index].outcome == .revert {
            let path = plan.preview.files[index].path
            guard let step = plan.steps[path] else { continue }
            do {
                try Self.apply(step, at: root.appendingPathComponent(path))
                // Kept even when the base had no such file: nil then means "out of the index".
                restoredIndex.updateValue(plan.base[path], forKey: path)
            } catch {
                plan.preview.files[index].outcome = .conflict
            }
        }
        try await resetIndex(restoredIndex, stagedAs: plan.end, root: root)
        return plan.preview
    }

    /// One file as a checkpoint holds it.
    private struct RevertSide: Equatable, Sendable {
        let mode: String
        let blob: String

        var isLink: Bool { mode == "120000" }
        /// A submodule's commit, not a file: nothing to read or write.
        var isGitlink: Bool { mode == "160000" }
        var isExecutable: Bool { mode == "100755" }
    }

    /// What writing one file back takes.
    private enum RevertStep: Sendable {
        /// The content, and the permissions to leave it with.
        case write(Data, permissions: Int)
        case link(String)
        case delete
    }

    private struct RevertPlan: Sendable {
        var preview: RevertPreview
        var steps: [String: RevertStep]
        var base: [String: RevertSide]
        var end: [String: RevertSide]
        var root: URL
    }

    private func planFileRevert(base: String, end: String, paths: [String], only wanted: [String]?) async throws -> RevertPlan {
        let root = try await repositoryRoot()
        let prefix = try await checkoutPrefix()
        guard !paths.isEmpty else { return RevertPlan(preview: RevertPreview(files: [], checkoutPrefix: prefix), steps: [:], base: [:], end: [:], root: root) }
        if try await isEmptyWhileHeadIsNot(base) { throw ShellError(Self.emptyCheckpointMessage) }
        // Every file the stretch changed, as git names it from the root: one diff of two
        // checkpoint trees, cheap whatever the repository's size. The stretch's own paths
        // then pick from it, whichever way its turns recorded them, from the checkout or
        // from the root (see `RevertPreview.names`): a name that fits either way is the
        // file, and a file of the same name elsewhere is never taken for it. So no pathspec:
        // a path from the checkout would have to be told from one from the root first.
        let changedInStretch = try RevertPreview(numstat: await output(["diff", "--numstat", "-z", "--no-renames", "--no-ext-diff", base, end]), checkoutPrefix: prefix)
        var preview = changedInStretch
        preview.files = changedInStretch.files.filter { file in
            paths.contains { changedInStretch.names(file.path, as: $0) }
                && (wanted?.contains { changedInStretch.names(file.path, as: $0) } ?? true)
        }
        guard !preview.files.isEmpty else { return RevertPlan(preview: preview, steps: [:], base: [:], end: [:], root: root) }
        let changed = preview.files.map(\.path)
        let before = try await sides(in: base, at: changed, root: root)
        let after = try await sides(in: end, at: changed, root: root)
        var steps: [String: RevertStep] = [:]
        for index in preview.files.indices {
            let file = preview.files[index]
            let (outcome, step) = try await judge(file, before: before[file.path], after: after[file.path], base: base, end: end, root: root)
            preview.files[index].outcome = outcome
            if let step { steps[file.path] = step }
        }
        return RevertPlan(preview: preview, steps: steps, base: before, end: after, root: root)
    }

    /// What undoing the stretch does to one file, and the write that does it.
    private func judge(_ file: RevertPreview.File, before: RevertSide?, after: RevertSide?, base: String, end: String, root: URL) async throws -> (RevertPreview.File.Outcome, RevertStep?) {
        let url = root.appendingPathComponent(file.path)
        switch (before, after) {
        case (nil, nil):
            return (.unchanged, nil)
        case (nil, let made?):
            // The stretch made it: it goes only while it is still exactly as the stretch left it.
            guard Self.exists(url) else { return (.unchanged, nil) }
            return try await matches(url, made) ? (.revert, .delete) : (.conflict, nil)
        case (let gone?, nil):
            // The stretch deleted it: it comes back only where nothing has taken its place.
            guard !gone.isGitlink else { return (.conflict, nil) }
            guard Self.exists(url) else { return (.revert, try await step(for: gone, in: base, path: file.path)) }
            return try await matches(url, gone) ? (.unchanged, nil) : (.conflict, nil)
        case (let old?, let new?):
            guard Self.exists(url), !old.isGitlink, !new.isGitlink else { return (.conflict, nil) }
            if try await matches(url, new) { return (.revert, try await step(for: old, in: base, path: file.path)) }
            if try await matches(url, old) { return (.unchanged, nil) }
            // Changed again since: only a text file's lines can be told apart.
            guard !file.isBinary, !old.isLink, !new.isLink, !Self.isLink(url) else { return (.conflict, nil) }
            // A merge git cannot do (a file it takes for binary after all, one too large
            // for it) leaves this file alone, not the whole stretch undone.
            guard let merged = try? await MergeFile.merge(
                base: content(of: end, path: file.path),
                ours: Data(contentsOf: url),
                theirs: content(of: base, path: file.path)
            ), !merged.hasConflicts else { return (.conflict, nil) }
            // The file stays the one it is now, its permissions included.
            let permissions = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.posixPermissions] as? Int ?? 0o644
            return (.revert, .write(merged.merged, permissions: permissions))
        }
    }

    /// The write that puts `side` back as the checkpoint `ref` holds it (never a gitlink:
    /// the callers leave those alone).
    private func step(for side: RevertSide, in ref: String, path: String) async throws -> RevertStep {
        if side.isLink { return .link(String(decoding: try await blob(side.blob), as: UTF8.self)) }
        return .write(try await content(of: ref, path: path), permissions: side.isExecutable ? 0o755 : 0o644)
    }

    /// Whether the file at `url` is what `side` holds: the same link target, or the same blob
    /// as git would hash the file's content to (with the checkout's filters, as a capture
    /// hashed it).
    private func matches(_ url: URL, _ side: RevertSide) async throws -> Bool {
        if side.isGitlink { return false }
        if side.isLink {
            guard Self.isLink(url), let target = try? FileManager.default.destinationOfSymbolicLink(atPath: url.path) else { return false }
            return Data(target.utf8) == (try await blob(side.blob))
        }
        guard !Self.isLink(url) else { return false }
        let result = try await run(["hash-object", "--", url.path])
        try Self.check(result)
        return result.trimmedOutput == side.blob
    }

    /// A file's content as the checkout would hold it, filters applied, from the tree `ref`.
    private func content(of ref: String, path: String) async throws -> Data {
        let result = try await run(["cat-file", "--filters", "\(ref):\(path)"], timeout: 120)
        try Self.check(result)
        return result.stdout
    }

    private func blob(_ hash: String) async throws -> Data {
        let result = try await run(["cat-file", "blob", hash])
        try Self.check(result)
        return result.stdout
    }

    /// The mode and blob of each of `paths` (repository-relative) in `ref`, for the ones it has.
    private func sides(in ref: String, at paths: [String], root: URL) async throws -> [String: RevertSide] {
        let listing = try await output(["--literal-pathspecs", "ls-tree", "-r", "-z", "--full-name", ref, "--"] + paths.map { root.appendingPathComponent($0).path })
        var sides: [String: RevertSide] = [:]
        sides.reserveCapacity(paths.count)
        for entry in listing.split(separator: "\0") {
            // "<mode> <type> <hash>\t<path>"
            guard let tab = entry.firstIndex(of: "\t") else { continue }
            let fields = entry[..<tab].split(separator: " ")
            guard fields.count == 3 else { continue }
            sides[String(entry[entry.index(after: tab)...])] = RevertSide(mode: String(fields[0]), blob: String(fields[2]))
        }
        return sides
    }

    /// Puts each of `paths` back in the index as `restored` has it (nil: out of the index),
    /// but only where the index still holds what the stretch left (`stagedAs`, the file as
    /// the stretch's end checkpoint has it, or nothing where it deleted the file): an agent
    /// that staged its change, or `git rm`ed the file, has that undone with the change, and
    /// whatever was staged otherwise is nobody's to undo.
    private func resetIndex(_ restored: [String: RevertSide?], stagedAs end: [String: RevertSide], root: URL) async throws {
        guard !restored.isEmpty else { return }
        let top = Git(root.path)
        let listing = try await top.output(["--literal-pathspecs", "ls-files", "-s", "-z", "--"] + restored.keys.sorted())
        var staged: [String: RevertSide] = [:]
        for entry in listing.split(separator: "\0") {
            // "<mode> <hash> <stage>\t<path>"
            guard let tab = entry.firstIndex(of: "\t") else { continue }
            let fields = entry[..<tab].split(separator: " ")
            guard fields.count == 3, fields[2] == "0" else { continue }
            staged[String(entry[entry.index(after: tab)...])] = RevertSide(mode: String(fields[0]), blob: String(fields[1]))
        }
        // NUL-terminated, so a path with a tab or a newline in it reaches git whole.
        var lines = ""
        for (path, side) in restored.sorted(by: { $0.key < $1.key }) {
            let now = staged[path]
            // The index holds what the stretch left: its file, or no entry where the
            // stretch took the file out of the index too.
            guard now == end[path] else { continue }
            if let side {
                lines += "\(side.mode) \(side.blob)\t\(path)\0"
            } else if let now {
                // Mode 0 takes the entry out; the hash is as long as the repository's.
                lines += "0 \(String(repeating: "0", count: now.blob.count))\t\(path)\0"
            }
        }
        guard !lines.isEmpty else { return }
        try Self.check(await top.run(["update-index", "-z", "--index-info"], input: Data(lines.utf8)))
    }

    /// Writes one file back: through a copy renamed into place, so a reader never sees half
    /// of it; a link replaces whatever stood at the path.
    private static func apply(_ step: RevertStep, at url: URL) throws {
        let manager = FileManager.default
        switch step {
        case .delete:
            try manager.removeItem(at: url)
        case .link(let target):
            try manager.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            if exists(url) { try manager.removeItem(at: url) }
            try manager.createSymbolicLink(atPath: url.path, withDestinationPath: target)
        case .write(let data, let permissions):
            try manager.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            if isLink(url) { try manager.removeItem(at: url) }
            try data.write(to: url, options: .atomic)
            try manager.setAttributes([.posixPermissions: permissions], ofItemAtPath: url.path)
        }
    }

    /// Whether anything stands at `url`, a link whose target is gone included.
    private static func exists(_ url: URL) -> Bool {
        (try? FileManager.default.attributesOfItem(atPath: url.path)) != nil
    }

    private static func isLink(_ url: URL) -> Bool {
        (try? FileManager.default.attributesOfItem(atPath: url.path))?[.type] as? FileAttributeType == .typeSymbolicLink
    }

    /// Every checkpoint ref held here under `prefix` (one thread's, or `checkpointRoot` for
    /// every thread's), with the commit each points at; nil when git could not list them.
    private func checkpoints(under prefix: String) async -> [(ref: String, commit: String)]? {
        guard let listing = try? await output(["for-each-ref", "--format=%(objectname) %(refname)", prefix]) else {
            return nil
        }
        // The line's type is spelled out: left to inference, `split` on a String also reads
        // as Sequence's, into `ArraySlice<Character>`, and the index arithmetic below then
        // fits both and is ambiguous.
        return listing.split(separator: "\n").compactMap { (line: Substring) -> (ref: String, commit: String)? in
            guard let space = line.firstIndex(of: " ") else { return nil }
            return (ref: String(line[line.index(after: space)...]), commit: String(line[..<space]))
        }
    }

    func deleteCheckpoints(thread: UUID) async {
        guard let refs = await checkpoints(under: Self.checkpointPrefix(thread: thread)) else { return }
        let commands = refs.map { "delete \($0.ref)\n" }.joined()
        guard !commands.isEmpty else { return }
        _ = try? await run(["update-ref", "--stdin"], input: Data(commands.utf8))
    }

    /// Lets go of every checkpoint ref held here for any of `threads`, from one listing of
    /// every thread's and in one transaction: a repository with many threads costs two git
    /// processes, not two per thread. Every other thread's refs are left alone, and each of
    /// these goes only if it still points where it was listed, so a ref written again
    /// meanwhile stays and the transaction then fails whole. Whether none of theirs is left:
    /// false when git could not list them or the transaction failed.
    func deleteCheckpoints(threads: Set<UUID>) async -> Bool {
        guard let refs = await checkpoints(under: Self.checkpointRoot) else { return false }
        let commands = refs.compactMap { entry -> String? in
            guard let thread = Self.checkpointThread(of: entry.ref), threads.contains(thread) else { return nil }
            return "delete \(entry.ref) \(entry.commit)\n"
        }.joined()
        guard !commands.isEmpty else { return true }
        return (try? await run(["update-ref", "--stdin"], input: Data(commands.utf8)))?.succeeded ?? false
    }

    /// Lets go of a thread's checkpoints taken before turn `firstKept`, all but `preserved`,
    /// in one transaction. Each goes only if it still points where it was listed: a turn
    /// that took its number again meanwhile (a revert frees the numbers after it) keeps its
    /// fresh snapshot, and the transaction then fails whole, to be tried again after the
    /// next turn. A name without a turn in it is left alone.
    func pruneCheckpoints(thread: UUID, keepingTurnsFrom firstKept: Int, preserving preserved: Set<String>) async {
        guard let refs = await checkpoints(under: Self.checkpointPrefix(thread: thread)) else { return }
        let commands = refs.compactMap { entry -> String? in
            guard !preserved.contains(entry.ref), let turn = Self.checkpointTurn(of: entry.ref), turn < firstKept else { return nil }
            return "delete \(entry.ref) \(entry.commit)\n"
        }.joined()
        guard !commands.isEmpty else { return }
        _ = try? await run(["update-ref", "--stdin"], input: Data(commands.utf8))
    }

    // MARK: - Copies of the checkout

    private static let identity = [
        "GIT_AUTHOR_NAME": "Bottega", "GIT_AUTHOR_EMAIL": "bottega@localhost",
        "GIT_COMMITTER_NAME": "Bottega", "GIT_COMMITTER_EMAIL": "bottega@localhost",
    ]

    /// A commit of `tree` on top of HEAD that no ref points at: the checkout as it is,
    /// uncommitted work included, with the history behind it. Something a worktree can
    /// start from.
    func commitTree(_ tree: String, message: String) async throws -> String {
        var arguments = ["commit-tree", tree, "-m", message]
        if await hasCommits() { arguments += ["-p", "HEAD"] }
        let result = try await run(arguments, environment: Self.identity)
        try Self.check(result)
        return result.trimmedOutput
    }

    /// A worktree at `path` with `commit` checked out and no branch: a copy of the
    /// checkout to work in, whose own status shows only what changed in it.
    func addDetachedWorktree(at path: String, commit: String) async throws {
        try Self.check(await run(["worktree", "add", "--detach", path, commit], timeout: 300))
    }

    /// Forgets worktrees whose folders are gone from disk.
    func pruneWorktrees() async {
        _ = try? await run(["worktree", "prune"])
    }

    /// Applies `patch` to the working tree and stages nothing. A hunk that no longer fits
    /// is merged three-way against the blobs the patch names; a file the merge cannot
    /// settle keeps conflict markers, and those paths come back. Throws when nothing
    /// could be applied at all.
    func apply(_ patch: String) async throws -> [String] {
        let data = Data(patch.utf8)
        let plain = try await run(["apply", "--binary", "--whitespace=nowarn", "-"], input: data)
        if plain.succeeded { return [] }

        // The three-way merge works through an index that matches the working tree, so
        // it gets a throwaway one that does, and the real index never changes.
        let fileManager = FileManager.default
        let index = fileManager.temporaryDirectory.appendingPathComponent("bottega-apply-\(UUID().uuidString)")
        defer { try? fileManager.removeItem(at: index) }
        let environment = ["GIT_INDEX_FILE": index.path]
        if let indexPath = await indexPath(), fileManager.fileExists(atPath: indexPath) {
            try? fileManager.copyItem(atPath: indexPath, toPath: index.path)
        } else if await hasCommits() {
            try Self.check(await run(["read-tree", "HEAD"], environment: environment))
        }
        try Self.check(await run(["add", "-A", "--", "."], environment: environment, timeout: 300))
        let merged = try await run(["apply", "--3way", "--binary", "--whitespace=nowarn", "-"], environment: environment, input: data)
        if merged.succeeded { return [] }
        // "U path" names each file left with markers; without one, nothing was applied.
        let conflicts = merged.errorOutput.split(separator: "\n").compactMap { line -> String? in
            line.hasPrefix("U ") ? String(line.dropFirst(2)).trimmingCharacters(in: .whitespaces) : nil
        }
        guard !conflicts.isEmpty else { throw ShellError(merged.failureMessage) }
        return conflicts
    }
}

private extension Substring {
    func value(after prefix: String) -> String? {
        hasPrefix(prefix) ? String(dropFirst(prefix.count)) : nil
    }
}
