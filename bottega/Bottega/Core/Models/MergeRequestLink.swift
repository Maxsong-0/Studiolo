// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A merge request (GitLab), pull request (GitHub, Gitea, Forgejo, Codeberg) that a link in
/// the chat points at, for the "Merge" row on the link's popover and the helper it spawns.
struct MergeRequestLink: Equatable, Sendable {
    enum Forge: Sendable {
        case gitlab
        case github
        /// Gitea and its forks, Forgejo and Codeberg among them: one CLI, `tea`, serves them all.
        case gitea
    }

    var url: URL
    var forge: Forge
    var number: Int

    /// How each forge writes its own: "!95" on GitLab, "#95" everywhere else.
    var label: String { forge == .gitlab ? "!\(number)" : "#\(number)" }

    /// `…/group/project/-/merge_requests/95` (any GitLab host, `diffs` or another page after
    /// the number included), `owner/repo/pull/95` (github.com or a GitHub Enterprise host),
    /// or `owner/repo/pulls/95` (Gitea, Forgejo, Codeberg).
    init?(url: URL) {
        let parts = url.pathComponents.filter { $0 != "/" }
        if let index = parts.firstIndex(of: "merge_requests"), index >= 1, index + 1 < parts.count,
           let number = Int(parts[index + 1]), number > 0 {
            forge = .gitlab
            self.number = number
        } else if parts.count >= 4, parts[2] == "pull", let number = Int(parts[3]), number > 0 {
            forge = .github
            self.number = number
        } else if parts.count >= 4, parts[2] == "pulls", let number = Int(parts[3]), number > 0 {
            forge = .gitea
            self.number = number
        } else {
            return nil
        }
        self.url = url
    }

    /// The helper's title: the same words as the popover's row.
    var title: String { "Merge \(label)" }

    /// What the helper is asked to do: land the request with the forge's own CLI, then leave
    /// the checkout on the merged main. Complete on its own, since a native provider reads
    /// no AGENTS.md before it starts.
    ///
    /// It merges the way the app's own merges do (`AppModel.mergeCrewProject` through
    /// `Git.mergePullRequest`): squashed on GitLab and GitHub, with a merge commit instead
    /// when the request carries commits that the checkout's copy of the target branch has
    /// and the remote's does not yet, since a squash would bring them back as a second copy
    /// and that branch could no longer fast-forward; tea merges as it does there. GitHub's
    /// merge used to be a merge commit every time, and GitLab's whatever the request itself
    /// was set to. The request's branch is left alone: the link can be anyone's request,
    /// and deleting its branch (GitHub's `--delete-branch` takes the local one too, and
    /// moves the checkout off it) is what the app's merge does only for a branch it made
    /// for the request itself.
    var prompt: String {
        let land = switch forge {
        case .gitlab:
            "Use glab from this directory: `glab mr merge \(number) --yes --squash`, leaving out `--squash` when the request carries commits that this checkout's copy of its target branch has and the remote's does not yet (a squash would copy them, and that branch here could no longer fast-forward). If GitLab answers that the request is still being checked (a 405), wait a few seconds and try again; if it reports conflicts, stop and say so instead of resolving them."
        case .github:
            "Use gh from this directory: `gh pr merge \(number) --squash`, or `--merge` instead when the pull request carries commits that this checkout's copy of its base branch has and the remote's does not yet (a squash would copy them, and that branch here could no longer fast-forward). If GitHub answers that the method is not allowed on this repository, use one it allows. If GitHub reports the pull request is not mergeable, stop and say so instead of resolving it."
        case .gitea:
            "Use tea from this directory: `tea pulls merge \(number)`. If tea is not installed or signed in to this host, or the pull request is not mergeable, stop and say so instead of working around it."
        }
        return """
        Merge \(label) now: \(url.absoluteString)

        \(land) Once it is merged, sync the checkout: if this directory is on main with a clean working tree, run `git pull --ff-only`; otherwise leave the working tree alone and say so. Do not rebuild, install or relaunch anything. Reply in a few lines with the merged commit and the request's link.
        """
    }
}
