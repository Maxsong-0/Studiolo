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
    /// The digest of the text as it was when it came on screen. A number can come to stand for another ask: when LMAS starts a new
    /// log, or opens it again.
    @State private var read: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            AskText(ask: ask).font(.body)
            if let claim { claim }
            if ask.answer == nil, let read, read != ask.textDigest {
                // Where the buttons are not, so that a click meant for them does not land on this.
                Button("This Has Changed, I Have Read It Again") { self.read = ask.textDigest }
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                buttons
            }
        }
        .onAppear { read = ask.textDigest }
    }

    private func approve() {
        if read == ask.textDigest { model.approve(ask) }
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
            HStack(spacing: 8) {
                if model.isDeciding(ask.id) {
                    ProgressView().controlSize(.small)
                    Text("Sending…").font(.callout).foregroundStyle(.secondary)
                } else {
                    HStack(spacing: 8) {
                        Button {
                            approve()
                        } label: {
                            Label("Approve with Touch ID", systemImage: "touchid")
                                .font(.system(size: 12.5, weight: .semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 14)
                                .frame(height: 32)
                                .background(Color(red: 10 / 255, green: 108 / 255, blue: 1), in: .capsule)
                                .contentShape(.capsule)
                        }
                        .buttonStyle(.plain)
                        .keyboardShortcut(hasShortcut ? KeyboardShortcut(.return, modifiers: .command) : nil)
                        Button { model.decline(ask.id) } label: {
                            Text("Decline")
                                .font(.system(size: 12.5, weight: .semibold))
                                .padding(.horizontal, 14)
                                .frame(height: 32)
                                .contentShape(.capsule)
                        }
                        .buttonStyle(.plain)
                    }
                    .disabled(!model.isLive)
                    .opacity(model.isLive ? 1 : 0.45)
                    .help(model.isLive ? "" : "Not connected to LMAS")
                }
                Spacer()
                if ask.onlyDevice {
                    Label("Only Touch ID counts for this one", systemImage: "touchid").font(.caption).foregroundStyle(.secondary)
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
                    // Another text: its button, not the keys.
                    .onChange(of: ask.textDigest) { hasReachedEnd = false }
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
