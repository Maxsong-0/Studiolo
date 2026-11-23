// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

// The Studiolo pieces of the sidebar: the status mark on a linked project's icon, the
// archive card, the group of archives no project is linked to, and the sheets that link
// and open archives. `SidebarView` only mounts them.

extension StudioloTone {
    /// The circle the dashboard's status starts with, in its own colour: the same word the
    /// user reads in Obsidian and in Studiolo, so it does not follow the theme's hues.
    var color: Color {
        switch self {
        case .green: Color(nsColor: .systemGreen)
        case .yellow: Color(nsColor: .systemYellow)
        case .blue: Color(nsColor: .systemBlue)
        case .red: Color(nsColor: .systemRed)
        case .neutral: Chrome.secondaryText
        }
    }
}

// MARK: - A linked project's row

/// The dot on a linked project's icon, in its archive's status colour. Always shown, not
/// only on hover: the row's trailing edge belongs to its hover buttons.
struct StudioloProjectMark: View {
    @Environment(AppModel.self) private var model
    let project: Project

    var body: some View {
        if let archive = model.studioloArchive(for: project) {
            StudioloStatusDot(tone: archive.card?.tone ?? .neutral, sitsOnCorner: true)
                .accessibilityLabel(Text(archive.card?.status ?? "Studiolo archive"))
        }
    }
}

struct StudioloStatusDot: View {
    let tone: StudioloTone
    /// The dot's size at the stock text size.
    var size: CGFloat = 6
    /// Set on an icon's corner, a little past it, as the badge on a sidebar row's icon.
    var sitsOnCorner = false

    /// The window's text size: the icon the dot sits on grows with it (see
    /// `SidebarIconBadge`), and so do the dot and its offset.
    @Environment(\.chatZoom) private var zoom

    var body: some View {
        let side = ChatZoom.scaled(size, by: zoom)
        Circle()
            .fill(tone.color)
            .frame(width: side, height: side)
            .offset(
                x: sitsOnCorner ? ChatZoom.scaled(3, by: zoom) : 0,
                y: sitsOnCorner ? ChatZoom.scaled(2, by: zoom) : 0
            )
    }
}

/// A project row's Studiolo parts: the tooltip with the archive's dashboard, and the card.
/// Whether the card is open belongs to the list the row is in, not to the model: the same
/// project has a row in every sidebar on screen (the column, even while it is folded away,
/// and the floating panel or the Threads popover), and only the one whose menu was used
/// should open it.
struct StudioloProjectRowChrome: ViewModifier {
    @Environment(AppModel.self) private var model
    let project: Project
    @Binding var isCardPresented: Bool

    func body(content: Content) -> some View {
        content
            .help(StudioloCardText.tooltip(for: project, model: model))
            .popover(isPresented: $isCardPresented, arrowEdge: .trailing) {
                StudioloCardPopover(prefix: project.studioloPrefix, projectID: project.id)
            }
    }
}

/// The Studiolo items of a project row's ellipsis popover and context menu. Built from
/// value snapshots with a weak model, as the row's own context menu items are: the AppKit
/// menu outlives the right-click, and its callbacks must not keep the row alive. `showCard`
/// opens the card on the row, through its list (see `StudioloProjectRowChrome`).
enum StudioloRowActions {
    @MainActor
    static func items(project: Project, model: AppModel, showCard: @escaping @MainActor () -> Void) -> [RowAction] {
        guard model.studiolo.isAvailable else { return [] }
        let projectID = project.id
        guard project.studioloPrefix != nil else {
            return [
                RowAction(title: "Link Studiolo archive…", symbol: "link", startsGroup: true) { [weak model, projectID] in
                    model?.presentStudioloSheet(.linkProject(projectID: projectID))
                },
                RowAction(title: "Create Studiolo archive…", symbol: "folder.badge.plus") { [weak model, projectID] in
                    model?.presentStudioloSheet(.create(projectID: projectID))
                },
            ]
        }
        var items: [RowAction] = []
        if let prefix = project.studioloPrefix, model.studioloArchive(for: project) != nil {
            items.append(RowAction(title: "Open Studiolo archive", symbol: "doc.text", startsGroup: true) { [weak model, prefix] in
                model?.showArchive(prefix)
            })
            items.append(RowAction(title: "Studiolo dashboard…", symbol: "books.vertical") { [weak model] in
                guard model != nil else { return }
                showCard()
            })
            items.append(RowAction(title: "Change Studiolo archive…", symbol: "link") { [weak model, projectID] in
                model?.presentStudioloSheet(.linkProject(projectID: projectID))
            })
        } else {
            // Linked to an archive the vault no longer has: renamed or removed since.
            items.append(RowAction(title: "Link Studiolo archive…", symbol: "link", startsGroup: true) { [weak model, projectID] in
                model?.presentStudioloSheet(.linkProject(projectID: projectID))
            })
        }
        items.append(RowAction(title: "Unlink Studiolo archive", symbol: "minus.circle") { [weak model, projectID] in
            model?.unlinkStudiolo(projectID)
        })
        return items
    }
}

/// The words the tooltip and the card share.
enum StudioloCardText {
    @MainActor
    static func tooltip(for project: Project, model: AppModel) -> String {
        guard model.studiolo.isAvailable, let prefix = project.studioloPrefix else { return "" }
        guard let archive = model.studiolo.archive(prefix) else {
            return "Studiolo: the vault has no \(prefix) any more"
        }
        guard let card = archive.card else { return "Studiolo · \(archive.name): no dashboard yet" }
        return (["Studiolo · \(archive.name)"] + fields(card).map { "\($0.label): \($0.value)" })
            .joined(separator: "\n")
    }

    /// The dashboard rows the card lists, in order, the empty ones left out.
    static func fields(_ card: StudioloIndexCard, full: Bool = false) -> [(label: String, value: String)] {
        var rows: [(String, String?)] = [
            ("Status", card.status),
            ("Open bugs", card.openBugs),
            ("Focus", card.focus),
            ("Latest log", card.latestLog),
        ]
        if full {
            rows.append(("Version", card.version))
            rows.append(("Repository", card.repository))
        }
        return rows.compactMap { label, value in
            guard let value, !value.isEmpty else { return nil }
            return (label, value)
        }
    }
}

// MARK: - The archive card

/// An archive's dashboard in full, with the ways out to Studiolo, Obsidian and Finder.
struct StudioloCardPopover: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    /// The archive; nil once the project the card hangs on has been unlinked.
    let prefix: String?
    /// The project whose row the card hangs on; nil on an unlinked archive's row.
    var projectID: UUID?

    var body: some View {
        let archive = prefix.flatMap(model.studiolo.archive)
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                StudioloStatusDot(tone: archive?.card?.tone ?? .neutral, size: 8)
                Text(verbatim: archive?.name ?? prefix ?? "Studiolo")
                    .zoomedFont(size: 13, weight: .semibold)
                    .foregroundStyle(Chrome.primaryText)
                    .lineLimit(1)
            }
            if let archive, let prefix {
                if let card = archive.card {
                    VStack(alignment: .leading, spacing: 7) {
                        ForEach(StudioloCardText.fields(card, full: true), id: \.label) { field in
                            VStack(alignment: .leading, spacing: 1) {
                                Text(verbatim: field.label)
                                    .zoomedFont(size: 11, weight: .medium)
                                    .foregroundStyle(Chrome.secondaryText)
                                Text(verbatim: field.value)
                                    .zoomedFont(size: 12)
                                    .foregroundStyle(Chrome.primaryText)
                                    .textSelection(.enabled)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                } else {
                    note("This archive has no \(StudioloIndex.indexFile) yet.")
                }
                HStack(spacing: 8) {
                    Button("Open in Studiolo") { open(model.studioloWebURL(prefix: prefix)) }
                        .buttonStyle(.glassProminent)
                        .disabled(model.studioloWebURL(prefix: prefix) == nil)
                    Button("Obsidian") { open(model.studioloObsidianURL(prefix: prefix)) }
                        .buttonStyle(.glass)
                        .help("Open the archive's INDEX in Obsidian")
                    Button("Finder") {
                        if let folder = model.studioloFolder(prefix: prefix) { Workspace.revealInFinder(folder.path) }
                    }
                    .buttonStyle(.glass)
                    .help("Reveal the archive in Finder")
                }
                .controlSize(.small)
                .padding(.top, 2)
            } else {
                if let prefix {
                    note("The vault has no \(prefix) any more: it was renamed or removed.")
                } else {
                    note("The project is not linked to an archive.")
                }
                if let projectID {
                    Button("Link an archive…") {
                        // Closed first; the sheet follows on the next turn.
                        dismiss()
                        model.presentStudioloSheet(.linkProject(projectID: projectID))
                    }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                }
            }
        }
        .padding(14)
        .frame(width: 340, alignment: .leading)
        .presentedChrome()
    }

    private func note(_ text: String) -> some View {
        Text(verbatim: text)
            .zoomedFont(size: 12)
            .foregroundStyle(Chrome.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
    }

    private func open(_ url: URL?) {
        guard let url else { return }
        NSWorkspace.shared.open(url)
    }
}

// MARK: - Archives no project is linked to

/// The foot of the project list: the vault's archives that no project in the sidebar is
/// linked to, folded until opened. Each can be linked to a project, given its code folder,
/// or opened in Studiolo.
struct StudioloArchiveSection: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let archives = model.studiolo.isAvailable ? model.unlinkedStudioloArchives : []
        if !archives.isEmpty {
            let collapsed = model.studiolo.archivesCollapsed
            StudioloArchiveHeader(count: archives.count, collapsed: collapsed)
            if !collapsed {
                ForEach(archives) { archive in
                    StudioloArchiveRow(archive: archive)
                }
            }
        }
    }
}

private struct StudioloArchiveHeader: View {
    @Environment(AppModel.self) private var model
    let count: Int
    let collapsed: Bool

    var body: some View {
        Button {
            withAnimation(Chrome.panelSlide) { model.studiolo.archivesCollapsed.toggle() }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                Image(systemName: "chevron.right")
                    .zoomedFont(size: 9, weight: .semibold)
                    .rotationEffect(.degrees(collapsed ? 0 : 90))
                    .animation(Chrome.panelSlide, value: collapsed)
                Text(verbatim: "Studiolo archives")
                    .zoomedFont(size: 12, weight: .semibold)
                Text(verbatim: "\(count)")
                    .zoomedFont(size: 12, monospacedDigit: true)
                    .opacity(0.8)
                    .fixedSize()
                    .contentTransition(.identity)
                Spacer(minLength: 4)
            }
            .foregroundStyle(Chrome.secondaryText)
            .padding(.horizontal, Chrome.rowHorizontalPadding)
            .padding(.top, 14)
            .padding(.bottom, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help(collapsed ? "Show the archives no project is linked to" : "Hide the Studiolo archives")
        .accessibilityLabel(Text("Studiolo archives, \(count)"))
        .accessibilityValue(Text(collapsed ? "Collapsed" : "Expanded"))
    }
}

private struct StudioloArchiveRow: View {
    @Environment(AppModel.self) private var model
    let archive: StudioloArchive

    @State private var isMenuPresented = false
    @State private var isCardPresented = false

    var body: some View {
        SidebarRow(
            title: archive.name,
            accessoryWidth: 20,
            action: { model.showArchive(archive.prefix) },
            icon: {
                SidebarIconBadge { SidebarSymbol("books.vertical") }
                    .opacity(0.7)
                    .overlay(alignment: .bottomTrailing) {
                        StudioloStatusDot(tone: archive.card?.tone ?? .neutral, sitsOnCorner: true)
                    }
            },
            accessory: { hovering in
                if hovering || isMenuPresented {
                    RowActionsButton(actions: Self.actions(prefix: archive.prefix, model: model, dashboard: $isCardPresented), isPresented: $isMenuPresented)
                }
            }
        )
        .help(archive.card?.status.map { "\(archive.name) · \($0)" } ?? archive.name)
        .popover(isPresented: $isCardPresented, arrowEdge: .trailing) {
            StudioloCardPopover(prefix: archive.prefix)
        }
        .contextMenu { [prefix = archive.prefix, model, dashboard = $isCardPresented] in
            RowActionMenuButtons(actions: Self.actions(prefix: prefix, model: model, dashboard: dashboard))
        }
    }

    /// Value snapshots and a weak model, for the context menu as for the project rows;
    /// the dashboard card's binding alone, never the row.
    private static func actions(prefix: String, model: AppModel, dashboard: Binding<Bool>) -> [RowAction] {
        [
            RowAction(title: "Open archive", symbol: "doc.text") { [weak model, prefix] in
                model?.showArchive(prefix)
            },
            RowAction(title: "Dashboard…", symbol: "books.vertical") { [dashboard] in
                dashboard.wrappedValue = true
            },
            RowAction(title: "Link to a project…", symbol: "link", startsGroup: true) { [weak model, prefix] in
                model?.presentStudioloSheet(.linkArchive(prefix: prefix))
            },
            RowAction(title: "Add its code folder…", symbol: "folder.badge.plus") { [weak model, prefix] in
                model?.addStudioloProject(for: prefix)
            },
            RowAction(title: "Open in Studiolo", symbol: "safari", startsGroup: true) { [weak model, prefix] in
                guard let url = model?.studioloWebURL(prefix: prefix) else { return }
                NSWorkspace.shared.open(url)
            },
        ]
    }
}

// MARK: - Sheets

/// The window's Studiolo sheets, one at a time from `StudioloStore.sheet`. Mounted on the
/// window's root rather than the sidebar, so the offer after Add project… shows with the
/// sidebar hidden too, and only once however many sidebars are on screen.
struct StudioloSheets: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        @Bindable var studiolo = model.studiolo
        content.sheet(item: $studiolo.sheet) { request in
            Group {
                switch request.kind {
                case let .offer(projectID): StudioloOfferSheet(projectID: projectID)
                case let .create(projectID): StudioloCreateSheet(projectID: projectID)
                case let .linkProject(projectID): StudioloLinkProjectSheet(projectID: projectID)
                case let .linkArchive(prefix): StudioloLinkArchiveSheet(prefix: prefix)
                }
            }
            .presentedChrome()
        }
    }
}

/// Shown once after Add project… when no archive's dashboard names the new folder.
private struct StudioloOfferSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let projectID: UUID

    var body: some View {
        let project = model.project(projectID)
        VStack(alignment: .leading, spacing: 12) {
            Text(verbatim: "File \(project?.name ?? "the project") in Studiolo?")
                .zoomedFont(size: 17, weight: .semibold)
            Text(verbatim: "No archive in the vault names \((project?.path ?? "") as NSString).abbreviatingWithTildeInPath as its repository. Open a new one for it, or link one the vault already has.")
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 12) {
                Button("Not now", role: .cancel) { dismiss() }
                    .buttonStyle(.glass)
                    .keyboardShortcut(.cancelAction)
                Spacer()
                // Closed first, as the card closes itself: the next sheet follows on the
                // next turn, once this one has gone (see `presentStudioloSheet`).
                Button("Link existing…") {
                    dismiss()
                    model.presentStudioloSheet(.linkProject(projectID: projectID))
                }
                    .buttonStyle(.glass)
                Button("Create archive…") {
                    dismiss()
                    model.presentStudioloSheet(.create(projectID: projectID))
                }
                    .buttonStyle(.glassProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(.top, 4)
        }
        .padding(20)
        .frame(width: 460)
        .onAppear { if project == nil { dismiss() } }
    }
}

/// A new archive for a project: its name and a line about it, then what to do next.
private struct StudioloCreateSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let projectID: UUID

    @State private var name = ""
    @State private var summary = ""
    @State private var isCreating = false
    @State private var failure: StudioloArchiveWriter.Failure?
    @State private var created: StudioloArchiveWriter.Outcome?
    @State private var completionOpened = false
    @State private var registration: AppModel.StudioloRegistrationDraft?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let created {
                result(created)
            } else {
                form
            }
        }
        .padding(20)
        .frame(width: 480)
        // The archive is written and linked whatever the sheet does meanwhile; closed half
        // way, the result page with what is left to do would never show.
        .interactiveDismissDisabled(isCreating)
        .onAppear {
            guard name.isEmpty, let project = model.project(projectID) else { return }
            name = project.name
        }
    }

    @ViewBuilder
    private var form: some View {
        let project = model.project(projectID)
        let problem = model.studiolo.root.flatMap { StudioloArchiveWriter.validate(name: name, vault: $0) }
        Text(verbatim: "Create a Studiolo archive")
            .zoomedFont(size: 17, weight: .semibold)
        Text(verbatim: "The vault's project skeleton, your global CLAUDE.md beside it, and a dashboard naming \((project?.path ?? "") as NSString).abbreviatingWithTildeInPath as the repository.")
            .zoomedFont(size: 12)
            .foregroundStyle(Chrome.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
        VStack(alignment: .leading, spacing: 5) {
            Text(verbatim: "Name")
                .zoomedFont(size: 11, weight: .medium)
                .foregroundStyle(Chrome.secondaryText)
            TextField("", text: $name, prompt: Text(verbatim: project?.name ?? "Archive name"))
                .textFieldStyle(.roundedBorder)
            if let problem, !name.isEmpty {
                Text(verbatim: problem.localizedDescription)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.danger)
            } else {
                Text(verbatim: "Becomes \(StudioloIndex.projectsFolder)/\(StudioloArchiveWriter.archiveName(name)) in the vault.")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
        VStack(alignment: .leading, spacing: 5) {
            Text(verbatim: "One line about it (optional)")
                .zoomedFont(size: 11, weight: .medium)
                .foregroundStyle(Chrome.secondaryText)
            TextField("", text: $summary, prompt: Text(verbatim: "What the project is"))
                .textFieldStyle(.roundedBorder)
        }
        if let failure {
            Text(verbatim: failure.localizedDescription)
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.danger)
                .fixedSize(horizontal: false, vertical: true)
        }
        HStack(spacing: 12) {
            Spacer()
            Button("Cancel", role: .cancel) { dismiss() }
                .buttonStyle(.glass)
                .keyboardShortcut(.cancelAction)
                .disabled(isCreating)
            Button {
                Task { await create() }
            } label: {
                if isCreating {
                    ProgressView().controlSize(.small)
                } else {
                    Text("Create")
                }
            }
            .buttonStyle(.glassProminent)
            .keyboardShortcut(.defaultAction)
            .disabled(problem != nil || isCreating || project == nil)
        }
        .padding(.top, 4)
    }

    @ViewBuilder
    private func result(_ created: StudioloArchiveWriter.Outcome) -> some View {
        Text(verbatim: "Archive created")
            .zoomedFont(size: 17, weight: .semibold)
        Text(verbatim: "\(created.prefix) is in the vault and the project is linked to it. Studiolo serves it to agents once it is on the whitelist.")
            .zoomedFont(size: 12)
            .foregroundStyle(Chrome.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
        if created.missingRules {
            Text(verbatim: "~/.claude/CLAUDE.md was not there, so the archive has no CLAUDE.md yet.")
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.warning)
                .fixedSize(horizontal: false, vertical: true)
        }
        VStack(alignment: .leading, spacing: 8) {
            nextStep(
                title: "Have Crew complete it",
                detail: "A thread in this project with the brief for the rest of the opening checklist, waiting to be sent.",
                done: completionOpened ? "Opened" : nil
            ) {
                model.completeStudioloArchive(prefix: created.prefix, for: projectID)
                completionOpened = true
            }
            // Goes once Studiolo keeps its own project register (its plan B): the
            // whitelist then follows the vault and there is nothing to register.
            nextStep(
                title: "Draft the Studiolo registration",
                detail: "The three whitelist entries the archive needs, as a brief in the project linked to \(AppModel.studioloOwnPrefix).",
                done: registration.map { $0 == .drafted ? "Opened" : "Copied: no project is linked to \(AppModel.studioloOwnPrefix)" }
            ) {
                registration = model.draftStudioloRegistration(prefix: created.prefix)
            }
        }
        .padding(.top, 2)
        HStack {
            Spacer()
            Button("Done") { dismiss() }
                .buttonStyle(.glassProminent)
                .keyboardShortcut(.defaultAction)
        }
        .padding(.top, 4)
    }

    private func nextStep(title: String, detail: String, done: String?, action: @escaping () -> Void) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: title)
                    .zoomedFont(size: 13)
                Text(verbatim: done ?? detail)
                    .zoomedFont(size: 11)
                    .foregroundStyle(done == nil ? Chrome.secondaryText : Chrome.success)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            Button(done == nil ? "Open" : "Again", action: action)
                .buttonStyle(.glass)
                .controlSize(.small)
        }
        .padding(10)
        .background(Chrome.overlay(0.05), in: .rect(cornerRadius: Chrome.blockCornerRadius, style: .continuous))
    }

    private func create() async {
        failure = nil
        isCreating = true
        defer { isCreating = false }
        switch await model.createStudioloArchive(for: projectID, name: name, summary: summary) {
        case let .success(outcome): created = outcome
        case let .failure(error): failure = error
        }
    }
}

/// Picks the archive a project is filed under.
private struct StudioloLinkProjectSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let projectID: UUID

    var body: some View {
        let project = model.project(projectID)
        let linkedBy = Dictionary(grouping: model.projects.filter { $0.id != projectID }, by: { $0.studioloPrefix ?? "" })
        StudioloPickerSheet(
            title: "Link \(project?.name ?? "the project") to an archive",
            detail: "The archive's status shows on the project's icon, and its card opens from the project's menu.",
            items: model.studiolo.archives.map { archive in
                StudioloPickerItem(
                    id: archive.prefix,
                    title: archive.name,
                    detail: linkedBy[archive.prefix].map { "Also linked to " + $0.map(\.name).joined(separator: ", ") },
                    tone: archive.card?.tone ?? .neutral,
                    isChecked: project?.studioloPrefix == archive.prefix
                )
            },
            onPick: { prefix in
                model.linkStudiolo(projectID, to: prefix)
                dismiss()
            },
            onCancel: { dismiss() }
        )
    }
}

/// Picks the project an unlinked archive belongs to.
private struct StudioloLinkArchiveSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let prefix: String

    var body: some View {
        let archive = model.studiolo.archive(prefix)
        StudioloPickerSheet(
            title: "Link \(archive?.name ?? prefix) to a project",
            detail: "Pick the project in the sidebar whose code this archive is about.",
            items: model.projects.map { project in
                let current = project.studioloPrefix.map { model.studiolo.archive($0)?.name ?? $0 }
                return StudioloPickerItem(
                    id: project.id.uuidString,
                    title: project.name,
                    detail: current.map { "Linked to \($0) now" } ?? (project.path as NSString).abbreviatingWithTildeInPath,
                    tone: nil,
                    isChecked: false
                )
            },
            onPick: { id in
                guard let projectID = UUID(uuidString: id) else { return }
                model.linkStudiolo(projectID, to: prefix)
                dismiss()
            },
            onCancel: { dismiss() }
        )
    }
}

private struct StudioloPickerItem: Identifiable {
    let id: String
    let title: String
    let detail: String?
    /// An archive's status; nil for a project, which has none of its own.
    let tone: StudioloTone?
    let isChecked: Bool
}

private struct StudioloPickerSheet: View {
    let title: String
    let detail: String
    let items: [StudioloPickerItem]
    let onPick: (String) -> Void
    let onCancel: () -> Void

    @State private var query = ""

    var body: some View {
        let shown = query.isEmpty ? items : items.filter { $0.title.localizedCaseInsensitiveContains(query) }
        VStack(alignment: .leading, spacing: 12) {
            Text(verbatim: title)
                .zoomedFont(size: 17, weight: .semibold)
            Text(verbatim: detail)
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            TextField("", text: $query, prompt: Text("Filter"))
                .textFieldStyle(.roundedBorder)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    ForEach(shown) { item in
                        StudioloPickerRow(item: item) { onPick(item.id) }
                    }
                }
            }
            .frame(height: 320)
            .background(Chrome.overlay(0.03), in: .rect(cornerRadius: Chrome.blockCornerRadius, style: .continuous))
            HStack {
                Spacer()
                Button("Cancel", role: .cancel, action: onCancel)
                    .buttonStyle(.glass)
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}

private struct StudioloPickerRow: View {
    let item: StudioloPickerItem
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Image(systemName: "checkmark")
                    .zoomedFont(size: 11, weight: .semibold)
                    .opacity(item.isChecked ? 1 : 0)
                    .frame(width: 14)
                if let tone = item.tone {
                    StudioloStatusDot(tone: tone, size: 7)
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: item.title)
                        .zoomedFont(size: 13)
                        .foregroundStyle(Chrome.primaryText)
                        .lineLimit(1)
                    if let detail = item.detail {
                        Text(verbatim: detail)
                            .zoomedFont(size: 11)
                            .foregroundStyle(Chrome.secondaryText)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 4)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isHovering ? Chrome.overlay(0.06) : .clear, in: .rect(cornerRadius: Chrome.rowCornerRadius, style: .continuous))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
    }
}
