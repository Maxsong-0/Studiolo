// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

// The commits Crew makes itself: the one a team's work goes out as when the auto-merge
// takes it, and, with "Commit each hand's work as it lands" on, one per hand as its work
// lands. Both are written from a fixed template in Chinese, the language the user's
// commits are written in, rather than by a model call, and both end with Co-Authored-By
// trailers naming the models that did the work.

extension AppModel {
    /// The message a team's work is committed with: the chat's title, what the commit
    /// carries, the hands whose files are in it with their tasks, and the trailers of the
    /// lead's model and those hands' models.
    func crewMergeCommitMessage(for lead: ChatThread, hands: [CrewMergeHand], files: [DiffFile]) -> String {
        let title = lead.title == ChatThread.untitled ? "团队的改动" : TextCleanup.singleLine(lead.title, limit: 60)
        let additions = files.reduce(0) { $0 + $1.additions }
        let deletions = files.reduce(0) { $0 + $1.deletions }
        var lines = [
            "Crew：\(title)",
            "",
            "Bottega Crew 团队完成后自动提交：\(files.count) 个文件（+\(additions) −\(deletions)）。",
        ]
        // Only the hands this commit carries files of: a hand of the team that changed
        // nothing here, or whose work went out with an earlier merge, is not named.
        let contributors = hands.filter { !$0.files.isEmpty }
        if !contributors.isEmpty {
            lines.append("")
            for hand in contributors {
                lines.append("- \(hand.name)：\(TextCleanup.singleLine(hand.task, limit: 100))（\(hand.files.count) 个文件）")
            }
        }
        let indexes = Set(contributors.map(\.index))
        let handIDs = crewTeam(of: lead.id).filter { indexes.contains($0.crew?.index ?? -1) }.map(\.id)
        return Self.withCoAuthors(lines, crewCoAuthors(of: [lead.id] + handIDs))
    }

    /// The message one hand's landed work is committed with, under its name and task.
    func crewLandingCommitMessage(_ info: CrewHandInfo, handID: UUID, files: [CrewLanding.File]) -> String {
        let name = info.persona.name
        let task = TextCleanup.singleLine(info.task, limit: 60)
        let additions = files.reduce(0) { $0 + $1.additions }
        let deletions = files.reduce(0) { $0 + $1.deletions }
        let lines = [
            task.isEmpty ? "WIP(\(name))" : "WIP(\(name))：\(task)",
            "",
            "Crew hand \(name) 的改动干净落地后即时提交：\(files.count) 个文件（+\(additions) −\(deletions)），任务中途停下也不会丢。",
        ]
        return Self.withCoAuthors(lines, crewCoAuthors(of: [handID]))
    }

    /// Commits a hand's cleanly landed work on the checkout's branch at once (the
    /// `crewCommitsLandings` setting), its files only: whatever else the checkout holds,
    /// the lead's edits, another hand's, the user's, stays as it was. A checkout on a
    /// detached HEAD, or with a rebase or merge half done, is left alone, and so is one
    /// whose HEAD moved while the commit was made: the work stays in the checkout,
    /// uncommitted, as it would with the setting off, and the lead's timeline says why.
    func commitCrewLanding(_ landing: CrewLanding, of info: CrewHandInfo, handID: UUID, leadID: UUID, checkout: String) async {
        let git = Git(checkout)
        // On no branch, the commit would be left behind by the next checkout of one.
        guard let status = await git.status(), status.branch != nil else { return }
        guard !(await git.hasOperationInProgress()) else { return }
        do {
            _ = try await git.commitPaths(landing.files.map(\.path), message: crewLandingCommitMessage(info, handID: handID, files: landing.files))
        } catch {
            existingRuntime(for: leadID)?.appendCrewNote("""
            \(info.persona.name)'s work landed but was not committed.
            \(error.localizedDescription)

            The work is in the checkout, uncommitted, as it would be with "Commit each hand's work as it lands" off.
            """)
        }
    }

    /// A Co-Authored-By trailer per model among the threads, in their order and each once:
    /// the models that did the work, the way the user's own commits credit theirs.
    func crewCoAuthors(of threadIDs: [UUID]) -> [String] {
        var trailers: [String] = []
        for id in threadIDs {
            guard let thread = thread(id) else { continue }
            let trailer = "Co-Authored-By: " + crewCoAuthor(provider: thread.provider, model: thread.model)
            if !trailers.contains(trailer) { trailers.append(trailer) }
        }
        return trailers
    }

    /// A model as a co-author: Claude's models under Anthropic's no-reply address, the way
    /// Claude Code signs its own ("Claude Fable 5.1"); every other provider's under the
    /// app's own local identity, since no address of theirs is known to stand for a model.
    private func crewCoAuthor(provider: ProviderKind, model: String?) -> String {
        let label = Self.crewModelLabel(providers.model(model, for: provider), id: model)
        switch provider {
        case .claude:
            let name = label.map { $0.lowercased().hasPrefix("claude") ? $0 : "Claude \($0)" } ?? "Claude"
            return "\(name) <noreply@anthropic.com>"
        default:
            let name = label.map { "\(provider.displayName) (\($0))" } ?? provider.displayName
            return "\(name) <bottega@localhost>"
        }
    }

    /// A model's name for a trailer: the catalogue's, or the version its description opens
    /// with ("Fable 5.1 · Most capable…" gives "Fable 5.1"); the id when the catalogue does
    /// not list the model; nil for the provider's default, whose model is not known here.
    private static func crewModelLabel(_ option: ModelOption?, id: String?) -> String? {
        guard let option else {
            guard let id, !id.isEmpty, id != "default", id != "auto" else { return nil }
            return id
        }
        guard option.id != "default", option.id != "auto" else { return nil }
        if let detail = option.detail,
           let opening = detail.components(separatedBy: " · ").first?.trimmingCharacters(in: .whitespaces),
           opening.hasPrefix(option.name + " "),
           opening.dropFirst(option.name.count + 1).allSatisfy({ $0.isNumber || $0 == "." }) {
            return opening
        }
        return option.name
    }

    /// The lines, then a blank line and the trailers when there are any.
    private static func withCoAuthors(_ lines: [String], _ trailers: [String]) -> String {
        (trailers.isEmpty ? lines : lines + [""] + trailers).joined(separator: "\n")
    }
}
