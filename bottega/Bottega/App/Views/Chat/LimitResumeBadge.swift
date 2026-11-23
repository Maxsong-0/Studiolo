// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The usage-limit notice once its chat waits to pick itself back up: the words and the
/// mark in the timeline's pill, with the reader's two ways out of the wait beside them.
struct LimitResumeBadge: View {
    @Environment(\.chatZoom) private var zoom
    @Environment(AppModel.self) private var model
    /// The notice as stored; its message is the words while the wait is live.
    let notice: Notice
    /// The chat whose auto-continue the notice belongs to.
    let threadID: UUID

    var body: some View {
        let waiting = model.autoContinue.resumesAt[threadID] != nil
        HStack(spacing: 8) {
            Image(systemName: "info.circle")
                .font(.chat(.callout, zoom: zoom))
                .foregroundStyle(.secondary)
            Text(verbatim: words(waiting: waiting))
                .font(.chat(.callout, weight: .medium, zoom: zoom))
                .foregroundStyle(Chrome.primaryText.opacity(0.9))
            if waiting {
                Button { model.autoContinue.cancel(threadID) } label: {
                    Text("Cancel")
                        .font(.chat(.caption, weight: .semibold, zoom: zoom))
                        .foregroundStyle(Chrome.secondaryText)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Stop the automatic continue")
                Button { model.autoContinue.handOff(threadID) } label: {
                    Text("New thread")
                        .font(.chat(.caption, weight: .semibold, zoom: zoom))
                        .foregroundStyle(Chrome.secondaryText)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Stop the automatic continue and pick the work up in a new thread, where you can choose another model")
            }
        }
        .padding(.leading, TimelineMetrics.pillLeading)
        .padding(.trailing, TimelineMetrics.pillTrailing)
        .padding(.vertical, TimelineMetrics.pillVertical)
        .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: TimelineMetrics.pillRadius, style: .continuous))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.trailing, 96)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(words(waiting: waiting)))
    }

    /// The words for the current state of the wait.
    private func words(waiting: Bool) -> String {
        if waiting { return notice.message }
        guard let outcome = model.autoContinue.outcomes[threadID] else { return notice.message }
        switch outcome {
        case .cancelled: return "Usage limit reached. Auto-continue cancelled."
        case .handedOff: return "Usage limit reached. Continuing in a new thread."
        case .resumed: return "Usage limit reached. Continued automatically."
        }
    }
}
