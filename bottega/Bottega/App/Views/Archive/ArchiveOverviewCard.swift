// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The dashboard across the top of an archive's page: what its INDEX says (the rows the
/// sidebar's card shows, see `StudioloIndexCard`) and how many tasks its board has in
/// progress. Each tile opens the note it was read from.
struct ArchiveOverviewCard: View {
    let page: ArchivePageModel
    let card: StudioloIndexCard?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 8) {
                tile("Status", value: card?.status, tone: card?.tone) { page.open(StudioloIndex.indexFile) }
                tile("Version", value: card?.version) { page.open(StudioloIndex.indexFile) }
                tile("In progress", value: page.openTasks.map { "\($0) \($0 == 1 ? "task" : "tasks")" }) { page.open(ArchiveFiles.tasksFile) }
                tile("Open bugs", value: card?.openBugs) { page.open(StudioloIndex.indexFile) }
                tile("Latest log", value: card?.latestLog) {
                    if let log = ArchiveFiles.latestLog(in: page.tree) { page.open(log.path) } else { page.open(StudioloIndex.indexFile) }
                }
            }
            if let focus = card?.focus, !focus.isEmpty {
                tile("Focus", value: focus, lines: 3) { page.open(StudioloIndex.indexFile) }
            }
            if card == nil {
                Text("This archive has no \(StudioloIndex.indexFile) yet.")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
    }

    /// One row of the dashboard as a tile; the value is the row's text, or a dash for a
    /// row the INDEX lacks.
    private func tile(_ label: String, value: String?, tone: StudioloTone? = nil, lines: Int = 2, open: @escaping () -> Void) -> some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: label)
                    .zoomedFont(size: 10.5, weight: .medium)
                    .foregroundStyle(Chrome.secondaryText)
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    if let tone {
                        StudioloStatusDot(tone: tone, size: 7)
                    }
                    Text(verbatim: value?.nilIfEmpty ?? "—")
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.primaryText)
                        .lineLimit(lines)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Chrome.overlay(0.05), in: .rect(cornerRadius: Chrome.blockCornerRadius, style: .continuous))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help("Open the note this is read from")
    }
}
