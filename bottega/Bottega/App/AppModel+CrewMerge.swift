// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Foundation

// With "Merge when the team is done" on, a Crew lead's finished job lands by itself: the
// files the lead and its hands changed go out as a commit on a branch of their own, the
// branch is pushed, a merge request is opened and merged with the forge's CLI, and the
// checkout is brought up to date. The checkout never changes branch for any of it: the
// commit is built from a snapshot of the working tree, so nothing running in the checkout
// meanwhile is disturbed, and nothing uncommitted that is not the team's goes along. How
// far it goes is the user's choice (`CrewMergeMode`): that whole way by default, as far as
// the pushed branch, or no further than a commit on the checkout's own branch.

/// A hand whose work a merge takes, for the note the merge popover reads.
struct CrewMergeHand {
    var name: String
    var index: Int
    var task: String
    /// The hand's files among the merge's, relative to the checkout.
    var files: [String]
}

/// What merging one project's share of a Crew job came to.
private enum CrewProjectMerge {
    case merged
    case alreadyOnMain
    case failed
}

extension AppModel {
    /// One project's share of a job: the paths this merge may take, the turns and hands
    /// they settle, the paths that lie under no checkout, the hands' files, and - of the
    /// paths the job's own records hold - the ones another chat's unfinished work claims
    /// instead, each under the name of the chat that claims it.
    typealias CrewWork = (paths: [String], turnIDs: [UUID], handIDs: [UUID], outside: [String], hands: [CrewMergeHand], held: [String: String])

    /// Whether any chat's team work is on its way to the remote right now.
    var isAnyCrewMergeRunning: Bool { liveRuntimes.contains { $0.isCrewMerging } }

    /// How long after its last message a lead is still one the auto-merge visits. It is
    /// the window `resumeCrewMerges` sweeps, and so also how long a hand's claim on its
    /// files lives once the lead has gone quiet: a lead outside it never merges again,
    /// and a claim of its hands could only hold those files from every later merge.
    private static var crewMergeWindow: TimeInterval { 3 * 24 * 3600 }

    /// Merges the last run left unfinished: a lead chat with the auto-merge on, no turn
    /// running, no hand still out, and work of its own that never went out - a finished
    /// turn's files, or a hand whose landing no merge has taken, which stands even when
    /// every turn reads as spent. The merge itself finds the files and lets a job that
    /// changed nothing be.
    func resumeCrewMerges() async {
        guard settings.crewAutoMerge else { return }
        let cutoff = Date.now.addingTimeInterval(-Self.crewMergeWindow)
        let leads = threads.filter { !$0.isArchived && crewIsOn($0) && $0.updatedAt > cutoff }
        for lead in leads {
            let runtime = self.runtime(for: lead.id)
            await runtime.ensureLoaded()
            guard runtime.phase == .idle, !runtime.isCrewMerging, runningCrewHands(of: lead.id) == 0 else { continue }
            // The job is finished, whatever its turns read: a lead whose report-review turn
            // was booked spent while its hand's work never went out has no unmerged turn
            // left, and requiring one here stranded exactly that hand. A hand whose landing
            // no merge has taken yet is work of its own, and the test below counts it.
            guard runtime.turns.last?.status == .completed else { continue }
            let finished = runtime.crewUnmergedTurns.filter { $0.status == .completed }
            // A chat with nothing of its own to show never merges: Crew merely being
            // switched on is not work, and the checkout sweep would otherwise take whatever
            // else in that checkout had changed, another chat's files or the user's own.
            let hasOwnWork = finished.contains { !($0.touchedPaths ?? []).isEmpty }
                || crewTeam(of: lead.id).contains { $0.crew.map { $0.mergedAt == nil && ($0.landing?.landed ?? false) } ?? false }
            guard hasOwnWork else { continue }
            await autoMergeCrewWork(of: lead.id)
        }
    }

    /// Lands a lead's finished work, when the setting says so. Runs once per finished job;
    /// what happened lands in the lead's timeline as a note from Crew.
    func autoMergeCrewWork(of leadID: UUID) async {
        guard settings.crewAutoMerge, let lead = thread(leadID), crewIsOn(lead), let project = project(lead.projectID),
              let runtime = existingRuntime(for: leadID), !runtime.isCrewMerging,
              runtime.phase == .idle, runtime.turns.last?.status == .completed,
              runningCrewHands(of: leadID) == 0 else { return }
        runtime.isCrewMerging = true
        runtime.crewMergeStartedAt = .now
        runtime.crewMergeNoteID = UUID().uuidString
        runtime.crewMergeStage = CrewMergeStage.gathering.words
        runtime.crewMergeStep = .gathering
        defer {
            runtime.isCrewMerging = false
            runtime.crewMergeStage = nil
            runtime.crewMergeStep = nil
            runtime.crewMergeStartedAt = nil
            runtime.crewMergeNoteID = nil
        }
        // A hand's work still going into the checkout finishes first: the snapshot the
        // commit is built from must hold the patch whole or not at all. Landings that have
        // not started yet wait for the merge instead (see `waitForSettledCheckout`).
        await runtime.crewLanding?.value
        guard runtime.phase == .idle, runtime.turns.last?.status == .completed,
              runningCrewHands(of: leadID) == 0 else { return }

        // What the team touched, and only that, so a sibling's uncommitted work in the
        // same checkout stays behind. The chat's own checkout goes first, then every
        // other sidebar project that holds touched files, one merge each; only paths
        // under no sidebar project at all stay out. What the records do not hold in one
        // of those other checkouts cannot be told apart from a sibling's work there and
        // stays behind without a word: none of it was this merge's to take.
        let ownCheckout = lead.worktreePath ?? project.path
        let own = await crewWork(of: leadID, runtime: runtime, checkout: ownCheckout, git: Git(ownCheckout), projectID: project.id, leadCheckout: ownCheckout, leadProjectID: project.id, sweepsCheckout: true)
        var groups: [(project: Project, checkout: String, work: CrewWork)] = [(project, ownCheckout, own)]
        // An outside path belongs to the sidebar project with the longest path that
        // holds it, so nested checkouts resolve to the inner one.
        var owners: [String: Project] = [:]
        for path in own.outside {
            if let holder = projects.filter({ $0.id != project.id && TouchedPaths.relative(path, root: $0.path) != nil }).max(by: { $0.path.count < $1.path.count }) {
                owners[path] = holder
            }
        }
        for other in projects where other.id != project.id && owners.values.contains(where: { $0.id == other.id }) {
            let work = await crewWork(of: leadID, runtime: runtime, checkout: other.path, git: Git(other.path), projectID: other.id, leadCheckout: ownCheckout, leadProjectID: project.id, sweepsCheckout: false)
            groups.append((other, other.path, work))
        }
        // A hand's scratch (a throwaway script in /tmp, a stray write into its own copy)
        // is nobody's project and not worth a word, and nor is the record a job keeps in a
        // project's archive (the coordination rows, the day's log, the bug pages and the
        // INDEX a lead closes out; see `CrewCoordination`): only a real folder outside the
        // sidebar is something the user can act on. The archive's other numbered folders
        // (the framework notes, the memory) are named apart: no merge takes them, and a hand
        // was told to keep out of some of them.
        let unowned = own.outside.filter { path in !Self.isScratchPath(path) && !projects.contains { TouchedPaths.relative(path, root: $0.path) != nil } && !CrewCoordination.isRecordPath(path) }
        let archiveNotes = unowned.filter(CrewCoordination.isArchiveNotePath)
        let stray = unowned.filter { !CrewCoordination.isArchiveNotePath($0) }
        func strayBody(_ paths: [String]) -> String {
            let shown = paths.prefix(6).map { "`\($0)`" }.joined(separator: ", ") + (paths.count > 6 ? " and \(paths.count - 6) more" : "")
            return "\(paths.count == 1 ? "1 file was" : "\(paths.count) files were") changed outside every project: \(shown). Add the folder that holds them to the sidebar and ask for the merge again."
        }
        // Said as the job settles, with the stray files, never on a retry that holds it:
        // the notes stay where they were written whatever the merge comes to.
        func noteArchiveNotes() {
            guard !archiveNotes.isEmpty else { return }
            let shown = archiveNotes.prefix(6).map { "`\($0)`" }.joined(separator: ", ") + (archiveNotes.count > 6 ? " and \(archiveNotes.count - 6) more" : "")
            let folders = CrewCoordination.handWritableFolders.joined(separator: ", ")
            note(leadID, "Crew left \(archiveNotes.count == 1 ? "a note" : "\(archiveNotes.count) notes") in a project archive alone.", "\(archiveNotes.count == 1 ? "1 file was" : "\(archiveNotes.count) files were") changed in a notes archive outside \(folders) and the INDEX: \(shown). Hands may write in those alone; check the change was meant (a lead updating the framework notes after a change to the stack is).")
        }
        // A hold in any project keeps the whole job pending; otherwise a shared turn
        // could be marked merged by its unblocked project's share.
        var held: [String: String] = [:]
        for group in groups {
            for (path, owner) in group.work.held {
                held[(group.checkout as NSString).appendingPathComponent(path)] = owner
            }
        }
        if !held.isEmpty {
            let detail = Self.heldDetail(held)
            let last = runtime.crewMerges.last
            if last?.outcome != .held || last?.detail != detail {
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .held, project: nil, label: nil, url: nil, files: held.count, detail: detail))
                note(leadID, "Crew is waiting for another chat's files.", detail + " The work stays in the checkout. Finish the unfinished work in that chat; Crew retries automatically while this job remains eligible.")
            }
            return
        }
        groups.removeAll { $0.work.paths.isEmpty }
        guard !groups.isEmpty else {
            guard !stray.isEmpty else {
                // Nothing to send and nothing outside a project: whatever the turns
                // recorded is already on the branch, ignored, build output, or gone. Mark
                // it spent, or the merge note counts those files forever.
                runtime.markCrewMerged(own.turnIDs)
                let spentAt = Date.now
                for handID in own.handIDs { updateCrewHand(handID) { $0.mergedAt = spentAt } }
                noteArchiveNotes()
                return
            }
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .stray, project: nil, label: nil, url: nil, files: stray.count, detail: strayBody(stray)))
            note(leadID, "Crew did not merge: the team's files are outside every project.", strayBody(stray))
            noteArchiveNotes()
            return
        }
        let several = groups.count > 1
        var outcomes: [CrewProjectMerge] = []
        for group in groups {
            outcomes.append(await mergeCrewProject(of: leadID, lead: lead, project: group.project, checkout: group.checkout, work: group.work, runtime: runtime, several: several))
        }
        guard !outcomes.contains(.failed) else { return }
        // The work is on the default branch now, so these turns are spent: the next
        // job merges what comes after them and never this again.
        runtime.markCrewMerged(groups.flatMap { $0.work.turnIDs })
        let mergedAt = Date.now
        for handID in groups.flatMap({ $0.work.handIDs }) { updateCrewHand(handID) { $0.mergedAt = mergedAt } }
        // The projects' shares went out; only what no project holds stayed behind.
        if !stray.isEmpty {
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .stray, project: nil, label: nil, url: nil, files: stray.count, detail: strayBody(stray)))
            note(leadID, "Crew left \(stray.count == 1 ? "a file" : "\(stray.count) files") outside every project.", strayBody(stray))
        }
        noteArchiveNotes()
    }

    /// A path no sidebar project could ever hold: a hand's copy of a checkout under the
    /// worktrees folder, the temporary folders a hand drops a throwaway script into,
    /// `~/Library`, and the dot-folders under home where agents keep their own state
    /// (`~/.claude`, `~/.codex`, `~/.hosted-code-dev`): a lead writing its memory files
    /// is not leaving work behind.
    private static func isScratchPath(_ path: String) -> Bool {
        let full = ((path as NSString).expandingTildeInPath as NSString).standardizingPath
        let home = (LoginEnvironment.homeDirectory as NSString).standardizingPath
        var roots = [Storage.worktreesDirectory.path, NSTemporaryDirectory(), "/tmp", "/private/tmp", "/var/folders", "/private/var/folders"]
        roots.append((home as NSString).appendingPathComponent("Library"))
        let underHome = home.hasSuffix("/") ? home : home + "/"
        if full.hasPrefix(underHome), full.dropFirst(underHome.count).first == "." { return true }
        // `standardizingPath` drops a leading `/private` only while the file still exists,
        // so a hand's temp file deleted since it was written keeps its `/private/tmp`
        // form while the roots lose theirs: both spellings are tried.
        let spellings = full.hasPrefix("/private/") ? [full, String(full.dropFirst("/private".count))] : [full]
        return roots.contains { root in
            let root = (root as NSString).standardizingPath
            let prefix = root.hasSuffix("/") ? root : root + "/"
            return spellings.contains { $0.hasPrefix(prefix) }
        }
    }

    /// Merges one project's share of a lead's finished work from its checkout: the guards
    /// for repository, remote and operation in progress; the conflict-marker check; the
    /// commit, then as far as `crewMergeMode` goes (the checkout's own branch for a local
    /// commit; a pushed branch; or the branch, its merge request and the merge); and the
    /// notes. The caller groups the projects and marks the turns and hands merged once
    /// every project went through.
    private func mergeCrewProject(of leadID: UUID, lead: ChatThread, project: Project, checkout: String, work: CrewWork, runtime: ThreadRuntime, several: Bool) async -> CrewProjectMerge {
        func stage(_ name: String) -> String { several ? "\(project.name): \(name)" : name }
        func title(_ name: String) -> String { several ? "\(project.name): \(name)" : name }
        // The words carry the project's name with several projects, the step is the dot on the pill's track.
        func advance(_ step: CrewMergeStage) { runtime.crewMergeStage = stage(step.words); runtime.crewMergeStep = step }

        let mode = settings.crewMergeMode
        let git = Git(checkout)
        guard await git.isRepository(), await git.hasCommits() else { return .failed }
        // Only a merge request or a pushed branch needs somewhere to push to; a local commit
        // is the way for a repository with none.
        if mode.pushes, await git.remoteURL() == nil {
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew did not merge: no remote.") + ": The team's work is in the checkout; there is no origin to push it to. Merge as Local commit (Settings › Crew) commits it in the checkout instead."))
            note(leadID, title("Crew did not merge: no remote."), "The team's work is in the checkout; there is no origin to push it to. To commit it in the checkout instead, set Merge as to Local commit in Settings › Crew.")
            return .failed
        }
        guard !(await git.hasOperationInProgress()) else {
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew did not merge: a rebase or merge is underway.") + ": The team's work is in the checkout; finish that first and merge by hand."))
            note(leadID, title("Crew did not merge: a rebase or merge is underway."), "The team's work is in the checkout; finish that first and merge by hand.")
            return .failed
        }

        let sorted = work.paths
        // A hand's landing can leave conflict markers behind (see `Git.apply`), and a lead
        // told not to look at git status may answer without settling them. Those markers
        // must never reach the remote: the job stays in the checkout until they are gone.
        let conflicted = await Self.pathsWithConflictMarkers(sorted, in: checkout)
        guard conflicted.isEmpty else {
            let files = conflicted.map { "`\($0)`" }.joined(separator: ", ")
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew did not merge: conflict markers in \(conflicted.count == 1 ? "a file" : "\(conflicted.count) files").") + ": \(files) still \(conflicted.count == 1 ? "holds" : "hold") conflict markers from a hand's landing."))
            note(leadID, title("Crew did not merge: conflict markers in \(conflicted.count == 1 ? "a file" : "\(conflicted.count) files")."), "\(files) still \(conflicted.count == 1 ? "holds" : "hold") conflict markers from a hand's landing. Resolve them, then ask for the merge again.")
            return .failed
        }

        do {
            let head = try await git.commitHash()
            advance(.committing)
            let tree = try await git.captureTree(paths: sorted)
            // Everything the team did is committed already: taken along by another chat's
            // merge from the same checkout, committed by hand, or committed hand by hand as
            // it landed (`crewCommitsLandings`). The job is spent either way, and there is
            // nothing to tell: the work is where it was meant to go.
            guard try await tree != git.treeHash(of: "HEAD") else { return .alreadyOnMain }

            let patch = (try? await git.diff(from: head, to: tree)) ?? ""
            // A whole job's patch is big and parsing it counts every line: off the main
            // actor, so the chat stays live while the work goes out.
            let files = await Task.detached(priority: .utility) { DiffParser.parse(patch) }.value
            // A fixed template, not a model call: the message is there once the files are known.
            let message = crewMergeCommitMessage(for: lead, hands: work.hands, files: files)
            let subject = TextCleanup.singleLine(message, limit: 72)

            let status = await git.status()
            let current = status?.branch
            // A local commit on no branch would be left behind by the next checkout of one,
            // with the job booked as merged (see `commitCrewLanding`, which holds back the
            // same way): it waits for the checkout to be on a branch.
            guard mode.pushes || current != nil else {
                let detail = "The checkout is on a detached HEAD, where a commit would be lost at the next checkout of a branch."
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew did not merge: the checkout is on no branch.") + ": " + detail))
                note(leadID, title("Crew did not merge: the checkout is on no branch."), detail + " The work is still in the checkout; switch to a branch, then ask for the merge again.")
                return .failed
            }
            // Only work bound for the remote goes by the remote's default branch.
            let target: String
            if mode.pushes {
                target = await git.defaultBranch()
            } else {
                target = current ?? "HEAD"
            }
            // On the default branch the commit goes to a branch of its own, made and pushed
            // without ever being checked out; on any other branch it is that branch's. A
            // local commit always goes on the checkout's own branch: nothing leaves the Mac.
            let ownBranch = mode.pushes && (current == nil || current == target)
            let branch = ownBranch ? Self.crewBranchName(for: lead) : (current ?? "HEAD")
            let commit = try await git.commitWork(tree, parent: head, message: message)
            // Writing the branch to a commit built on `head` drops anything committed
            // since: a minute passes here, and the user may have committed in a terminal.
            // HEAD is read again so the user gets told in plain words when that happened,
            // and the move itself names the value it expects, so git refuses the write
            // outright if the branch shifts in the moment between the two.
            guard try await git.commitHash() == head else {
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew did not merge: the checkout moved while it worked.") + ": A commit landed on `\(current ?? target)` while the team's work was being prepared."))
                note(leadID, title("Crew did not merge: the checkout moved while it worked."), "A commit landed on `\(current ?? target)` while the team's work was being prepared, and moving the branch now would orphan it. The work is still in the checkout; ask for the merge again.")
                return .failed
            }
            // A branch of its own must not exist yet; the checkout's own branch must still
            // be where it was read. A local commit moves HEAD itself, which carries the
            // branch it is on along, on the same condition.
            try await git.updateRef(mode.pushes ? "refs/heads/\(branch)" : "HEAD", to: commit, expecting: ownBranch ? nil : head)
            if !ownBranch {
                // The branch moved under the checkout: the index catches up, the working
                // tree already has the content.
                try await git.resetIndex(paths: sorted)
            }

            // A local commit is the whole of it.
            guard mode.pushes else {
                let short = String(commit.prefix(7))
                let place = current ?? target
                var lines = [Self.filesLine(files) + " committed on `\(place)` as `\(short)`."]
                lines += Self.mergeHandsLines(work.hands)
                lines.append("")
                lines.append("Nothing was pushed: Merge as is set to Local commit in Settings › Crew.")
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .merged, project: project.name, label: "\(short) on \(place)", url: nil, files: files.count, detail: nil, mode: .localCommit))
                note(leadID, title("Crew merged \(short) into \(place)"), lines.joined(separator: "\n"))
                existingRuntime(for: leadID)?.noteDiffChanged()
                return .merged
            }

            advance(.pushing)
            try await git.pushBranch(branch)

            // A pushed branch stops here: the merge request is the user's to open, from the
            // link the forge offers for it when there is one.
            guard mode == .mergeRequest else {
                let open = await crewNewRequestURL(git: git, source: branch, target: target)
                var lines: [String] = open.map { [$0.absoluteString, ""] } ?? []
                lines.append(Self.filesLine(files) + " pushed on `\(branch)`, for `\(target)`.")
                lines += Self.mergeHandsLines(work.hands)
                lines.append("")
                lines.append("No merge request was opened (Merge as is set to Push branch in Settings › Crew): open one for `\(branch)` into `\(target)` when the work is ready.")
                if ownBranch {
                    lines.append(await standOnPushedBranch(git, branch: branch, from: head, target: current, paths: sorted)
                        ? "The checkout is on `\(branch)` now, where the team's files are committed. Once it is merged, `git switch \(target)` and `git pull`."
                        : "The team's files stay uncommitted on `\(target)` in the checkout: it moved while the branch went out. Once `\(branch)` is merged, `git stash push` those files, `git pull`, then `git stash pop`.")
                }
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .merged, project: project.name, label: branch, url: open, files: files.count, detail: nil, mode: .pushBranch))
                note(leadID, title("Crew pushed \(branch)"), lines.joined(separator: "\n"))
                existingRuntime(for: leadID)?.noteDiffChanged()
                return .merged
            }

            let body = mergeRequestBody(for: lead, hands: work.hands, files: files, through: Set(work.turnIDs), runtime: runtime)
            advance(.opening)
            let requestURL: URL?
            do {
                requestURL = try await git.createPullRequest(title: subject, body: body, source: branch, target: target)
            } catch {
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew pushed \(branch) but could not open a merge request.") + ": \(error.localizedDescription)"))
                note(leadID, title("Crew pushed \(branch) but could not open a merge request."), "\(error.localizedDescription)\n\nOpen one for `\(branch)` into `\(target)` and merge it from there. The work is still in the checkout.")
                return .failed
            }
            guard let url = requestURL,
                  let link = MergeRequestLink(url: url) else {
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: title("Crew pushed \(branch) but could not open a merge request.")))
                note(leadID, title("Crew pushed \(branch) but could not open a merge request."), "Open one for `\(branch)` into `\(target)` and merge it from there.")
                return .failed
            }
            advance(.merging)
            // Squashed into one commit, unless the default branch holds commits the remote
            // does not have yet: a branch made from it carries them, a squash would copy them
            // into its one commit, and the next pull would bring a second copy in beside the
            // originals (the checkout could not follow either, see `syncDefaultBranch`). A
            // merge commit keeps them as they are.
            let squashes = ownBranch ? await git.isAncestor(head, of: "origin/\(target)") : true
            do {
                // The branch goes with the merge only when it was made for the request. The
                // checkout's own branch is someone's to keep, and deleting it would move the
                // checkout off it.
                try await git.mergePullRequest(link, deleteSourceBranch: ownBranch, squash: squashes)
            } catch {
                runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: link.label, url: url, files: 0, detail: title("Crew opened \(link.label) but could not merge it.") + ": \(error.localizedDescription)"))
                note(leadID, title("Crew opened \(link.label) but could not merge it."), "\(url.absoluteString)\n\n\(error.localizedDescription)\n\nMerge it from the link once it is ready; the branch `\(branch)` has the team's work.")
                return .failed
            }

            var lines = ["\(url.absoluteString)", "", Self.filesLine(files) + " landed on `\(target)` from `\(branch)`."]
            if !squashes {
                lines.append("Merged with a merge commit rather than squashed: `\(target)` had commits not yet on the remote, which a squash would have copied.")
            }
            lines += Self.mergeHandsLines(work.hands)
            // What the merge left out (files written outside the checkout, build output,
            // ignored or missing paths, a sibling chat's files) is not said: none of it
            // was this merge's to take, and every line about it read as if something
            // had gone wrong.
            if ownBranch {
                advance(.syncing)
                let synced = await syncDefaultBranch(git, from: head, target: target, ownPaths: sorted)
                lines.append(synced ? "The checkout is up to date." : "The checkout was left as it was: `\(target)` moved on in other ways meanwhile, or the team's files changed again; `git pull` when it suits you.")
            } else if lead.worktreePath != nil {
                // The lead worked in a worktree: the project's own checkout follows when it can.
                advance(.syncing)
                let main = Git(project.path)
                if await main.status()?.branch == target, await main.dirtyPaths().isEmpty, !(await main.hasOperationInProgress()) {
                    if (try? await main.pullFastForward()) != nil { lines.append("The project checkout is up to date.") }
                }
            }
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .merged, project: project.name, label: link.label, url: url, files: files.count, detail: nil, mode: .mergeRequest))
            note(leadID, title("Crew merged \(link.label)"), lines.joined(separator: "\n"))
            existingRuntime(for: leadID)?.noteDiffChanged()
            return .merged
        } catch {
            runtime.recordCrewMerge(CrewMergeRecord(at: .now, outcome: .failed, project: project.name, label: nil, url: nil, files: 0, detail: "Crew could not merge the team's work: \(error.localizedDescription)"))
            note(leadID, title("Crew could not merge the team's work."), "\(error.localizedDescription)\n\nThe work is still in the checkout.")
            return .failed
        }
    }

    /// The files a lead's job changed, gathered three ways because no one way sees them
    /// all:
    ///
    /// - the turns of this job, meaning the ones no merge has taken yet, and the paths
    ///   they reported;
    /// - the hands that worked alongside those turns: what a hosted hand landed, and,
    ///   for a hand the provider runs inside the lead's own session, the edits its own
    ///   timeline landed. A native hand works in this very checkout and lands nothing,
    ///   so without its timeline its files are simply missing and the branch goes out
    ///   half-written;
    /// - and, where the first of those turns left a checkpoint, whatever else the checkout
    ///   holds that it did not then, less whatever was already uncommitted at the time.
    ///   That catches the edits no tool reported, a hand writing through a shell say, and
    ///   costs nothing for a file whose content matches HEAD anyway.
    ///
    /// Paths outside the checkout are dropped rather than passed on: a lead writes to its
    /// own memory files, and `git add -A -- <path>` on one of those fails outright and
    /// takes the whole merge with it.
    private func crewWork(of leadID: UUID, runtime: ThreadRuntime, checkout: String, git: Git, projectID: UUID, leadCheckout: String, leadProjectID: UUID, sweepsCheckout: Bool) async -> CrewWork {
        let turns = runtime.crewUnmergedTurns
        var reported: [String] = turns.flatMap { $0.touchedPaths ?? [] }
        // A hand counts until a merge has taken its work (see `CrewHandInfo.mergedAt`),
        // and only that explicit mark proves a merge took it: a lead turn's `crewMerged`
        // flag is bookkeeping, and a turn falsely booked spent would let a timestamp
        // shortcut discard a hand whose files were still in the checkout. A hand from
        // before the mark existed that already went out settles below instead, where the
        // gathering finds its paths unchanged from HEAD.
        var handPaths: [(id: UUID, name: String, index: Int, task: String, paths: [String])] = []
        for hand in crewTeam(of: leadID) {
            guard let info = hand.crew, info.mergedAt == nil,
                  info.landing?.patchPath == nil, info.landing?.error == nil else { continue }
            // The checkout the hand's work landed in: the chat's own for a hand in its
            // project, that project's for a hand sent elsewhere. A path the hand's tools
            // reported lies in its own copy of that checkout and stands for the same path
            // there; a landing path is relative to it already. Anchored at that checkout,
            // a path resolves against this pass's checkout or counts as outside it, so a
            // hand's work in another project reaches that project's merge instead of
            // reading as this project's, and its copy never shows up as a stray folder.
            let handRoot = hand.projectID == leadProjectID ? leadCheckout : (project(hand.projectID)?.path ?? leadCheckout)
            func anchored(_ path: String) -> String {
                let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty else { return "" }
                if let copy = hand.worktreePath, let inCopy = TouchedPaths.relative(trimmed, root: copy) {
                    return (handRoot as NSString).appendingPathComponent(inCopy)
                }
                if trimmed.hasPrefix("/") || trimmed.hasPrefix("~") || trimmed.contains("://") { return trimmed }
                return (handRoot as NSString).appendingPathComponent(trimmed)
            }
            var own: [String] = []
            var seen = Set<String>()
            func collect(_ path: String) {
                guard !path.isEmpty, let relative = TouchedPaths.relative(anchored(path), root: checkout), seen.insert(relative).inserted else { return }
                own.append(relative)
            }
            for file in info.landing?.files ?? [] { collect(file.path) }
            reported += (info.landing?.files ?? []).map { anchored($0.path) }
            // A hand's timeline runs to megabytes. One still open is read as it is; one
            // that has gone cold is decoded off the main actor rather than brought back
            // as a runtime, which would parse the whole history on the main thread for
            // every hand of the team, one after another, while the chat sits frozen.
            let items: [TimelineItem]
            if let live = existingRuntime(for: hand.id) {
                items = live.entries.map(\.item)
            } else {
                let handID = hand.id
                items = await Task.detached(priority: .utility) { Storage.decodeDocument(handID)?.items ?? [] }.value
            }
            for item in items {
                guard case .tool(let call) = item.content else { continue }
                // Only what the tool landed: a row's edits are what it was asked to write,
                // and a write that was declined, failed or never approved is in no checkout.
                reported += call.landedEdits.map { anchored($0.path) }
                for edit in call.landedEdits { collect(edit.path) }
            }
            handPaths.append((hand.id, CrewRoster.persona(at: info.index).name, info.index, TextCleanup.singleLine(info.task, limit: 120), own))
        }

        // The lead's own tool edits, from its timeline: a lead works in any sidebar project
        // too, and a path it wrote outside its checkout is that project's share of the merge.
        // Its turns' `touchedPaths` hold only what lies inside the checkout.
        let unmerged = Set(turns.map(\.id))
        for entry in runtime.entries where entry.item.turnID.map(unmerged.contains) == true {
            guard case .tool(let call) = entry.item.content else { continue }
            for edit in call.landedEdits {
                let trimmed = edit.path.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty else { continue }
                if trimmed.hasPrefix("/") || trimmed.hasPrefix("~") || trimmed.contains("://") {
                    reported.append(trimmed)
                } else {
                    reported.append((leadCheckout as NSString).appendingPathComponent(trimmed))
                }
            }
        }

        var paths = Set<String>()
        var outside = Set<String>()
        for path in reported where !path.isEmpty && !TouchedPaths.isBuildOutput(path) {
            if let relative = TouchedPaths.relative(path, root: checkout) {
                paths.insert(relative)
            } else {
                outside.insert(path)
            }
        }

        // Files another chat's team put in this same checkout that no merge has taken
        // yet: that chat's merge carries them, never this one's. Without this, the
        // sweep below took a sibling's landed files along, and the sibling's own merge
        // then found nothing left and said nothing.
        let siblingPaths = siblingOwnedPaths(in: checkout, projectID: projectID, excluding: leadID)
        let siblingClaims = Set(siblingPaths.keys)
        // A start checkpoint that holds nothing while HEAD holds files knows no file the
        // checkout had (see `Git.isEmptyWhileHeadIsNot`): every untracked file, the user's
        // own included, would read as the turn's and go out with the merge. The records
        // alone speak for the turn then.
        if sweepsCheckout,
           let base = turns.first?.baseCheckpoint,
           (try? await git.isEmptyWhileHeadIsNot(base)) == false,
           let alreadyDirty = try? await git.changedPaths(from: "HEAD", to: base),
           let now = try? await git.captureTree(),
           let changedSince = try? await git.changedPaths(from: base, to: now) {
            let theirs = Set(alreadyDirty)
            let fresh = changedSince.filter { !theirs.contains($0) }
            paths.formUnion(fresh.filter { !siblingClaims.contains($0) })
        }

        // A file a live sibling hand or chat owns never goes out under this chat, even
        // when this chat's own records list it: a turn's touched paths are read off the
        // checkout's diff, and so can hold a file another chat made while the turn ran.
        // The turns and hands that put those files there are not booked as done here
        // either: a turn kept open keeps its files pending, so they go out with a later
        // merge that can take them, instead of reading as merged while lying uncommitted.
        let claimed = paths.intersection(siblingClaims)
        paths.subtract(siblingClaims)
        paths = paths.filter { !TouchedPaths.isBuildOutput($0) }
        paths.subtract(await git.ignoredPaths(among: paths.sorted()))
        let tracked = await git.trackedPaths(among: paths.sorted())
        let fileManager = FileManager.default
        let checkoutURL = URL(fileURLWithPath: checkout)
        let missing = Set(paths.filter { path in
            !fileManager.fileExists(atPath: checkoutURL.appendingPathComponent(path).path) && !tracked.contains(path)
        })
        paths.subtract(missing)
        let hands = handPaths.map { entry in
            CrewMergeHand(name: entry.name, index: entry.index, task: entry.task, files: entry.paths.filter { paths.contains($0) })
        }
        // What this merge took: every turn and hand whose files were all sendable here.
        // One that still has a file another chat holds stays open, so its files are not
        // booked as merged before a commit carries them.
        let turnIDs = turns.filter { turn in
            (turn.touchedPaths ?? []).allSatisfy { path in
                guard let relative = TouchedPaths.relative(path, root: checkout) else { return true }
                return !claimed.contains(relative)
            }
        }.map(\.id)
        let handIDs = handPaths.filter { Set($0.paths).isDisjoint(with: claimed) }.map(\.id)
        // The held files with the chat holding each, so a job that can send nothing at
        // all can say who has it rather than being written off in silence.
        let held = claimed.reduce(into: [String: String]()) { result, path in result[path] = siblingPaths[path] }
        return (paths.sorted(), turnIDs, handIDs, outside.sorted(), hands, held)
    }

    /// Files another chat's team put or is still putting in this same checkout that no
    /// merge has taken yet, each mapped to the chat that holds it. That chat's own merge
    /// carries them, never this one's, so the checkout sweep in `crewWork` keeps them out
    /// of a merge the same way.
    ///
    /// A claim stands only while the chat that made it can still land its work
    /// (`canStillMerge`). A hand left unmarked by an older merge - its lead merged again
    /// since, or the job is long over - used to go on claiming its files forever, and
    /// every chat that later touched one of them went out empty, without a word.
    private func siblingOwnedPaths(in checkout: String, projectID: UUID, excluding leadID: UUID) -> [String: String] {
        let team = Set(crewTeam(of: leadID).map(\.id))
        var paths: [String: String] = [:]
        for other in threads where other.id != leadID && other.projectID == projectID && !team.contains(other.id) {
            if other.isCrewHand {
                guard let info = other.crew, info.mergedAt == nil else { continue }
                // A hand still out has files coming whatever its lead is doing; a hand
                // that has finished holds them only for a lead that can still land them.
                let parent = other.parentThreadID.flatMap { thread($0) }
                guard info.status == .running || parent.map(canStillMerge) == true else { continue }
                for file in info.landing?.files ?? [] {
                    if let relative = TouchedPaths.relative(file.path, root: checkout) {
                        paths[relative] = parent?.title ?? other.title
                    }
                }
            } else if let live = existingRuntime(for: other.id) {
                // A stopped turn may contain partial edits. Keep those protected even
                // when completed hand landings no longer reserve the same files.
                for turn in live.crewUnmergedTurns where canStillMerge(other) || turn.status != .completed {
                    for path in turn.touchedPaths ?? [] {
                        if let relative = TouchedPaths.relative(path, root: checkout) { paths[relative] = other.title }
                    }
                }
            }
        }
        return paths
    }

    /// Active work keeps its claims regardless of age. Completed hand landings stop
    /// reserving files when their idle lead is no longer eligible for recovery.
    private func canStillMerge(_ lead: ChatThread) -> Bool {
        let live = existingRuntime(for: lead.id)
        if let live, live.phase != .idle || live.isCrewMerging { return true }
        guard !lead.isArchived, crewIsOn(lead),
              lead.updatedAt > Date.now.addingTimeInterval(-Self.crewMergeWindow) else { return false }
        // Landed hand work no merge has taken keeps the claim alive the same way an
        // unmerged turn does: this is the eligibility `resumeCrewMerges` uses.
        let landedHandWork = crewTeam(of: lead.id).contains { $0.crew.map { $0.mergedAt == nil && ($0.landing?.landed ?? false) } ?? false }
        guard let live else { return lead.lastStatus == .completed }
        return live.turns.last?.status == .completed && (!live.crewUnmergedTurns.isEmpty || landedHandWork)
    }

    /// Brings the checkout's default branch up to the merge without touching the working
    /// tree: the branch moves to what origin has, the index takes the merged version of the
    /// team's files (the working tree already holds it), and any other file the remote
    /// changed meanwhile is checked out only where it is clean locally. Returns false, and
    /// changes nothing, where that cannot be done safely.
    /// Push branch from the default branch: the checkout moves onto the branch it pushed,
    /// which is HEAD plus the team's files as the working tree has them, so only HEAD and
    /// the index of those files change. Left on the default branch, the files would stay
    /// uncommitted there: the pull after the branch is merged refuses over them, and the
    /// next job pushes them again. False, with the checkout as it was, when HEAD moved
    /// since the commit was made or git would not do it; `target` is the branch to go
    /// back to should the index not follow (nil for a detached HEAD).
    private func standOnPushedBranch(_ git: Git, branch: String, from head: String, target: String?, paths: [String]) async -> Bool {
        guard (try? await git.commitHash()) == head, (try? await git.pointHead(at: branch)) != nil else { return false }
        do {
            try await git.resetIndex(paths: paths)
            return true
        } catch {
            // Back where it stood: on its branch, or detached at the commit it was on. HEAD
            // is a symbolic ref to the new branch by now, so a plain write to HEAD would move
            // that branch instead.
            if let target {
                try? await git.pointHead(at: target)
            } else {
                try? await git.detachHead(at: head)
            }
            return false
        }
    }

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

    /// The request's description, in Chinese like its commit: what the team did, with the
    /// hands whose files it carries, and how the lead summed it up.
    private func mergeRequestBody(for lead: ChatThread, hands: [CrewMergeHand], files: [DiffFile], through: Set<UUID>, runtime: ThreadRuntime) -> String {
        var lines = ["由 Bottega 在 Crew 团队完成后开启。", ""]
        // The hands of this job with files in it, as the commit names them: a hand whose
        // work went out with an earlier merge is no part of this request.
        let contributors = hands.filter { !$0.files.isEmpty }
        if !contributors.isEmpty {
            lines.append("## 参与的 hand")
            for hand in contributors {
                lines.append("- **\(hand.name)**：\(TextCleanup.singleLine(hand.task, limit: 120))（\(hand.files.count) 个文件）")
            }
            lines.append("")
        }
        if let reply = runtime.entries.last(where: { $0.kind == .assistant && $0.item.turnID.map(through.contains) == true }).flatMap({ entry -> String? in
            guard case .assistant(let message) = entry.item.content else { return nil }
            return message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        }), !reply.isEmpty {
            lines.append("## Lead 的总结")
            lines.append(reply.count > 2_000 ? String(reply.prefix(2_000)) + "…" : reply)
            lines.append("")
        }
        lines.append("## 文件")
        lines += files.map { "- `\($0.path)` (+\($0.additions) −\($0.deletions))" }
        return lines.joined(separator: "\n")
    }

    private static func crewBranchName(for lead: ChatThread) -> String {
        let title = lead.title == ChatThread.untitled ? "team" : lead.title
        var slug = title.lowercased().map { $0.isLetter || $0.isNumber ? String($0) : "-" }.joined()
        while slug.contains("--") { slug = slug.replacingOccurrences(of: "--", with: "-") }
        slug = slug.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        if slug.count > 40 { slug = String(slug.prefix(40)).trimmingCharacters(in: CharacterSet(charactersIn: "-")) }
        let suffix = String(UUID().uuidString.lowercased().prefix(6))
        return "crew/\(slug.isEmpty ? "team" : slug)-\(suffix)"
    }

    /// The hands whose work a merge took, a line each under a "Hands" line after a blank
    /// one: the shape `CrewMergeNote` reads back for the merged card. Nothing without hands.
    private static func mergeHandsLines(_ hands: [CrewMergeHand]) -> [String] {
        guard !hands.isEmpty else { return [] }
        var lines = ["", "Hands"]
        for hand in hands {
            if hand.files.isEmpty {
                lines.append("- \(hand.name) (\(hand.index)): \(hand.task) — no files")
            } else {
                lines.append("- \(hand.name) (\(hand.index)): \(hand.task) — \(hand.files.count == 1 ? "1 file" : "\(hand.files.count) files"): \(hand.files.joined(separator: ", "))")
            }
        }
        return lines
    }

    /// Where the forge offers to open a merge request for a branch just pushed, for the
    /// pushed-branch note's link: GitHub's and Gitea's compare pages, GitLab's new-request
    /// form. Nil where the remote has no web address. Credentials the remote's address
    /// carries stay out of the link.
    private func crewNewRequestURL(git: Git, source: String, target: String) async -> URL? {
        guard var web = await git.remoteWebURL(), web.host != nil else { return nil }
        if web.user != nil, var components = URLComponents(url: web, resolvingAgainstBaseURL: false) {
            components.user = nil
            components.password = nil
            if let stripped = components.url { web = stripped }
        }
        var base = web.absoluteString
        if base.hasSuffix("/") { base.removeLast() }
        var queryValue = CharacterSet.urlQueryAllowed
        queryValue.remove(charactersIn: "&=+#?")
        guard let sourcePath = source.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed),
              let targetPath = target.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed),
              let sourceQuery = source.addingPercentEncoding(withAllowedCharacters: queryValue),
              let targetQuery = target.addingPercentEncoding(withAllowedCharacters: queryValue) else { return nil }
        switch await git.forge() {
        case .github:
            return URL(string: "\(base)/compare/\(targetPath)...\(sourcePath)?expand=1")
        case .gitlab:
            return URL(string: "\(base)/-/merge_requests/new?merge_request%5Bsource_branch%5D=\(sourceQuery)&merge_request%5Btarget_branch%5D=\(targetQuery)")
        case .gitea:
            return URL(string: "\(base)/compare/\(targetPath)...\(sourcePath)")
        }
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

    /// What to say about files a merge had to leave with another chat's unfinished work:
    /// the files, and the chats holding them. Used for both the note the user reads and
    /// the record the lead's next message is briefed from, so the two never disagree.
    private static func heldDetail(_ held: [String: String]) -> String {
        let files = held.keys.sorted()
        let shown = files.prefix(6).map { "`\($0)`" }.joined(separator: ", ") + (files.count > 6 ? " and \(files.count - 6) more" : "")
        let names = Set(held.values).sorted().joined(separator: ", ")
        return "\(files.count == 1 ? "1 file the team changed is" : "\(files.count) files the team changed are") also part of unfinished work in \(names): \(shown)."
    }

    private static func filesLine(_ files: [DiffFile]) -> String {
        let additions = files.reduce(0) { $0 + $1.additions }
        let deletions = files.reduce(0) { $0 + $1.deletions }
        return "\(files.count == 1 ? "1 file" : "\(files.count) files") (+\(additions) −\(deletions))"
    }

    /// A note from Crew in the lead's timeline, and word of it when the chat is out of view.
    private func note(_ leadID: UUID, _ title: String, _ body: String) {
        // The note lands under the id the merging pill has been drawn with, so the timeline
        // keeps one row from the first stage to the outcome; the id is spent here, so a
        // second note in the same run would get one of its own.
        let runtime = existingRuntime(for: leadID)
        let id = runtime?.crewMergeNoteID
        runtime?.crewMergeNoteID = nil
        runtime?.appendCrewNote(title + "\n" + body, id: id)
        let onScreen = selectedThreadID == leadID
        guard !(NSApp.isActive && onScreen) else { return }
        updateThread(leadID) { $0.hasUnread = true }
        updateDockBadge()
        if settings.notifyWhenFinished, let lead = thread(leadID) {
            notify(threadID: leadID, title: lead.title, body: title)
        }
    }
}
