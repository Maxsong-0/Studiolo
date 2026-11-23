// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

struct ApprovalCard: View {
    let request: ApprovalRequest
    let runtime: ThreadRuntime

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: request.symbol)
                    .zoomedFont(.body)
                    .foregroundStyle(Chrome.warning)
                Text(request.headline)
                    .zoomedFont(.headline)
                Spacer()
            }
            switch request.kind {
            case .command:
                Text(request.title)
                    .zoomedFont(.callout, design: .monospaced)
                    .textSelection(.enabled)
                    .lineLimit(8)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 10, style: .continuous))
            case .plan:
                Text("Approve the plan above to let the agent start building.")
                    .zoomedFont(.callout)
                    .foregroundStyle(.secondary)
            default:
                Text(request.title)
                    .zoomedFont(.callout)
                    .textSelection(.enabled)
            }
            if let detail = request.detail, !detail.isEmpty {
                Text(detail)
                    .zoomedFont(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
            if let reason = request.reason, !reason.isEmpty {
                Text(reason)
                    .zoomedFont(.callout)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 8) {
                Spacer()
                // Only the card at the top of the stack answers to the chord. The chord is the
                // chat box's send, which approves this card while nothing is typed there (see
                // `ComposerView.submit`, and `ThreadWindow` with the keyboard in no box): no
                // button shortcut, which would answer across the whole window, a hand's box
                // included, and take the send from words typed.
                let ownsShortcut = request.id == runtime.approvals.first?.id
                ForEach(request.options.reversed()) { option in
                    if option.role == .approve {
                        Button(option.title) { runtime.resolve(request, option: option) }
                            .buttonStyle(.glassProminent)
                            .help(approveHelp(option, ownsShortcut: ownsShortcut))
                    } else {
                        Button(option.title) { runtime.resolve(request, option: option) }
                            .buttonStyle(.glass)
                    }
                }
            }
        }
        .padding(16)
        // The same hue as the card's symbol, so a theme recolours the whole card together.
        .glassEffect(.regular.tint(Chrome.warning.opacity(0.1)), in: .rect(cornerRadius: 20, style: .continuous))
    }

    /// The approve button's tooltip, naming the chord that presses it, on the top card alone.
    private func approveHelp(_ option: ApprovalRequest.Option, ownsShortcut: Bool) -> String {
        guard ownsShortcut else { return "" }
        return "\(option.title) (\(KeyChord.composerSend.description) while the chat box is empty)"
    }
}
