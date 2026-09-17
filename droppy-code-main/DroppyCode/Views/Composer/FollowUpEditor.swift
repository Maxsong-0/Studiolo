import SwiftUI

/// Edits a queued follow-up in place. The text and files being edited live on the thread
/// (`ThreadRuntime.followUpEdit`), not here, so the editor closing with the thread and
/// opening again later shows the edit exactly as it was left.
struct FollowUpEditor: View {
    @Bindable var edit: PromptEdit
    let runtime: ThreadRuntime

    var body: some View {
        PromptEditor(
            title: "Edit follow-up",
            text: $edit.text,
            attachments: $edit.attachments,
            onSubmit: save,
            onCancel: cancel
        ) {
        } actions: {
            Button("Cancel", role: .cancel, action: cancel)
                .buttonStyle(.glass)
            Button("Save", action: save)
                .buttonStyle(.glassProminent)
                .disabled(edit.isEmpty)
        }
    }

    private func save() {
        guard !edit.isEmpty else { return }
        runtime.updateFollowUp(edit.id, text: edit.text, attachments: edit.attachments)
        runtime.followUpEdit = nil
    }

    private func cancel() {
        runtime.followUpEdit = nil
    }
}
