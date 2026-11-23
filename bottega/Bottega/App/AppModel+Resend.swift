// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

// A message sent again from the top of a thread of its own: the message menu's "Resend in
// new thread" (see `UserMessageRow`).

extension AppModel {
    /// Sends `text` and `attachments`, a message from the thread `sourceID`, as the first
    /// message of a new thread in the same project. The new thread starts the way the New
    /// thread button starts one (the chat on screen's provider and model, the default
    /// workspace) and is selected. A thread that starts in a worktree waits for it before
    /// sending, so the turn runs where the thread keeps working; when the worktree cannot be
    /// made, the message stays in the new thread's chat box to send by hand.
    func resend(_ text: String, attachments: [Attachment], from sourceID: UUID) async {
        guard let source = thread(sourceID), let project = project(source.projectID) else { return }
        let inWorktree = settings.defaultWorkspaceMode == .worktree
        // A local start, whatever the default: the worktree, if one is wanted, is made below,
        // where the send can wait for it; `newThread` would make it on a task of its own.
        guard let thread = newThread(in: project, workspace: .local) else { return }
        let newRuntime = runtime(for: thread.id)
        // As the box showed it when it went out: the quotes it led with as quotes, then the words.
        let parts = ReplyQuote.peel(text)
        newRuntime.draft = ComposerDraft(text: parts.body, attachments: attachments, quotes: parts.quotes)
        if inWorktree {
            await createWorktree(for: thread.id)
            guard self.thread(thread.id)?.worktreePath != nil else { return }
        }
        newRuntime.send()
    }
}
