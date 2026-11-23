// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// Commands the menu bar, the command palette and the main window share, where the work is
/// more than one call.
extension AppModel {
    /// Search threads: the caret goes to the thread search in the list on screen. With the list
    /// nowhere in sight (the column hidden and no floating panel standing in for it) the column
    /// opens first, and from another window the main one comes forward for it.
    func focusSidebarSearch() {
        if !WindowManager.shared.isMainWindowKey { WindowManager.shared.showMain() }
        if !settings.sidebarFloats, !sidebar.isVisible { sidebar.toggle() }
        sidebar.requestSearchFocus()
    }

    /// Command-Return with the keyboard in no text being typed (see `ThreadWindow`): the
    /// selected chat's top request is approved, as its empty chat box approves it, and only
    /// while nothing is typed there, so the chord never approves over words waiting to go.
    /// False when there is nothing to approve.
    func approveSelectedRequest() -> Bool {
        guard let id = selectedThreadID, let runtime = existingRuntime(for: id), runtime.draftIsEmpty else { return false }
        return runtime.approveTopRequest()
    }
}
