import SwiftUI

struct ApprovalCard: View {
    let request: ApprovalRequest
    let runtime: ThreadRuntime

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: request.symbol)
                    .foregroundStyle(Chrome.warning)
                Text(request.headline)
                    .font(.headline)
                Spacer()
            }
            switch request.kind {
            case .command:
                Text(request.title)
                    .font(.system(.callout, design: .monospaced))
                    .textSelection(.enabled)
                    .lineLimit(8)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 10, style: .continuous))
            case .plan:
                Text("Approve the plan above to let the agent start building.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            default:
                Text(request.title)
                    .font(.callout)
                    .textSelection(.enabled)
            }
            if let detail = request.detail, !detail.isEmpty {
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
            if let reason = request.reason, !reason.isEmpty {
                Text(reason)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 8) {
                Spacer()
                // Only the card at the top of the stack answers to the chord: two cards both
                // claiming it left neither of them reliably approving.
                let ownsShortcut = request.id == runtime.approvals.first?.id
                ForEach(request.options.reversed()) { option in
                    if option.role == .approve {
                        Button(option.title) { runtime.resolve(request, option: option) }
                            .buttonStyle(.glassProminent)
                            .keyboardShortcut(ownsShortcut ? KeyboardShortcut(.return, modifiers: .command) : nil)
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
}
