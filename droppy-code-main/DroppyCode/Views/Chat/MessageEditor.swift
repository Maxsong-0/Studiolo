import SwiftUI

/// Edits a message already sent. Its text and files come back to be changed, and sending
/// rewinds the thread to just before it, puts the files back too when the switch is on,
/// and sends the edit from there. The edit lives on the thread (`ThreadRuntime.messageEdit`),
/// so the editor closing with the thread and opening again later shows it as it was left.
struct MessageEditor: View {
    @Bindable var edit: PromptEdit
    let runtime: ThreadRuntime

    /// What the working tree has changed since the message, or why that cannot be told.
    private enum Changes {
        case checking
        case found(RevertPreview)
        case unavailable(String)

        init(_ result: Result<RevertPreview, Error>?) {
            switch result {
            case .success(let preview): self = .found(preview)
            case .failure(let error): self = .unavailable(error.localizedDescription)
            case nil: self = .checking
            }
        }
    }

    @State private var changes: Changes
    @State private var restoresFiles = true
    @State private var error: String?
    @State private var isSending = false

    init(edit: PromptEdit, runtime: ThreadRuntime) {
        self.edit = edit
        self.runtime = runtime
        // A preview the pencil's hover already fetched opens the editor complete, with
        // nothing to grow into place a moment later.
        _changes = State(initialValue: Changes(runtime.cachedRevertPreview(for: edit.id)))
    }

    var body: some View {
        PromptEditor(
            title: "Edit message",
            text: $edit.text,
            attachments: $edit.attachments,
            onSubmit: send,
            onCancel: cancel
        ) {
            fileChanges
            Text("Sending replaces this message and everything after it.")
                .font(.callout)
                .foregroundStyle(.secondary)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .foregroundStyle(Chrome.danger)
            }
        } actions: {
            Button("Cancel", role: .cancel, action: cancel)
                .buttonStyle(.glass)
            Button("Send", action: send)
                .buttonStyle(.glassProminent)
                .disabled(!canSend)
        }
        .disabled(isSending)
        .task(id: edit.id) {
            let result: Result<RevertPreview, Error>
            do { result = .success(try await runtime.revertPreview(for: edit.id).value) } catch { result = .failure(error) }
            guard !Task.isCancelled else { return }
            changes = Changes(result)
        }
    }

    private var canSend: Bool {
        // Until the file check is in, sending would silently keep the files.
        if case .checking = changes { return false }
        return !edit.isEmpty && !isSending && !runtime.isRunning
    }

    @ViewBuilder private var fileChanges: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch changes {
            case .checking:
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Checking for file changes…")
                }
                .foregroundStyle(.secondary)
            case .unavailable(let reason):
                Label(reason, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
            case .found(let preview) where preview.files.isEmpty:
                Label("This thread changed no files since this message", systemImage: "checkmark.circle")
                    .foregroundStyle(.secondary)
            case .found(let preview) where preview.restorable.isEmpty:
                // Everything it changed has been changed again by something else since.
                Label(preview.files.count == 1 ? "The file it changed has changed again since, so it stays as it is"
                      : "The \(preview.files.count) files it changed have changed again since, so they stay as they are",
                      systemImage: "hand.raised")
                    .foregroundStyle(.secondary)
                fileList(preview.files)
            case .found(let preview):
                let restorable = preview.restorable
                Toggle(isOn: $restoresFiles) {
                    HStack(spacing: 8) {
                        Text("Undo the file changes this thread made")
                        Spacer(minLength: 8)
                        Text(restorable.count == 1 ? "1 file" : "\(restorable.count) files")
                            .foregroundStyle(.secondary)
                        DiffStatLabel(additions: preview.additions, deletions: preview.deletions)
                    }
                    .frame(maxWidth: .infinity)
                }
                .toggleStyle(.switch)
                .controlSize(.small)
                fileList(preview.files)
            }
        }
        .font(.callout)
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Chrome.overlay(0.05), in: .rect(cornerRadius: 12, style: .continuous))
    }

    /// The thread's files: the ones going back, and dimmed, the ones kept because something
    /// else changed them since. A handful sit in the box; more scroll in a fixed strip.
    @ViewBuilder private func fileList(_ files: [RevertPreview.File]) -> some View {
        let rows = LazyVStack(alignment: .leading, spacing: 6) {
            ForEach(files) { file in
                HStack(spacing: 8) {
                    Image(systemName: file.isKept ? "hand.raised" : "doc")
                        .foregroundStyle(.secondary)
                    Text(verbatim: file.path)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 4)
                    if file.isKept {
                        Text("Kept").foregroundStyle(.secondary)
                    } else if file.isBinary {
                        Text("Binary").foregroundStyle(.secondary)
                    } else {
                        DiffStatLabel(additions: file.additions, deletions: file.deletions)
                    }
                }
                .font(.caption)
                // Off, or kept: the row stays as a reminder of what is not going back.
                .opacity(restoresFiles && !file.isKept ? 1 : 0.45)
                .help(file.isKept ? "\(file.path)\nChanged again since this thread last touched it, so it is left alone." : file.path)
            }
        }
        if files.count > 5 {
            ScrollView { rows }.frame(height: 110)
        } else {
            rows
        }
    }

    private func send() {
        guard canSend else { return }
        var restore = false
        if case .found(let preview) = changes { restore = restoresFiles && !preview.files.isEmpty }
        isSending = true
        error = nil
        Task {
            do {
                try await runtime.resend(edit.id, text: edit.text, attachments: edit.attachments, restoreFiles: restore)
                runtime.messageEdit = nil
            } catch {
                self.error = error.localizedDescription
            }
            isSending = false
        }
    }

    private func cancel() {
        runtime.messageEdit = nil
    }
}
