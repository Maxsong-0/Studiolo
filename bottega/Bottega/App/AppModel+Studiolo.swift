// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Foundation

/// The user's Studiolo vault as the sidebar sees it: the archives under `项目/`, kept
/// current by a watch on the folder, and which Studiolo sheet is up.
///
/// Studiolo stays the place for everything else (editing, policy, the board, the audit);
/// here the user only sees the archives and opens new ones.
@MainActor
@Observable
final class StudioloStore {
    /// Every archive in the vault, by name. Empty with no vault.
    private(set) var archives: [StudioloArchive] = [] {
        didSet { byPrefix = Dictionary(archives.map { ($0.prefix, $0) }, uniquingKeysWith: { first, _ in first }) }
    }
    /// The same archives by prefix, for the rows that show one. Observed like the list: a
    /// row's dot, tooltip and menu read an archive through here and must hear it change.
    private var byPrefix: [String: StudioloArchive] = [:]
    /// Whether the chosen folder is a vault and has been read: until then no row, menu
    /// item or sheet mentions Studiolo.
    private(set) var isAvailable = false
    private(set) var root: URL?

    /// The Studiolo sheet on screen, one at a time for the whole window. Asked for through
    /// `AppModel.presentStudioloSheet`, never set directly.
    var sheet: StudioloSheetRequest?
    /// The sidebar's group of archives no project is linked to starts folded.
    var archivesCollapsed = true

    @ObservationIgnored private var vault: StudioloVault?
    /// Told each time the archives change, after they are in place.
    @ObservationIgnored var onArchivesChanged: (() -> Void)?

    func archive(_ prefix: String) -> StudioloArchive? { byPrefix[prefix] }

    /// Watches the vault at `path`, or nothing when it is not one. Called at launch and
    /// whenever Settings points somewhere else.
    func watch(_ path: String) {
        let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath).standardizedFileURL
        guard url != root || vault == nil else { return }
        vault?.stop()
        vault = nil
        root = url
        archives = []
        isAvailable = false
        guard StudioloVault.isVault(url) else { return }
        let vault = StudioloVault(root: url) { [weak self] archives in
            Task { @MainActor in self?.deliver(archives, from: url) }
        }
        self.vault = vault
        vault.start()
    }

    private func deliver(_ archives: [StudioloArchive], from url: URL) {
        // A vault let go since may still have had a read in flight.
        guard url == root, vault != nil else { return }
        self.archives = archives
        isAvailable = true
        onArchivesChanged?()
    }
}

/// One asking for a Studiolo sheet. Every request is its own item, so asking again always
/// puts the sheet up, even for the same project and even if an earlier one never showed.
struct StudioloSheetRequest: Identifiable {
    let id = UUID()
    let kind: StudioloSheet
}

/// The Studiolo sheets, each naming what it is about.
enum StudioloSheet: Hashable {
    /// Just added from the folder panel and matched by no archive: open one, link one, or not now.
    case offer(projectID: UUID)
    /// A new archive for the project.
    case create(projectID: UUID)
    /// Pick the archive the project is filed under.
    case linkProject(projectID: UUID)
    /// Pick the project an unlinked archive belongs to.
    case linkArchive(prefix: String)
}

extension AppModel {
    /// Starts reading the vault in Settings; from then on every change to the archives
    /// links the projects they name.
    func startStudiolo() {
        studiolo.onArchivesChanged = { [weak self] in self?.autoLinkStudiolo() }
        studiolo.watch(settings.studioloVaultPath)
    }

    /// Settings' vault folder: stored, then watched in place of the last one. The archive
    /// pages of the last one go with it.
    func setStudioloVault(_ path: String) {
        settings.studioloVaultPath = path
        studiolo.watch(path)
        dropArchivePages()
    }

    /// The archive the project is linked to, when the vault has it.
    func studioloArchive(for project: Project) -> StudioloArchive? {
        project.studioloPrefix.flatMap(studiolo.archive)
    }

    /// Archives no project is linked to, for the sidebar's own group.
    var unlinkedStudioloArchives: [StudioloArchive] {
        let linked = Set(projects.compactMap(\.studioloPrefix))
        return studiolo.archives.filter { !linked.contains($0.prefix) }
    }

    /// The archive whose dashboard names this folder, for a project nothing has decided yet.
    func studioloAutoLink(forPath path: String) -> StudioloLink? {
        guard studiolo.isAvailable else { return nil }
        return StudioloIndex.pairing(projectPath: path, archives: studiolo.archives, home: NSHomeDirectory())
            .map { .archive($0.prefix) }
    }

    /// Links every undecided project an archive's dashboard names. A link or an unlinking
    /// the user chose is never touched; several checkouts of one repository all link.
    func autoLinkStudiolo() {
        for project in projects where project.studioloLink == nil {
            guard let link = studioloAutoLink(forPath: project.path) else { continue }
            updateProject(project.id) { $0.studioloLink = link }
        }
    }

    /// Puts a Studiolo sheet up on the main actor's next turn, once the menu, popover or
    /// panel that asked for it has gone: a sheet presented while a popover is still on its
    /// way out can be swallowed (the same reason `PopoverItem` runs its action late).
    func presentStudioloSheet(_ kind: StudioloSheet) {
        Task { @MainActor in self.studiolo.sheet = StudioloSheetRequest(kind: kind) }
    }

    /// After the folder panel: a project that has just joined and that no archive names is
    /// offered one, once.
    func offerStudioloArchive(for projectID: UUID) {
        guard studiolo.isAvailable, let project = self.project(projectID), project.studioloLink == nil else { return }
        presentStudioloSheet(.offer(projectID: projectID))
    }

    func linkStudiolo(_ projectID: UUID, to prefix: String) {
        updateProject(projectID) { $0.studioloLink = .archive(prefix) }
    }

    func unlinkStudiolo(_ projectID: UUID) {
        updateProject(projectID) { $0.studioloLink = .unlinked }
    }

    /// Opens a new archive for the project in the vault and links the project to it. The
    /// write happens off the main thread; the sidebar shows the archive once the watch
    /// reads it back.
    func createStudioloArchive(for projectID: UUID, name: String, summary: String) async -> Result<StudioloArchiveWriter.Outcome, StudioloArchiveWriter.Failure> {
        guard let root = studiolo.root, let project = self.project(projectID) else { return .failure(.notVault) }
        let request = StudioloArchiveWriter.Request(
            vault: root,
            name: name,
            summary: summary,
            projectPath: project.path,
            today: Self.studioloDay.string(from: .now),
            globalRules: URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".claude/CLAUDE.md")
        )
        let result = await Task.detached(priority: .userInitiated) { () -> Result<StudioloArchiveWriter.Outcome, StudioloArchiveWriter.Failure> in
            do throws(StudioloArchiveWriter.Failure) {
                return .success(try StudioloArchiveWriter.create(request))
            } catch {
                return .failure(error)
            }
        }.value
        if case let .success(outcome) = result { linkStudiolo(projectID, to: outcome.prefix) }
        return result
    }

    private static let studioloDay: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter
    }()

    // MARK: - Leaving for Studiolo, Obsidian and Finder

    /// The archive's board in the Studiolo web app. The web app's hash reads `project` on
    /// its board, run and operations views only, so the board is where a link can land.
    func studioloWebURL(prefix: String) -> URL? {
        let base = settings.studioloWebURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !base.isEmpty, let encoded = Self.componentEncoded(prefix) else { return nil }
        let trimmed = base.hasSuffix("/") ? String(base.dropLast()) : base
        return URL(string: "\(trimmed)/#view=board&project=\(encoded)")
    }

    /// The archive's INDEX in Obsidian, which names a vault by its folder's name.
    func studioloObsidianURL(prefix: String) -> URL? {
        guard let root = studiolo.root,
              let vault = Self.componentEncoded(root.lastPathComponent),
              let file = Self.componentEncoded(prefix + "/" + String(StudioloIndex.indexFile.dropLast(3)))
        else { return nil }
        return URL(string: "obsidian://open?vault=\(vault)&file=\(file)")
    }

    func studioloFolder(prefix: String) -> URL? {
        studiolo.root?.appendingPathComponent(prefix)
    }

    /// Everything but the unreserved characters escaped, so a `&`, `=`, `+` or space in a
    /// name reaches the other side as itself.
    private static func componentEncoded(_ text: String) -> String? {
        text.addingPercentEncoding(withAllowedCharacters: CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"))
    }

    // MARK: - Threads the new archive asks for

    /// A thread in the project with the brief waiting in its composer, unsent, for the user
    /// to read before it goes.
    func openStudioloThread(in project: Project, prefill: String) {
        guard let thread = newThread(in: project) else { return }
        runtime(for: thread.id).draft = ComposerDraft(text: prefill)
    }

    /// Have Crew fill in the rest of a fresh archive, in the project it belongs to.
    func completeStudioloArchive(prefix: String, for projectID: UUID) {
        guard let project = self.project(projectID) else { return }
        openStudioloThread(in: project, prefill: Self.completionBrief(prefix: prefix, vault: studiolo.root?.path ?? settings.studioloVaultPath))
    }

    enum StudioloRegistrationDraft: Equatable {
        /// Waiting in a new thread of the project linked to `项目/Studiolo`.
        case drafted
        /// No project is linked to it, so the brief went to the clipboard.
        case copied
    }

    /// The whitelist registration a new archive needs before Studiolo serves it, drafted in
    /// the project linked to `项目/Studiolo`; with no such project it goes to the clipboard.
    /// The caller says which, in place: it is asked from a sheet, where an alert cannot show.
    func draftStudioloRegistration(prefix: String) -> StudioloRegistrationDraft {
        let brief = Self.registrationBrief(prefix: prefix)
        if let studioloProject = studioloOwnProject {
            openStudioloThread(in: studioloProject, prefill: brief)
            return .drafted
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(brief, forType: .string)
        return .copied
    }

    /// An unlinked archive's code folder, from the folder panel: it joins the sidebar (or
    /// is found there), is linked to the archive, and opens a thread, as Add project does.
    func addStudioloProject(for prefix: String) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Add project"
        panel.message = "Choose the code folder for \(studiolo.archive(prefix)?.name ?? prefix)."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let project = addProject(at: url)
        linkStudiolo(project.id, to: prefix)
        newThread(in: project)
    }

    static let studioloOwnPrefix = StudioloIndex.projectsFolder + "/Studiolo"

    /// Studiolo's own repository among the projects linked to its archive: every checkout
    /// and worktree of it links there, and the registration belongs in the one its dashboard
    /// names, or else the one nearest the root.
    private var studioloOwnProject: Project? {
        let linked = projects.filter { $0.studioloPrefix == Self.studioloOwnPrefix }
        let named = Set(StudioloIndex.expandedPaths(studiolo.archive(Self.studioloOwnPrefix)?.card?.repositoryPaths ?? [], home: NSHomeDirectory()))
        return linked.first { named.contains(URL(fileURLWithPath: $0.path).standardizedFileURL.path) }
            ?? linked.min { $0.path.count < $1.path.count }
    }

    private static func completionBrief(prefix: String, vault: String) -> String {
        """
        Studiolo 档案 `\(prefix)/` 刚由 Bottega 按 vault `_模板/README.md` 开档 SOP 第 1、2 步建好（骨架、`CLAUDE.md` 快照、INDEX 的状态／代码仓库／最近日志／当前重点四行）。vault 在 `\(vault)`。

        请按开档 SOP 第 3 步补全，内容以本仓库的实际代码为准：
        1. `01-框架/` 三件套：技术栈（含版本号）、使用方式（含可复制命令）、目录结构；
        2. `00-总览/INDEX.md`：一句话简介、当前版本、当前重点；
        3. 在 `02-日志/` 写当日日志，记下这次建档。
        """
    }

    private static func registrationBrief(prefix: String) -> String {
        """
        把新档案 `\(prefix)` 纳入 Studiolo 的供给白名单，三处登记：
        1. `scripts/replica_common.py` 的 `ALLOW`；
        2. agent 启动参数 `--allow`；
        3. claude／codex 能力集的 `read.project_whitelist`（走策略平面 L3，一次性验证码由 Max 本人发布）。

        做法参照 Studiolo 归档 `2026-09-20-T-095-Tillerwright纳入供给白名单`；按 Studiolo 的并发 SOP 先查重、认领再动手。
        """
    }
}
