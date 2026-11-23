// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The open note in the middle of an archive's page: rendered, or while it is edited its
/// Markdown on the left and the rendering of what is typed on the right, a moment behind
/// the typing. Wiki links open the note they name on this page; every other link goes out
/// as it would from a chat.
struct ArchiveDocumentView: View {
    @Bindable var page: ArchivePageModel

    /// The draft as last rendered: a keystroke never re-renders the whole note at once.
    @State private var previewText = ""

    var body: some View {
        VStack(spacing: 0) {
            if let problem = page.problem {
                banner(problem, symbol: "exclamationmark.triangle", tint: Chrome.danger)
            } else if page.changedOnDisk {
                banner("This note changed on disk while you were editing. Saving merges the two.", symbol: "arrow.triangle.merge", tint: Chrome.warning)
            }
            if page.document == nil {
                RightSidebarEmpty(text: "Pick a note", detail: "The archive's notes are on the left; the dashboard's tiles open the notes they are read from.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if page.isEditing {
                HStack(spacing: 0) {
                    editor
                    Divider()
                    preview(previewText)
                }
                .task(id: page.draft) {
                    // The rendering follows the typing a moment behind, never per keystroke.
                    if previewText.isEmpty { previewText = page.draft; return }
                    try? await Task.sleep(for: .milliseconds(250))
                    guard !Task.isCancelled else { return }
                    previewText = page.draft
                }
            } else {
                preview(page.draft)
            }
        }
        .onChange(of: page.selectedPath) { _, _ in previewText = page.draft }
    }

    private var editor: some View {
        TextEditor(text: $page.draft)
            .font(.system(size: 12.5, design: .monospaced))
            .scrollContentBackground(.hidden)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func preview(_ text: String) -> some View {
        ScrollView {
            MarkdownView(text: WikiLinks.rewrite(text))
                .padding(.horizontal, 24)
                .padding(.vertical, 20)
                .frame(maxWidth: 760, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // A wiki link opens its note here; anything else leaves the app as usual. A
        // picture is found within the archive (see `ArchivePageModel.imageURL`).
        .environment(\.openURL, OpenURLAction { url in
            guard let target = WikiLinks.target(of: url) else { return .systemAction }
            page.open(link: target)
            return .handled
        })
        .environment(\.markdownImages, { [page] reference in page.imageURL(for: reference) })
    }

    private func banner(_ text: String, symbol: String, tint: Color) -> some View {
        HStack(spacing: 8) {
            Image(systemName: symbol)
                .foregroundStyle(tint)
            Text(verbatim: text)
                .zoomedFont(size: 11.5)
                .foregroundStyle(Chrome.primaryText)
                .lineLimit(2)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(tint.opacity(0.1))
    }
}
