// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// A Studiolo archive in the main area, in place of a thread: its dashboard across the
/// top, its notes down the left, the open note in the middle (rendered, or side by side
/// with its source while it is edited), and on the right where each version and its
/// author will show once Studiolo keeps the archive's history. Opened from an archive's
/// row in the sidebar (see `AppModel.showArchive`).
struct ArchivePage: View {
    @Environment(AppModel.self) private var model
    let prefix: String

    var body: some View {
        if let page = model.archivePage(for: prefix) {
            ArchivePageBody(page: page, archive: model.studiolo.archive(prefix))
        } else {
            RightSidebarEmpty(text: "No Studiolo vault", detail: "Choose the vault in Settings › Studiolo, and the archive opens here.")
        }
    }
}

private struct ArchivePageBody: View {
    @Environment(AppModel.self) private var model
    let page: ArchivePageModel
    /// The archive as the sidebar knows it; nil once the vault no longer has it.
    let archive: StudioloArchive?

    static let treeWidth: CGFloat = 230
    static let historyWidth: CGFloat = 220

    // Each alert is shown from a flag of its own, set from the page's state: an alert bound
    // straight to that state let the binding's setter (run as the alert closes, before its
    // button's action) clear the very state the action needed.
    @State private var asksAboutOpening = false
    @State private var asksAboutConflict = false
    @State private var asksAboutLeaving = false

    var body: some View {
        VStack(spacing: 0) {
            ArchiveToolbar(page: page, archive: archive)
            ArchiveOverviewCard(page: page, card: archive?.card)
                .padding(.horizontal, Chrome.chromeHorizontalPadding)
                .padding(.bottom, 12)
            Divider()
            HStack(spacing: 0) {
                ArchiveFileTree(page: page)
                    .frame(width: Self.treeWidth)
                Divider()
                ArchiveDocumentView(page: page)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                Divider()
                ArchiveHistoryColumn()
                    .frame(width: Self.historyWidth)
            }
        }
        .onChange(of: page.pendingOpen != nil, initial: true) { _, asks in asksAboutOpening = asks }
        .onChange(of: page.conflict != nil, initial: true) { _, asks in asksAboutConflict = asks }
        .onChange(of: model.archiveLeave?.prefix == page.prefix, initial: true) { _, asks in asksAboutLeaving = asks }
        .alert("Unsaved changes", isPresented: $asksAboutOpening) {
            Button("Save and open") { Task { await page.openPending(discardingDraft: false) } }
            Button("Discard and open", role: .destructive) { Task { await page.openPending(discardingDraft: true) } }
            Button("Cancel", role: .cancel) { page.pendingOpen = nil }
        } message: {
            Text("\(page.selectedPath ?? "This note") has changes that are not saved.")
        }
        .alert("Changed on disk in the same lines", isPresented: $asksAboutConflict) {
            Button("Keep mine as a copy") { Task { await page.keepDraftAsCopy() } }
            Button("Discard my changes", role: .destructive) { page.discardConflictingDraft() }
            Button("Keep editing", role: .cancel) { page.keepEditingAfterConflict() }
        } message: {
            Text("Something else changed \(page.selectedPath ?? "this note") since you opened it, in the lines you changed too, so the two cannot be merged. Your version can be kept beside it as a copy, or you can go on editing and save again later.")
        }
        .alert("Unsaved changes", isPresented: $asksAboutLeaving) {
            Button("Save and leave") { Task { await model.leaveArchive(saving: true) } }
            Button("Discard and leave", role: .destructive) { Task { await model.leaveArchive(saving: false) } }
            Button("Stay", role: .cancel) { model.stayOnArchive() }
        } message: {
            Text("\(page.selectedPath ?? "This note") has changes that are not saved.")
        }
    }
}

/// The row over the page: the archive's name and status, preview or edit, save, and the
/// ways out to Studiolo, Obsidian and Finder.
private struct ArchiveToolbar: View {
    @Environment(AppModel.self) private var model
    @Bindable var page: ArchivePageModel
    let archive: StudioloArchive?

    var body: some View {
        HStack(spacing: 10) {
            StudioloStatusDot(tone: archive?.card?.tone ?? .neutral, size: 9)
            Text(verbatim: archive?.name ?? (page.prefix as NSString).lastPathComponent)
                .zoomedFont(size: 15, weight: .semibold)
                .foregroundStyle(Chrome.primaryText)
                .lineLimit(1)
            if archive == nil {
                Text("Not in the vault any more")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer(minLength: 8)
            if page.document != nil {
                Picker("", selection: $page.isEditing) {
                    Text("Preview").tag(false)
                    Text("Edit").tag(true)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 150)
                .help("Read the note as it renders, or edit its Markdown beside the rendering")
            }
            if page.isDirty {
                Button("Save") { Task { await page.save() } }
                    .buttonStyle(.glassProminent)
                    .keyboardShortcut("s", modifiers: .command)
                    .disabled(page.isSaving)
                    .help("Save the note (⌘S)")
            }
            ChromeCapsule {
                ChromeIconButton(symbol: "safari", help: "Open the archive's board in Studiolo") {
                    open(model.studioloWebURL(prefix: page.prefix))
                }
                .disabled(model.studioloWebURL(prefix: page.prefix) == nil)
                ChromeDivider()
                ChromeIconButton(symbol: "book.closed", help: "Open the archive in Obsidian") {
                    open(model.studioloObsidianURL(prefix: page.prefix))
                }
                ChromeDivider()
                ChromeIconButton(symbol: "folder", help: "Reveal the archive in Finder") {
                    Workspace.revealInFinder(page.root.path)
                }
            }
        }
        .controlSize(.small)
        .padding(.horizontal, Chrome.chromeHorizontalPadding)
        .padding(.top, Chrome.chromeTopPadding)
        .padding(.bottom, 12)
    }

    private func open(_ url: URL?) {
        guard let url else { return }
        NSWorkspace.shared.open(url)
    }
}

/// The right column: where the archive's versions and each line's author go once Studiolo
/// keeps the archive's history. Until then it says so.
private struct ArchiveHistoryColumn: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("History")
                .zoomedFont(size: 12, weight: .semibold)
                .foregroundStyle(Chrome.secondaryText)
                .padding(.horizontal, Chrome.listInset)
                .padding(.top, 12)
            RightSidebarEmpty(
                text: "Versions arrive with Studiolo's content history",
                detail: "Each version of a note, who wrote it and which lines, once the server keeps them."
            )
            .frame(maxWidth: .infinity)
            Spacer(minLength: 0)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }
}
