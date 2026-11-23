// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// A single-hand popover's header: the hand's glyph, its name with a status caption, and its task.
struct CrewPopoverHeader: View {
    let persona: CrewPersona
    var status: CrewHandInfo.Status? = nil
    var caption: String? = nil
    var captionColor: Color = Chrome.secondaryText
    var title: String? = nil

    /// Sits at the header's trailing edge: a copy button, a link.
    var accessory: AnyView? = nil

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            CrewGlyph(persona: persona, size: 28, status: status)
                .padding(.top, 1)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    CrewNameText(persona: persona, size: 14, weight: .semibold, color: Chrome.primaryText)
                    if let caption, !caption.isEmpty {
                        Text(caption)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(captionColor)
                            .monospacedDigit()
                            .lineLimit(1)
                    }
                }
                if let title, !title.isEmpty {
                    Text(title)
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.primaryText.opacity(0.85))
                        .lineLimit(3)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
            if let accessory {
                accessory
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        // A shade of its own, so the header reads as the panel's top and the report as its page.
        .background(Chrome.overlay(0.035))
    }
}

/// A finished hand's report in the popover its pill opens: the header stays put, the report scrolls under it.
struct CrewHandReportPopover: View {
    let hand: CrewHandInfo
    private let blocks: [MarkdownBlock]

    private static let width: CGFloat = 460

    @MainActor
    init(hand: CrewHandInfo) {
        self.hand = hand
        let summary = hand.summary?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        blocks = summary.isEmpty ? [] : MarkdownView.blocks(for: summary)
    }

    var body: some View {
        VStack(spacing: 0) {
            CrewPopoverHeader(
                persona: hand.persona,
                status: hand.status,
                caption: Self.caption(for: hand),
                captionColor: Self.captionColor(for: hand.status),
                title: hand.task,
                accessory: blocks.isEmpty ? nil : AnyView(CopyButton(text: hand.summary ?? ""))
            )
            Divider()
            // Every block, not a lazy stack: the body is laid out whole to size the panel
            // before it opens (see `PopoverScroll`), so a lazy stack only measured
            // differently once shown and the panel cut its header off.
            PopoverScroll(maxHeight: 460, width: Self.width) {
                Group {
                    if blocks.isEmpty {
                        Text("No report yet.")
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                    } else {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                                MarkdownBlockView(block: block)
                                    .equatable()
                            }
                        }
                        .font(.system(size: 12))
                        .environment(\.markdownPointSize, 12)
                        .environment(\.markdownDimmed, false)
                        .textSelection(.enabled)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(width: Self.width)
        .environment(\.chatZoom, 1)
    }

    static func caption(for hand: CrewHandInfo) -> String {
        let base: String = switch hand.status {
        case .completed: "Finished"
        case .failed: "Failed"
        case .stopped: "Stopped"
        case .running: "Under way"
        }
        guard hand.status != .running, let finishedAt = hand.finishedAt else { return base }
        return "\(base) · \(duration(from: hand.startedAt, to: finishedAt))"
    }

    static func captionColor(for status: CrewHandInfo.Status) -> Color {
        switch status {
        case .failed: Chrome.danger
        case .stopped: Chrome.warning
        default: Chrome.secondaryText
        }
    }

    static func duration(from: Date, to: Date) -> String {
        let s = max(0, Int(to.timeIntervalSince(from)))
        if s < 60 { return "\(s)s" }
        if s < 3600 { return "\(s / 60) min" }
        return "\(s / 3600) h \((s % 3600) / 60) min"
    }
}

/// The lead's brief to a hand in the popover its pill opens: the same header over the brief's markdown.
struct CrewBriefPopover: View {
    let persona: CrewPersona
    let task: String?
    let text: String
    private let blocks: [MarkdownBlock]

    private static let width: CGFloat = 520

    @MainActor
    init(persona: CrewPersona, task: String?, text: String) {
        self.persona = persona
        self.task = task
        self.text = text
        blocks = MarkdownView.blocks(for: text)
    }

    var body: some View {
        VStack(spacing: 0) {
            CrewPopoverHeader(persona: persona, caption: "Brief", title: task, accessory: AnyView(CopyButton(text: text)))
            Divider()
            PopoverScroll(maxHeight: 460, width: Self.width) {
                Group {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                            MarkdownBlockView(block: block)
                                .equatable()
                        }
                    }
                    .font(.system(size: 12))
                    .environment(\.markdownPointSize, 12)
                    .environment(\.markdownDimmed, false)
                    .textSelection(.enabled)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(width: Self.width)
        .environment(\.chatZoom, 1)
    }
}
