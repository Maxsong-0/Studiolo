// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// Asks before a stretch of turns has its file changes undone (see
/// `ThreadRuntime.revertFiles`): what undoing would do to each file, then Undo or Cancel.
/// Opened from a turn's changes card and from the changes page.
struct UndoFilesPopover: View {
    let runtime: ThreadRuntime
    let scope: ThreadRuntime.RevertScope
    var paths: [String]? = nil
    let title: String
    let dismiss: () -> Void

    @State private var preview: Result<RevertPreview, Error>?
    @State private var error: String?
    @State private var isWorking = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(verbatim: title)
                .zoomedFont(size: 14, weight: .semibold)
            Text("Only these turns' own changes come out; whatever was changed since stays. The conversation is kept as it is.")
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            switch preview {
            case nil:
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Checking the files…")
                }
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
            case .failure(let failure):
                Label(failure.localizedDescription, systemImage: "exclamationmark.triangle")
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.secondaryText)
            case .success(let found):
                if found.files.isEmpty {
                    Label("Nothing to undo", systemImage: "checkmark.circle")
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.secondaryText)
                } else {
                    RevertFileList(files: found.files)
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.danger)
            }
            HStack(spacing: 8) {
                Spacer()
                Button("Cancel", action: dismiss)
                    .buttonStyle(.glass)
                Button("Undo", action: undo)
                    .buttonStyle(.glassProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canUndo)
            }
            .controlSize(.small)
        }
        .padding(16)
        .frame(width: 340)
        .disabled(isWorking)
        .task(id: runtime.diffRevision) {
            let result: Result<RevertPreview, Error>
            do { result = .success(try await runtime.revertPreview(scope, paths: paths).value) } catch { result = .failure(error) }
            guard !Task.isCancelled else { return }
            preview = result
        }
        .presentedChrome()
    }

    private var canUndo: Bool {
        guard case .success(let found)? = preview else { return false }
        return !found.restorable.isEmpty && !isWorking
    }

    private func undo() {
        guard canUndo else { return }
        isWorking = true
        error = nil
        Task {
            do {
                _ = try await runtime.revertFiles(scope, paths: paths)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            isWorking = false
        }
    }
}

/// The files of an undo preview, one row each: what happens to it, dimmed where it stays
/// as it is. A handful sit in the box; more scroll in a fixed strip.
struct RevertFileList: View {
    let files: [RevertPreview.File]

    var body: some View {
        let rows = LazyVStack(alignment: .leading, spacing: 6) {
            ForEach(files) { file in
                HStack(spacing: 8) {
                    Image(systemName: symbol(file))
                        .foregroundStyle(.secondary)
                    Text(verbatim: file.path)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 4)
                    switch file.outcome {
                    case .conflict:
                        Text("Left as is").foregroundStyle(.secondary)
                    case .unchanged:
                        Text("Already undone").foregroundStyle(.secondary)
                    case .revert:
                        if file.isBinary {
                            Text("Binary").foregroundStyle(.secondary)
                        } else {
                            DiffStatLabel(additions: file.additions, deletions: file.deletions)
                        }
                    }
                }
                .zoomedFont(size: 11)
                .opacity(file.outcome == .revert ? 1 : 0.45)
                .help(file.outcome == .conflict ? "\(file.path)\nChanged again since in the same lines, so these turns' changes there cannot be taken out alone." : file.path)
            }
        }
        if files.count > 6 {
            ScrollView { rows }.frame(height: 130)
        } else {
            rows
        }
    }

    private func symbol(_ file: RevertPreview.File) -> String {
        switch file.outcome {
        case .conflict: "hand.raised"
        case .unchanged: "checkmark.circle"
        case .revert: "doc"
        }
    }
}
