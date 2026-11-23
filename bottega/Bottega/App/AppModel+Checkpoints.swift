// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

extension AppModel {
    /// Lets go, once, of the file checkpoints still held by threads archived before
    /// archiving took them along (see `archive(_:)`): each turn's two hidden refs, and the
    /// snapshot commits behind them, which an archive kept for good would otherwise hold in
    /// the repository forever. Runs in the background after launch, one pass per repository
    /// the projects live in (one listing and one transaction each), for the threads archived
    /// at that moment. A thread restored meanwhile keeps its refs, as does every thread that
    /// is not archived and every one this library does not know.
    ///
    /// The pass is written down as done once it has gone over every repository within
    /// reach; one git could not be asked about, or whose transaction failed, has another go
    /// at the next launch. A folder gone from disk, or one in no repository, has nothing to
    /// sweep and holds nothing up. A library that did not read at launch (see
    /// `libraryLoadProblem`) is not the user's threads, and its lack of archived ones is no
    /// answer: nothing is swept or written down until a launch that reads it.
    func sweepArchivedCheckpoints() {
        guard !settings.archivedCheckpointsSwept, libraryLoadProblem == nil else { return }
        guard threads.contains(where: \.isArchived) else {
            settings.archivedCheckpointsSwept = true
            return
        }
        // Refs are the repository's, which every worktree of it shares, so the projects'
        // own folders reach all of them (as `ThreadRuntime.pruneCheckpoints` does).
        let directories = Set(projects.map(\.path)).sorted()
        Task.detached(priority: .utility) { [weak self] in
            var complete = true
            // Several projects can live in one repository: folders of a larger one, or a
            // worktree added as a project of its own. Each repository is gone over once.
            var repositories = Set<String>()
            for directory in directories where FileManager.default.fileExists(atPath: directory) {
                let git = Git(directory)
                let repository: String?
                do {
                    repository = try await git.commonDirectory()
                } catch {
                    complete = false
                    continue
                }
                guard let repository, repositories.insert(repository).inserted else { continue }
                // Read as each repository's turn comes rather than once at the start: a
                // thread restored since is a thread again, and keeps what it has.
                guard let archived = await self?.archivedThreadIDs() else { return }
                let swept = await git.deleteCheckpoints(threads: archived)
                if !swept { complete = false }
            }
            if complete { await self?.markArchivedCheckpointsSwept() }
        }
    }

    private func archivedThreadIDs() -> Set<UUID> {
        Set(threads.lazy.filter(\.isArchived).map(\.id))
    }

    private func markArchivedCheckpointsSwept() {
        settings.archivedCheckpointsSwept = true
    }
}
