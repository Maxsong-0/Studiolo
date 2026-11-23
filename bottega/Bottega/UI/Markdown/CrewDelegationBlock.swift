// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The lead's delegation block, the fenced `crew` JSON at the end of its reply, as the
/// card it stands for: the mark, "Summoning hands", and under them one row per hand with
/// the glyph of the hand being summoned, its task, and the project it goes to when that
/// is not the chat's own. Never the JSON itself. The card reads the block as it streams
/// and adds each hand as its brief closes; while nothing has closed yet, or when the
/// block cannot be read, it says the hands are being summoned and nothing more.
struct CrewDelegationBlock: View {
    @Environment(\.chatZoom) private var zoom
    let json: String
    var sent = false

    var body: some View {
        let reading = Self.reading(of: json)
        let delegations = reading.delegations
        let streaming = !reading.isComplete && !sent
        // The mark and the title share a line; the hands run under both, each glyph in
        // the mark's column, so the list reads from the card's own left edge.
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .center, spacing: TimelineMetrics.iconSpacing) {
                CrewMarkImage()
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 16, height: 16)
                    .accessibilityHidden(true)
                Text(verbatim: Self.title(for: delegations, streaming: streaming, sent: sent))
                    .font(.chat(.callout, weight: .medium, zoom: zoom))
                    .contentTransition(.opacity)
                if sent {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.chat(.callout, weight: .medium, zoom: zoom))
                        .foregroundStyle(Chrome.success)
                        .transition(.scale.combined(with: .opacity))
                        .accessibilityLabel("sent")
                }
            }
            if !delegations.isEmpty {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(Array(delegations.enumerated()), id: \.offset) { _, delegation in
                        handRow(delegation)
                    }
                    if streaming {
                        HStack(alignment: .center, spacing: TimelineMetrics.iconSpacing) {
                            Color.clear.frame(width: 16, height: 16)
                            Text("Writing the next brief…")
                                .font(.chat(.caption, zoom: zoom))
                                .foregroundStyle(Chrome.secondaryText)
                        }
                    }
                }
            }
        }
        .padding(.leading, TimelineMetrics.pillLeading)
        .padding(.trailing, TimelineMetrics.pillTrailing)
        .padding(.vertical, TimelineMetrics.pillVertical)
        .background(.tint.opacity(0.1), in: RoundedRectangle(cornerRadius: TimelineMetrics.pillRadius, style: .continuous))
        .overlay {
            // Out: one wave of the accent flows through the card, as through the hands panel.
            if sent {
                DotFieldSweep(tint: Chrome.accentNSColor)
                    .clipShape(RoundedRectangle(cornerRadius: TimelineMetrics.pillRadius, style: .continuous))
                    .transition(.opacity)
            }
        }
        .animation(Chrome.panelSlide, value: sent)
        .animation(Chrome.panelSlide, value: delegations.count)
        .transition(.opacity)
        .accessibilityElement(children: .combine)
    }

    /// One hand on its way: the glyph of the hand the lead named (the mark when it named
    /// none), its task, and the project it goes to when the brief sends it elsewhere.
    private func handRow(_ delegation: CrewDelegation) -> some View {
        HStack(alignment: .center, spacing: TimelineMetrics.iconSpacing) {
            if let persona = Self.persona(named: delegation.name) {
                CrewGlyph(persona: persona, size: 16)
                    .accessibilityLabel(Text(persona.name))
            } else {
                CrewMarkImage()
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 16, height: 16)
                    .accessibilityHidden(true)
            }
            Text(verbatim: delegation.task)
                .font(.chat(.caption, zoom: zoom))
                .foregroundStyle(Chrome.primaryText.opacity(0.75))
                .lineLimit(1)
                .truncationMode(.tail)
            if let project = Self.projectName(delegation.project) {
                HStack(spacing: 3) {
                    Image(systemName: "arrow.right")
                        .accessibilityHidden(true)
                    Text(verbatim: project)
                }
                .font(.chat(.caption2, weight: .medium, zoom: zoom))
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
                .padding(.leading, 2)
                .accessibilityLabel(Text("in \(project)"))
            }
        }
        .transition(.opacity)
    }

    /// "Summoning hands" while the briefs still stream; once the block has closed, how many
    /// are going (or went) out, the one hand by name when the lead named it.
    private static func title(for delegations: [CrewDelegation], streaming: Bool, sent: Bool) -> String {
        if streaming || delegations.isEmpty { return "Summoning hands" }
        let verb = sent ? "Summoned" : "Summoning"
        if delegations.count == 1 {
            if let persona = persona(named: delegations[0].name) { return "\(verb) \(persona.name)" }
            return "\(verb) a hand"
        }
        return "\(verb) \(delegations.count) hands"
    }

    /// The roster persona the lead announced for an entry, when it is a roster name.
    private static func persona(named name: String?) -> CrewPersona? {
        guard let name, !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return CrewRoster.persona(named: name)
    }

    /// A project the brief names, as its name: a path reads as its last folder.
    private static func projectName(_ project: String?) -> String? {
        guard let project = project?.trimmingCharacters(in: .whitespacesAndNewlines), !project.isEmpty else { return nil }
        let name = project.contains("/") ? (project as NSString).lastPathComponent : project
        return name.isEmpty ? project : name
    }

    /// The block read as far as it has streamed, by source: the reader is a single linear
    /// pass over the body and reads partial blocks on purpose, so every flush is cached
    /// and a finished block is the same card on every evaluation.
    @MainActor private static var readingCache = RecentCache<String, CrewPrompts.StreamedDelegations>(limit: 200)

    /// The entries whose briefs have closed so far, in the block's order, and whether the
    /// block itself has closed. The card reads the block as it streams and lists each
    /// hand as its brief closes; nothing until the first entry's brace has closed.
    @MainActor
    private static func reading(of json: String) -> CrewPrompts.StreamedDelegations {
        if let cached = readingCache.value(for: json) { return cached }
        let reading = CrewPrompts.streamedDelegations(inBody: json[...])
        readingCache.insert(reading, for: json)
        return reading
    }
}
