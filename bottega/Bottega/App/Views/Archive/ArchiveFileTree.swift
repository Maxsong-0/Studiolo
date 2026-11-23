// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The archive's notes down the left of its page: the skeleton folders first, in the
/// vault's order, then the rest, each folder opening to its notes. Follows the folder as
/// it changes (see `ArchivePageModel.rescan`).
struct ArchiveFileTree: View {
    let page: ArchivePageModel

    /// Folders shut by the user; the rest stand open.
    @State private var collapsed: Set<String> = []

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(page.tree) { node in
                    rows(node, depth: 0)
                }
            }
            .padding(.vertical, 8)
            .padding(.horizontal, 6)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder
    private func rows(_ node: ArchiveFileNode, depth: Int) -> some View {
        row(node, depth: depth)
        if let children = node.children, !collapsed.contains(node.path) {
            ForEach(children) { child in
                AnyView(rows(child, depth: depth + 1))
            }
        }
    }

    private func row(_ node: ArchiveFileNode, depth: Int) -> some View {
        let isSelected = node.path == page.selectedPath
        let isFolder = node.isFolder
        return Button {
            if isFolder {
                withAnimation(Chrome.panelSlide) {
                    if collapsed.contains(node.path) { collapsed.remove(node.path) } else { collapsed.insert(node.path) }
                }
            } else {
                page.open(node.path)
            }
        } label: {
            HStack(spacing: 5) {
                if isFolder {
                    Image(systemName: "chevron.right")
                        .chromeChevronFont()
                        .foregroundStyle(Chrome.secondaryText)
                        .rotationEffect(.degrees(collapsed.contains(node.path) ? 0 : 90))
                        .frame(width: 10)
                } else {
                    Image(systemName: "doc.text")
                        .zoomedFont(size: 10)
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 10)
                }
                Text(verbatim: isFolder ? node.name : String(node.name.dropLast(3)))
                    .zoomedFont(size: 12, weight: isFolder ? .medium : .regular)
                    .foregroundStyle(isSelected ? Chrome.accent : Chrome.primaryText)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 0)
            }
            .padding(.leading, CGFloat(depth) * 14 + 6)
            .padding(.trailing, 6)
            .frame(height: 24)
            .background(isSelected ? Chrome.accent.opacity(0.14) : .clear, in: .rect(cornerRadius: Chrome.rowCornerRadius, style: .continuous))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help(node.path)
    }
}
