// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import SwiftUI

/// An ask's text exactly as LMAS wrote it: every line, nothing shortened, hidden characters spelled out. This is
/// what an approval signs for.
struct AskText: View {
    let ask: Ask

    var body: some View {
        let shown = UntrustedText.revealing(ask.text)
        VStack(alignment: .leading, spacing: 8) {
            Text(verbatim: shown.text)
                .textSelection(.enabled)
                .lineSpacing(2)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            if shown.hidden > 0 {
                Label(
                    "\(Wording.count(shown.hidden, "hidden character is", "hidden characters are")) spelled out above, as ⟨U+…⟩.",
                    systemImage: "eye.trianglebadge.exclamationmark"
                )
                .font(.callout)
                .foregroundStyle(.orange)
            }
        }
    }
}

/// An ask with its buttons.
struct AskPanel: View {
    @Environment(WorkshopModel.self) private var model
    let ask: Ask
    /// ⌘Return approves. Only where all of the text has been on screen.
    var hasShortcut = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            AskText(ask: ask).font(.body)
            if let claim { claim }
            buttons
        }
    }

    /// What the agent says about its own work, kept apart from what LMAS found out for itself.
    private var claim: (some View)? {
        guard let merge = ask.about.merge.flatMap({ model.state.merges[$0] }), let card = merge.card.flatMap({ model.state.cards[$0] }),
            case .done(let result, _) = card.end, let letter = model.state.messages[result]
        else { return nil as AgentClaim? }
        return AgentClaim(letter: letter)
    }

    @ViewBuilder private var buttons: some View {
        if let answer = ask.answer {
            StatusChip(text: Wording.answer(answer.word, via: answer.via), symbol: answer.word == "approved" ? "checkmark.circle" : "xmark.circle")
        } else {
            HStack(spacing: 10) {
                if ask.onlyDevice {
                    Label("Only Touch ID counts for this one", systemImage: "touchid").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if model.isDeciding(ask.id) {
                    ProgressView().controlSize(.small)
                    Text("Sending…").font(.callout).foregroundStyle(.secondary)
                } else {
                    GlassEffectContainer {
                        HStack(spacing: 8) {
                            Button("Decline") { model.decline(ask.id) }
                                .buttonStyle(.glass)
                            Button {
                                model.approve(ask)
                            } label: {
                                Label("Approve", systemImage: "touchid")
                            }
                            .buttonStyle(.glassProminent)
                            .keyboardShortcut(hasShortcut ? KeyboardShortcut(.return, modifiers: .command) : nil)
                        }
                    }
                    .disabled(!model.isLive)
                    .help(model.isLive ? "" : "Not connected to LMAS")
                }
            }
        }
    }
}

private struct AgentClaim: View {
    let letter: Message
    @State private var isOpen = false

    var body: some View {
        DisclosureGroup(isExpanded: $isOpen) {
            PlainBlock(text: letter.body, limit: 200, monospaced: false)
        } label: {
            HStack(spacing: 6) {
                Text(verbatim: "What \(UntrustedText.line(letter.from, limit: 40)) says")
                Text("Unverified")
                    .font(.caption)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(.quaternary, in: .capsule)
            }
            .font(.callout)
        }
    }
}

struct AskSheet: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let id: String
    @State private var hasReachedEnd = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let ask = model.state.asks[id] {
                HStack {
                    Text(verbatim: "Asked by \(UntrustedText.line(ask.by, limit: 40))").font(.headline)
                    Spacer()
                    Text(verbatim: UntrustedText.line(id, limit: 24)).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                ScrollView { AskPanel(ask: ask, hasShortcut: hasReachedEnd) }
                    .onScrollGeometryChange(for: Bool.self) { geometry in
                        geometry.visibleRect.height > 0 && geometry.contentSize.height - geometry.visibleRect.maxY < 8
                    } action: { _, isAtEnd in
                        if isAtEnd { hasReachedEnd = true }
                    }
                    // Another text to read.
                    .onChange(of: ask.text) { hasReachedEnd = false }
                    .frame(maxHeight: 460)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Close") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }
}
