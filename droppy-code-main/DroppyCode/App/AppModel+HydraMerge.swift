import AppKit
import Foundation

// With "Merge when the team is done" on, a Hydra lead's finished job lands by itself: the
// files the lead and its heads changed go out as a commit on a branch of their own, the
// branch is pushed, a merge request is opened and merged with the forge's CLI, and the
// checkout is brought up to date. The checkout never changes branch for any of it: the
// commit is built from a snapshot of the working tree, so nothing running in the checkout
// meanwhile is disturbed, and nothing uncommitted that is not the team's goes along.

/// A head whose work a merge takes, for the note the merge popover reads.
struct HydraMergeHead {
    var name: String
    var index: Int
    var task: String
    /// The head's files among the merge's, relative to the checkout.
    var files: [String]
}

extension AppModel {
    /// Lands a lead's finished work, when the setting says so. Runs once per finished job;
    /// what happened lands in the lead's timeline as a note from Hydra.
    func autoMergeHydraWork(of leadID: UUID) async {
        guard settings.hydraAutoMerge, let lead = thread(leadID), hydraIsOn(lead), let project = project(lead.projectID),
              let runtime = existingRuntime(for: leadID), !runtime.isHydraMerging else { return }
        runtime.isHydraMerging = true
        runtime.hydraMergeStartedAt = .now
        runtime.hydraMergeNoteID = UUID().uuidString
        runtime.hydraMergeStage = "Gathering the team's files"
        defer {
            runtime.isHydraMerging = false
            runtime.hydraMergeStage = nil
            runtime.hydraMergeStartedAt = nil
            runtime.hydraMergeNoteID = nil
        }
        // A head's work still going into the checkout finishes first: the snapshot the
        // commit is built from must hold the patch whole or not at all. Landings that have
        // not started yet wait for the merge instead (see `waitForSettledCheckout`).
        await runtime.hydraLanding?.value

        let checkout = lead.worktreePath ?? project.path
        let git = Git(checkout)
        guard await git.isRepository(), await git.hasCommits() else { return }
        guard await git.remoteURL() != nil else {
            note(leadID, "Hydra did not merge: no remote.", "The team's work is in the checkout; there is no origin to push it to.")
            return
        }
        guard !(await git.hasOperationInProgress()) else {
            note(leadID, "Hydra did not merge: a rebase or merge is underway.", "The team's work is in the checkout; finish that first and merge by hand.")
            return
        }

        // What the team touched, and only that, so a sibling's uncommitted work in the
        // same checkout stays behind.
        let work = await hydraWork(of: leadID, runtime: runtime, checkout: checkout, git: git)
        let sorted = work.paths
        guard !sorted.isEmpty else { return }
        // A head's landing can leave conflict markers behind (see `Git.apply`), and a lead
        // told not to look at git status may answer without settling them. Those markers
        // must never reach the remote: the job stays in the checkout until they are gone.
        let conflicted = await Self.pathsWithConflictMarkers(sorted, in: checkout)
        guard conflicted.isEmpty else {
            let files = conflicted.map { "`\($0)`" }.joined(separator: ", ")
            note(leadID, "Hydra did not merge: conflict markers in \(conflicted.count == 1 ? "a file" : "\(conflicted.count) files").", "\(files) still \(conflicted.count == 1 ? "holds" : "hold") conflict markers from a head's landing. Resolve them, then ask for the merge again.")
            return
        }

        do {
            let head = try await git.commitHash()
            runtime.hydraMergeStage = "Writing the commit"
            let tree = try await git.captureTree(paths: sorted)
            // Everything the team did is committed already, or was merged before.
            guard try await tree != git.treeHash(of: "HEAD") else { return }

            let patch = (try? await git.diff(from: head, to: tree)) ?? ""
            // A whole job's patch is big and parsing it counts every line: off the main
            // actor, so the chat stays live while the work goes out.
            let files = await Task.detached(priority: .utility) { DiffParser.parse(patch) }.value
            runtime.hydraMergeStage = "Writing the commit message"
            let message = await commitMessage(for: lead, files: files, patch: patch, directory: checkout)
            let subject = TextCleanup.singleLine(message, limit: 72)

            let status = await git.status()
            let target = await git.defaultBranch()
            let current = status?.branch
            // On the default branch the commit goes to a branch of its own, made and pushed
            // without ever being checked out; on any other branch it is that branch's.
            let ownBranch = current == nil || current == target
            let branch = ownBranch ? Self.hydraBranchName(for: lead) : current!
            let commit = try await git.commitWork(tree, parent: head, message: message)
            // Writing the branch to a commit built on `head` drops anything committed
            // since: a minute passes here, and the user may have committed in a terminal.
            // HEAD is read again so the user gets told in plain words when that happened,
            // and the move itself names the value it expects, so git refuses the write
            // outright if the branch shifts in the moment between the two.
            guard try await git.commitHash() == head else {
                note(leadID, "Hydra did not merge: the checkout moved while it worked.", "A commit landed on `\(current ?? target)` while the team's work was being prepared, and moving the branch now would orphan it. The work is still in the checkout; ask for the merge again.")
                return
            }
            // A branch of its own must not exist yet; the checkout's own branch must still
            // be where it was read.
            try await git.updateRef("refs/heads/\(branch)", to: commit, expecting: ownBranch ? nil : head)
            if !ownBranch {
                // The branch moved under the checkout: the index catches up, the working
                // tree already has the content.
                try await git.resetIndex(paths: sorted)
            }
            runtime.hydraMergeStage = "Pushing the branch"
            try await git.pushBranch(branch)

            let body = mergeRequestBody(for: lead, files: files, runtime: runtime)
            runtime.hydraMergeStage = "Opening the merge request"
            let requestURL: URL?
            do {
                requestURL = try await hydraCreateMergeRequest(git: git, title: subject, body: body, source: branch, target: target)
            } catch {
                note(leadID, "Hydra pushed \(branch) but could not open a merge request.", "\(error.localizedDescription)\n\nOpen one for `\(branch)` into `\(target)` and merge it from there. The work is still in the checkout.")
                return
            }
            guard let url = requestURL,
                  let link = MergeRequestLink(url: url) else {
                note(leadID, "Hydra pushed \(branch) but could not open a merge request.", "Open one for `\(branch)` into `\(target)` and merge it from there.")
                return
            }
            runtime.hydraMergeStage = "Merging"
            do {
                try await git.mergePullRequest(link)
            } catch {
                note(leadID, "Hydra opened \(link.label) but could not merge it.", "\(url.absoluteString)\n\n\(error.localizedDescription)\n\nMerge it from the link once it is ready; the branch `\(branch)` has the team's work.")
                return
            }

            // The work is on the default branch now, so these turns are spent: the next
            // job merges what comes after them and never this again.
            runtime.markHydraMerged(work.turnIDs)
            let mergedAt = Date.now
            for headID in work.headIDs { updateHydraHead(headID) { $0.mergedAt = mergedAt } }

            var lines = ["\(url.absoluteString)", "", Self.filesLine(files) + " landed on `\(target)` from `\(branch)`."]
            if !work.heads.isEmpty {
                lines.append("")
                lines.append("Heads")
                for head in work.heads {
                    if head.files.isEmpty {
                        lines.append("- \(head.name) (\(head.index)): \(head.task) — no files")
                    } else {
                        lines.append("- \(head.name) (\(head.index)): \(head.task) — \(head.files.count == 1 ? "1 file" : "\(head.files.count) files"): \(head.files.joined(separator: ", "))")
                    }
                }
            }
            // Files the team wrote outside the checkout (a lead's own memory, say) are
            // dropped without a word: they were never the project's to merge, and the
            // note about them read as if something had gone wrong.
            if work.droppedBuildOutputs > 0 {
                lines.append("\(work.droppedBuildOutputs) build-output paths a head's build left behind were left out.")
            }
            if work.droppedIgnored > 0 {
                lines.append("\(work.droppedIgnored) ignored paths were left out.")
            }
            if work.droppedMissing > 0 {
                lines.append("\(work.droppedMissing) missing paths were left out.")
            }
            if ownBranch {
                runtime.hydraMergeStage = "Bringing the checkout up to date"
                let synced = await syncDefaultBranch(git, from: head, target: target, ownPaths: sorted)
                lines.append(synced ? "The checkout is up to date." : "The checkout was left as it was: `\(target)` moved on in other ways meanwhile, or the team's files changed again; `git pull` when it suits you.")
            } else if lead.worktreePath != nil {
                // The lead worked in a worktree: the project's own checkout follows when it can.
                runtime.hydraMergeStage = "Bringing the checkout up to date"
                let main = Git(project.path)
                if await main.status()?.branch == target, await main.dirtyPaths().isEmpty, !(await main.hasOperationInProgress()) {
                    if (try? await main.pullFastForward()) != nil { lines.append("The project checkout is up to date.") }
                }
            }
            note(leadID, "Hydra merged \(link.label)", lines.joined(separator: "\n"))
            existingRuntime(for: leadID)?.noteDiffChanged()
        } catch {
            note(leadID, "Hydra could not merge the team's work.", "\(error.localizedDescription)\n\nThe work is still in the checkout.")
        }
    }

    /// Opens the merge request for a Hydra branch. On GitLab this wraps `glab mr create`
    /// rather than calling `Git.createPullRequest` straight: glab answers a missing or
    /// expired token with `401 Unauthorized` next to a recovery file, which used to land in
    /// the timeline as-is. The token is checked with `glab auth status` first, a 401
    /// re-checks it (it may have been refreshed while the push ran) and otherwise tells
    /// the user how to sign in again, and any other failure that left a recovery file is
    /// retried once with `--recover` so its saved options are picked up. Squash and
    /// remove-source-branch are left to the project's defaults, as before.
    private func hydraCreateMergeRequest(git: Git, title: String, body: String, source: String, target: String) async throws -> URL? {
        guard await git.forge() == .gitlab, LoginEnvironment.which("glab") != nil else {
            return try await git.createPullRequest(title: title, body: body, source: source, target: target)
        }
        let host = await git.remoteWebURL()?.host ?? "your GitLab host"
        // An explicit project keeps glab from resolving the wrong one; it names the same
        // origin remote glab would use on its own. Any credentials embedded in the remote
        // stay out of the arguments.
        var arguments = ["mr", "create", "--title", title, "--description", body, "--yes"]
        if var web = await git.remoteWebURL(), web.host != nil {
            if web.user != nil, var components = URLComponents(url: web, resolvingAgainstBaseURL: false) {
                components.user = nil
                components.password = nil
                if let stripped = components.url { web = stripped }
            }
            arguments += ["--repo", web.absoluteString]
        }
        arguments += ["--source-branch", source, "--target-branch", target]

        // No token on file means `mr create` can only fail with a 401: say how to sign in
        // instead of running into it.
        if let detail = await hydraGitLabAuthFailure(in: git.directory) {
            throw HydraMergeRequestError.gitLabAuth(host: host, branch: source, detail: detail)
        }

        var failure = try await Shell.run(tool: "glab", arguments, in: git.directory, timeout: 300)
        if failure.succeeded { return Self.hydraMergeRequestURL(from: failure) }
        if Self.isGitLabAuthFailure(failure) {
            // The token may have been refreshed while the push ran: check again before
            // giving up, and retry with the explicit options when it is good now.
            if await hydraGitLabAuthFailure(in: git.directory) == nil {
                let retry = try await Shell.run(tool: "glab", arguments, in: git.directory, timeout: 300)
                if retry.succeeded { return Self.hydraMergeRequestURL(from: retry) }
                failure = retry
            }
            if Self.isGitLabAuthFailure(failure) {
                throw HydraMergeRequestError.gitLabAuth(host: host, branch: source, detail: failure.failureMessage)
            }
        }
        // A failure that saved its options is retried once with `--recover`, which loads
        // them back from the recovery file.
        if Self.mentionsRecoverFile(failure) {
            let retry = try await Shell.run(tool: "glab", arguments + ["--recover"], in: git.directory, timeout: 300)
            if retry.succeeded { return Self.hydraMergeRequestURL(from: retry) }
            failure = retry
        }
        throw HydraMergeRequestError.creationFailed(failure.failureMessage)
    }

    /// The reason `glab auth status` gives for not being signed in, or nil when the token
    /// is good. Runs in the checkout so glab checks the host the merge goes to.
    private func hydraGitLabAuthFailure(in directory: URL) async -> String? {
        // A throw means glab itself could not run here; let the creation attempt surface
        // that. A non-zero exit is glab reporting it is not signed in.
        guard let status = try? await Shell.run(tool: "glab", ["auth", "status"], in: directory, timeout: 30) else { return nil }
        if status.succeeded { return nil }
        let message = status.failureMessage
        return message.isEmpty ? "glab is not signed in to this GitLab host." : message
    }

    /// Whether glab's output is an authentication refusal: the 401 the merge used to fail with.
    private static func isGitLabAuthFailure(_ result: ShellResult) -> Bool {
        let text = (result.output + "\n" + result.errorOutput).lowercased()
        return text.contains("401") || text.contains("unauthorized")
            || (text.contains("authentication") && (text.contains("failed") || text.contains("expired") || text.contains("invalid") || text.contains("required")))
    }

    /// Whether glab's output points at a recovery file its failed creation left behind.
    private static func mentionsRecoverFile(_ result: ShellResult) -> Bool {
        (result.output + "\n" + result.errorOutput).lowercased().contains("recover")
    }

    /// The merge request's page in glab's output, parsed the same way `Git` does.
    private static func hydraMergeRequestURL(from result: ShellResult) -> URL? {
        let text = result.output + "\n" + result.errorOutput
        let match = text.firstMatch(of: #/https://\S+/#)
        return match.flatMap { URL(string: String($0.output)) }
    }

    /// The files a lead's job changed, gathered three ways because no one way sees them
    /// all:
    ///
    /// - the turns of this job, meaning the ones no merge has taken yet, and the paths
    ///   they reported;
    /// - the heads that worked alongside those turns: what a Droppy-run head landed, and,
    ///   for a head the provider runs inside the lead's own session, the edits its own
    ///   timeline reported. A native head works in this very checkout and lands nothing,
    ///   so without its timeline its files are simply missing and the branch goes out
    ///   half-written;
    /// - and, where the first of those turns left a checkpoint, whatever else the checkout
    ///   holds that it did not then, less whatever was already uncommitted at the time.
    ///   That catches the edits no tool reported, a head writing through a shell say, and
    ///   costs nothing for a file whose content matches HEAD anyway.
    ///
    /// Paths outside the checkout are dropped rather than passed on: a lead writes to its
    /// own memory files, and `git add -A -- <path>` on one of those fails outright and
    /// takes the whole merge with it.
    private func hydraWork(of leadID: UUID, runtime: ThreadRuntime, checkout: String, git: Git) async -> (paths: [String], turnIDs: [UUID], headIDs: [UUID], dropped: Int, droppedBuildOutputs: Int, droppedIgnored: Int, droppedMissing: Int, heads: [HydraMergeHead]) {
        let turns = runtime.hydraUnmergedTurns
        var reported: [String] = turns.flatMap { $0.touchedPaths ?? [] }
        // A head counts until a merge has taken its work (see `HydraHeadInfo.mergedAt`).
        // Heads from before that mark existed have no mark: one that finished before the
        // last merged turn began went out with an earlier merge and is left alone; the
        // rest have work in the checkout that no merge has taken yet.
        let lastMergedTurnStart = runtime.turns.last { $0.hydraMerged }?.startedAt
        var headIDs: [UUID] = []
        var headPaths: [(name: String, index: Int, task: String, paths: [String])] = []
        for head in hydraTeam(of: leadID) {
            guard let info = head.hydra, info.mergedAt == nil else { continue }
            if let lastMergedTurnStart, let finished = info.finishedAt, finished < lastMergedTurnStart { continue }
            headIDs.append(head.id)
            var own: [String] = []
            var seen = Set<String>()
            func collect(_ path: String) {
                guard !path.isEmpty, let relative = TouchedPaths.relative(path, root: checkout), seen.insert(relative).inserted else { return }
                own.append(relative)
            }
            for file in info.landing?.files ?? [] { collect(file.path) }
            reported += (info.landing?.files ?? []).map(\.path)
            // A head's timeline runs to megabytes. One still open is read as it is; one
            // that has gone cold is decoded off the main actor rather than brought back
            // as a runtime, which would parse the whole history on the main thread for
            // every head of the team, one after another, while the chat sits frozen.
            let items: [TimelineItem]
            if let live = existingRuntime(for: head.id) {
                items = live.entries.map(\.item)
            } else {
                let headID = head.id
                items = await Task.detached(priority: .utility) { Storage.decodeDocument(headID)?.items ?? [] }.value
            }
            for item in items {
                guard case .tool(let call) = item.content else { continue }
                reported += call.edits.map(\.path)
                for edit in call.edits { collect(edit.path) }
            }
            headPaths.append((HydraRoster.persona(at: info.index).name, info.index, TextCleanup.singleLine(info.task, limit: 120), own))
        }

        var paths = Set<String>()
        var outside = Set<String>()
        for path in reported where !path.isEmpty {
            if let relative = TouchedPaths.relative(path, root: checkout) {
                paths.insert(relative)
            } else {
                outside.insert(path)
            }
        }

        if let base = turns.first?.baseCheckpoint,
           let alreadyDirty = try? await git.changedPaths(from: "HEAD", to: base),
           let now = try? await git.captureTree(),
           let changedSince = try? await git.changedPaths(from: base, to: now) {
            let theirs = Set(alreadyDirty)
            paths.formUnion(changedSince.filter { !theirs.contains($0) })
        }

        let buildOutputs = Set(paths.filter(TouchedPaths.isBuildOutput))
        paths.subtract(buildOutputs)
        let ignored = await git.ignoredPaths(among: paths.sorted())
        paths.subtract(ignored)
        let tracked = await git.trackedPaths(among: paths.sorted())
        let fileManager = FileManager.default
        let checkoutURL = URL(fileURLWithPath: checkout)
        let missing = Set(paths.filter { path in
            !fileManager.fileExists(atPath: checkoutURL.appendingPathComponent(path).path) && !tracked.contains(path)
        })
        paths.subtract(missing)
        let heads = headPaths.map { entry in
            HydraMergeHead(name: entry.name, index: entry.index, task: entry.task, files: entry.paths.filter { paths.contains($0) })
        }
        return (paths.sorted(), turns.map(\.id), headIDs, outside.count, buildOutputs.count, ignored.count, missing.count, heads)
    }

    /// Brings the checkout's default branch up to the merge without touching the working
    /// tree: the branch moves to what origin has, the index takes the merged version of the
    /// team's files (the working tree already holds it), and any other file the remote
    /// changed meanwhile is checked out only where it is clean locally. Returns false, and
    /// changes nothing, where that cannot be done safely.
    private func syncDefaultBranch(_ git: Git, from head: String, target: String, ownPaths: [String]) async -> Bool {
        let remote = "origin/\(target)"
        guard (try? await git.fetch()) != nil, await git.isAncestor(head, of: remote) else { return false }
        guard let changed = try? await git.changedPaths(from: head, to: remote) else { return false }
        let own = Set(ownPaths)
        let others = changed.filter { !own.contains($0) }
        let dirty = Set(await git.dirtyPaths())
        guard others.allSatisfy({ !dirty.contains($0) }) else { return false }
        // One git process answers for every path at once; a process per path took a
        // remote that had moved on by thousands of files minutes to sort.
        let present = await git.presentPaths(among: others, in: remote)
        let kept = others.filter { present.contains($0) }
        let gone = others.filter { !present.contains($0) }
        // `reset --soft` moves the branch with no expected old value, so a commit made in
        // a terminal while the merge ran would be left with nothing pointing at it. The
        // checkout is left alone instead, and `git pull` is the user's to run.
        guard (try? await git.commitHash()) == head else { return false }
        do {
            try await git.resetSoft(to: remote)
            try await git.resetIndex(paths: ownPaths)
            try await git.checkoutPaths(from: remote, kept)
            try await git.removePaths(gone)
            return true
        } catch {
            return false
        }
    }

    /// The commit message: written by the text engine from the patch, like the commit
    /// sheet's, else the chat's title over the heads' tasks.
    private func commitMessage(for lead: ChatThread, files: [DiffFile], patch: String, directory: String) async -> String {
        if let engine = textEngine(preferring: lead.provider) {
            let summary = files.map { "\($0.change == .added ? "A" : ($0.change == .deleted ? "D" : "M")) \($0.path)" }.joined(separator: "\n")
            if let text = await TextGeneration.commitMessage(
                summary: summary,
                patch: patch,
                instructions: settings.commitInstructions,
                engine: engine,
                directory: URL(fileURLWithPath: directory)
            ), !text.isEmpty {
                return text
            }
        }
        var lines = [lead.title == ChatThread.untitled ? "Hydra: the team's work" : lead.title, ""]
        for head in hydraTeam(of: lead.id) {
            guard let info = head.hydra, !info.task.isEmpty else { continue }
            lines.append("- \(info.persona.name): \(TextCleanup.singleLine(info.task, limit: 100))")
        }
        return lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The request's description: what the team did and how the lead summed it up.
    private func mergeRequestBody(for lead: ChatThread, files: [DiffFile], runtime: ThreadRuntime) -> String {
        var lines = ["Opened by Droppy Code once its Hydra team finished.", ""]
        let heads = hydraTeam(of: lead.id).compactMap(\.hydra).filter { !$0.task.isEmpty }
        if !heads.isEmpty {
            lines.append("## Heads")
            for info in heads {
                var line = "- **\(info.persona.name)**: \(TextCleanup.singleLine(info.task, limit: 120))"
                if let landing = info.landing, !landing.files.isEmpty {
                    line += " (\(landing.files.count == 1 ? "1 file" : "\(landing.files.count) files"))"
                }
                lines.append(line)
            }
            lines.append("")
        }
        if let reply = runtime.entries.last(where: { $0.kind == .assistant }).flatMap({ entry -> String? in
            guard case .assistant(let message) = entry.item.content else { return nil }
            return message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        }), !reply.isEmpty {
            lines.append("## The lead's summary")
            lines.append(reply.count > 2_000 ? String(reply.prefix(2_000)) + "…" : reply)
            lines.append("")
        }
        lines.append("## Files")
        lines += files.map { "- `\($0.path)` (+\($0.additions) −\($0.deletions))" }
        return lines.joined(separator: "\n")
    }

    private static func hydraBranchName(for lead: ChatThread) -> String {
        let title = lead.title == ChatThread.untitled ? "team" : lead.title
        var slug = title.lowercased().map { $0.isLetter || $0.isNumber ? String($0) : "-" }.joined()
        while slug.contains("--") { slug = slug.replacingOccurrences(of: "--", with: "-") }
        slug = slug.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        if slug.count > 40 { slug = String(slug.prefix(40)).trimmingCharacters(in: CharacterSet(charactersIn: "-")) }
        let suffix = String(UUID().uuidString.lowercased().prefix(6))
        return "hydra/\(slug.isEmpty ? "team" : slug)-\(suffix)"
    }

    /// The files among `paths` that hold a three-way merge's conflict markers: a line that
    /// opens with `<<<<<<< ` and one that opens with `>>>>>>> `. Read off the main actor;
    /// a file that is gone (a deletion the team made) or too large to be source is skipped.
    private nonisolated static func pathsWithConflictMarkers(_ paths: [String], in checkout: String) async -> [String] {
        await Task.detached(priority: .utility) {
            let root = URL(fileURLWithPath: checkout)
            let opening = Data("\n<<<<<<< ".utf8)
            let closing = Data("\n>>>>>>> ".utf8)
            return paths.filter { path in
                let url = root.appendingPathComponent(path)
                guard let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize, size <= 4_000_000,
                      let data = try? Data(contentsOf: url) else { return false }
                let body = Data([0x0A]) + data
                return body.range(of: opening) != nil && body.range(of: closing) != nil
            }
        }.value
    }

    private static func filesLine(_ files: [DiffFile]) -> String {
        let additions = files.reduce(0) { $0 + $1.additions }
        let deletions = files.reduce(0) { $0 + $1.deletions }
        return "\(files.count == 1 ? "1 file" : "\(files.count) files") (+\(additions) −\(deletions))"
    }

    /// A note from Hydra in the lead's timeline, and word of it when the chat is out of view.
    private func note(_ leadID: UUID, _ title: String, _ body: String) {
        // The note lands under the id the merging pill has been drawn with, so the timeline
        // keeps one row from the first stage to the outcome; the id is spent here, so a
        // second note in the same run would get one of its own.
        let runtime = existingRuntime(for: leadID)
        let id = runtime?.hydraMergeNoteID
        runtime?.hydraMergeNoteID = nil
        runtime?.appendHydraNote(title + "\n" + body, id: id)
        let onScreen = selectedThreadID == leadID
        guard !(NSApp.isActive && onScreen) else { return }
        updateThread(leadID) { $0.hasUnread = true }
        updateDockBadge()
        if settings.notifyWhenFinished, let lead = thread(leadID) {
            notify(threadID: leadID, title: lead.title, body: title)
        }
    }
}

/// Why a Hydra merge request could not be opened on GitLab.
private enum HydraMergeRequestError: LocalizedError {
    /// glab is not signed in, or GitLab answered 401: the token is missing or expired.
    case gitLabAuth(host: String, branch: String, detail: String)
    /// glab failed for any other reason, recovery retry included.
    case creationFailed(String)

    var errorDescription: String? {
        switch self {
        case .gitLabAuth(let host, let branch, let detail):
            return "GitLab refused the login (401 Unauthorized) for \(host): the token glab signs in with is missing or expired.\n\n\(detail)\n\nSign back in with `glab auth login --hostname \(host)` (or export a valid GITLAB_TOKEN), then ask for the merge again. The branch `\(branch)` is already pushed, so the team's work is safe."
        case .creationFailed(let message):
            return message
        }
    }
}
