// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// Rename, pin or put away a line (LMAS section 11). It hangs from the line's header or its row in the sidebar; ⌘K
/// does the same from the keyboard. Its buttons and Return are what sign, through `WorkshopModel`.
struct ArrangePanel: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let line: BottegaKit.Line

    @State private var name = ""
    @FocusState private var isNaming: Bool

    var body: some View {
        let arrangement = model.state.arrangement(of: line.id)
        let own = model.state.ownTitle(of: line)
        let canSend = model.client != nil
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Arrange line").font(.system(size: 13, weight: .semibold))
                Spacer()
                Text(verbatim: UntrustedText.line(line.id, limit: 24))
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 5) {
                TextField(text: $name, prompt: Text(verbatim: own)) { Text("Name") }
                    .font(.system(size: 13))
                    .glassField()
                    .focused($isNaming)
                    .onSubmit { if model.rename(line.id, to: name) { dismiss() } }
                    .disabled(!canSend)
                    .accessibilityLabel("Name")
                HStack(spacing: 6) {
                    Text("Return renames it").foregroundStyle(.secondary)
                    Spacer()
                    if arrangement.title != nil {
                        // A plain or glass button, not AppKit's own: that follows the mouse itself, so the press is
                        // no longer the event being handled when it acts, and the gate turns it away.
                        Button { model.rename(line.id, to: "") } label: { Text("Use its own name").foregroundStyle(.tint) }
                            .buttonStyle(.plain)
                            .disabled(!canSend)
                            .help(Text(verbatim: "Back to “\(own)”"))
                    }
                }
                .font(.system(size: 10.5))
            }
            HStack(spacing: 8) {
                button(
                    arrangement.isPinned ? "Unpin" : "Pin", symbol: arrangement.isPinned ? "pin.slash" : "pin",
                    help: arrangement.isPinned ? "Stop keeping it at the top" : "Keep it at the top of its list"
                ) { model.setPinned(line.id, !arrangement.isPinned) }
                button(
                    arrangement.isArchived ? "Unarchive" : "Archive",
                    symbol: arrangement.isArchived ? "tray.and.arrow.up" : "archivebox",
                    help: arrangement.isArchived ? "Put it back with the others" : "Put it away at the bottom of its list"
                ) { model.setArchived(line.id, !arrangement.isArchived) }
            }
            .disabled(!canSend)
            if let note = note(canSend: canSend) {
                Text(verbatim: note).font(.system(size: 10.5)).foregroundStyle(.secondary)
            }
        }
        .padding(14)
        .frame(width: 280)
        .onAppear {
            name = model.state.title(of: line)
            isNaming = true
        }
        // What LMAS took comes back as the name; what the user is typing meanwhile is left alone.
        .onChange(of: model.state.title(of: line)) { old, new in
            if name.trimmingCharacters(in: .whitespacesAndNewlines) == old { name = new }
        }
    }

    private func note(canSend: Bool) -> String? {
        if !canSend { return "Recording, nothing can be sent" }
        return model.isArranging(line.id) ? "On its way to LMAS" : nil
    }

    private func button(_ title: String, symbol: String, help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: symbol).frame(maxWidth: .infinity)
        }
        .buttonStyle(.glass)
        .help(help)
    }
}
