// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct ProseRunView: View {
    let blocks: [MarkdownBlock]
    @Environment(\.markdownPointSize) private var pointSize
    @Environment(\.chatZoom) private var zoom
    @Environment(\.chatTypefaces) private var typefaces
    @Environment(\.markdownDimmed) private var dimmed
    @Environment(\.crewMentionPersonas) private var crewMentionPersonas
    @Environment(\.crewMentionTargets) private var crewMentionTargets
    @Environment(\.openURL) private var openURL
    @State private var linkView = WeakView()
    @State private var showsHand = false
    @State private var hoveredHandID: UUID?
    @State private var faviconRevision = 0

    var body: some View {
        let scaled = (pointSize * zoom * 2).rounded() / 2
        let sources = ProseRunRepresentable.joinedSources(blocks)
        let linkKey = RichLink.linkAssetKey(for: sources)
        let linkBox = linkView
        let hand = $showsHand
        let hoveredHand = $hoveredHandID
        let personasByName = Dictionary(crewMentionPersonas.map { ($0.name, $0) }, uniquingKeysWith: { first, _ in first })
        let targets = crewMentionTargets
        ProseRunRepresentable(blocks: blocks, pointSize: scaled, dimmed: dimmed, revision: faviconRevision, mentions: crewMentionPersonas, targets: crewMentionTargets, typefaces: typefaces, onOpenHand: { url in openURL(url) }, onOpenLink: { url in openURL(url) }, onHost: { [weak linkBox] view in linkBox?.value = view })
            .frame(maxWidth: .infinity, alignment: .leading)
            .onContinuousHover(coordinateSpace: .local) { [weak linkBox, hand, hoveredHand, personasByName, targets] phase in
                guard let view = linkBox?.value as? LinkTextView else { return }
                switch phase {
                case .active(let point):
                    let overLink = view.hasLink(at: point)
                    if overLink != hand.wrappedValue {
                        hand.wrappedValue = overLink
                        if overLink { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                    }
                    if let (name, url, rect) = view.handMentionRect(at: point),
                       let target = targets[name], target.url == url,
                       let persona = personasByName[name] {
                        if hoveredHand.wrappedValue != target.threadID {
                            hoveredHand.wrappedValue = target.threadID
                            CrewHandHoverPopover.shared.show(persona: persona, target: target, relativeTo: rect, of: view)
                        }
                    } else {
                        if hoveredHand.wrappedValue != nil {
                            hoveredHand.wrappedValue = nil
                            CrewHandHoverPopover.shared.hide()
                        }
                    }
                case .ended:
                    if hand.wrappedValue { NSCursor.pop(); hand.wrappedValue = false }
                    hoveredHand.wrappedValue = nil
                    CrewHandHoverPopover.shared.hide()
                }
            }
            .onDisappear { [hand, hoveredHand] in
                if hand.wrappedValue { NSCursor.pop(); hand.wrappedValue = false }
                hoveredHand.wrappedValue = nil
                CrewHandHoverPopover.shared.hide()
            }
            .task(id: linkKey) { [sources, linkKey, revision = $faviconRevision] in
                if !linkKey.isEmpty {
                    await InlineText.fetchLinkAssets(source: sources, streaming: false, revision: revision)
                }
            }
    }
}

struct ProseRunRepresentable: NSViewRepresentable {
    let blocks: [MarkdownBlock]
    var pointSize: CGFloat = 13
    var dimmed: Bool = false
    var revision: Int = 0
    var mentions: [CrewPersona] = []
    var targets: [String: CrewMentionTarget] = [:]
    /// The faces the prose and its inline code are set in (see `ChatTypefaces`).
    var typefaces = ChatTypefaces()
    var onOpenHand: ((URL) -> Void)?
    /// Receives a clicked link (see `LinkParagraphView.onOpenLink`).
    var onOpenLink: ((URL) -> Void)?
    var onHost: ((LinkTextView) -> Void)?

    func makeCoordinator() -> LinkParagraphView.Coordinator {
        LinkParagraphView.Coordinator()
    }

    func makeNSView(context: Context) -> LinkTextView {
        let view = LinkTextView()
        view.delegate = context.coordinator
        context.coordinator.onOpenLink = onOpenLink
        view.passesProseThrough = false
        onHost?(view)
        view.mergeTarget = context.environment.mergeRequestTarget
        return view
    }

    func updateNSView(_ view: LinkTextView, context: Context) {
        _ = revision
        let coordinator = context.coordinator
        let source = Self.joinedSources(blocks)
        let mentionNames = mentions.map(\.name)
        let handLinks = targets.mapValues(\.url)
        if coordinator.lastSource != source || coordinator.lastPointSize != pointSize
            || coordinator.lastDimmed != dimmed || coordinator.lastRevision != revision
            || coordinator.lastMentions != mentionNames || coordinator.lastHands != handLinks
            || coordinator.lastTypefaces != typefaces {
            coordinator.lastSource = source
            coordinator.lastPointSize = pointSize
            coordinator.lastDimmed = dimmed
            coordinator.lastRevision = revision
            coordinator.lastMentions = mentionNames
            coordinator.lastHands = handLinks
            coordinator.lastTypefaces = typefaces
            view.render(Self.attributed(blocks: blocks, pointSize: pointSize, dimmed: dimmed, mentions: mentions, targets: targets, typefaces: typefaces))
        }
        // Outside the render check: the click handlers are the environment's, and a new
        // one must land even on an update that drew nothing.
        view.onOpenHand = onOpenHand
        context.coordinator.onOpenLink = onOpenLink
        view.mergeTarget = context.environment.mergeRequestTarget
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: LinkTextView, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        return CGSize(width: width, height: nsView.height(forWidth: width))
    }

    static func dismantleNSView(_ view: LinkTextView, coordinator: LinkParagraphView.Coordinator) {
        view.delegate = nil
        view.onOpenHand = nil
        coordinator.onOpenLink = nil
        view.mergeTarget = nil
    }

    static func joinedSources(_ blocks: [MarkdownBlock]) -> String {
        var parts: [String] = []
        collect(blocks, into: &parts)
        return parts.joined(separator: "\n")
    }

    private static func collect(_ blocks: [MarkdownBlock], into parts: inout [String]) {
        for block in blocks {
            switch block {
            case .paragraph(let text), .heading(_, let text):
                parts.append(text)
            case .list(_, _, let items):
                for item in items {
                    parts.append(item.text)
                    collect(item.children, into: &parts)
                }
            case .quote(let inner):
                collect(inner, into: &parts)
            case .code, .table, .rule:
                break
            }
        }
    }

    @MainActor
    static func attributed(blocks: [MarkdownBlock], pointSize: CGFloat, dimmed: Bool, mentions: [CrewPersona], targets: [String: CrewMentionTarget] = [:], typefaces: ChatTypefaces = ChatTypefaces()) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for block in blocks {
            append(block, depth: 0, indent: 0, dimmed: dimmed, spacing: 12, pointSize: pointSize, mentions: mentions, targets: targets, typefaces: typefaces, to: out)
        }
        return out
    }

    @MainActor
    private static func append(_ block: MarkdownBlock, depth: Int, indent: CGFloat, dimmed: Bool, spacing: CGFloat, pointSize: CGFloat, mentions: [CrewPersona], targets: [String: CrewMentionTarget], typefaces: ChatTypefaces, to out: NSMutableAttributedString) {
        let base = Typefaces.proseFont(family: typefaces.text, size: pointSize)
        let color = dimmed ? NSColor.secondaryLabelColor : NSColor.labelColor
        switch block {
        case .paragraph(let text):
            separate(to: out, font: base, color: color)
            let line = NSMutableAttributedString(attributedString: LinkParagraphView.attributed(source: text, pointSize: pointSize, dimmed: dimmed, mentions: mentions, targets: targets, typefaces: typefaces))
            let style = NSMutableParagraphStyle()
            style.firstLineHeadIndent = indent
            style.headIndent = indent
            style.paragraphSpacing = spacing
            line.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: line.length))
            out.append(line)
        case .heading(let level, let text):
            separate(to: out, font: base, color: color)
            let size: CGFloat
            switch level {
            case 1: size = (pointSize * 1.35).rounded()
            case 2: size = (pointSize * 1.2).rounded()
            case 3: size = (pointSize * 1.08).rounded()
            default: size = pointSize
            }
            let line = NSMutableAttributedString(attributedString: LinkParagraphView.attributed(source: text, pointSize: size, dimmed: dimmed, mentions: mentions, targets: targets, typefaces: typefaces))
            let semibold = Typefaces.proseFont(family: typefaces.text, size: size, weight: .semibold)
            line.enumerateAttribute(.font, in: NSRange(location: 0, length: line.length), options: []) { value, range, _ in
                guard let font = value as? NSFont else { return }
                let traits = font.fontDescriptor.symbolicTraits
                if !traits.contains(.bold) && !traits.contains(.monoSpace) {
                    line.addAttribute(.font, value: semibold, range: range)
                }
            }
            let style = NSMutableParagraphStyle()
            style.firstLineHeadIndent = indent
            style.headIndent = indent
            style.paragraphSpacingBefore = level <= 2 ? 6 : 2
            style.paragraphSpacing = 12
            line.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: line.length))
            out.append(line)
        case .list(let ordered, let start, let items):
            for (index, item) in items.enumerated() {
                separate(to: out, font: base, color: color)
                let marker: String
                if let checked = item.checked {
                    marker = checked ? "☑" : "☐"
                } else if ordered {
                    marker = "\(start + index)."
                } else {
                    marker = depth == 0 ? "•" : "◦"
                }
                let line = NSMutableAttributedString(string: marker + "\t", attributes: [
                    .font: base,
                    .foregroundColor: color,
                ])
                if !item.text.isEmpty {
                    line.append(LinkParagraphView.attributed(source: item.text, pointSize: pointSize, dimmed: dimmed, mentions: mentions, targets: targets, typefaces: typefaces))
                }
                let style = NSMutableParagraphStyle()
                style.firstLineHeadIndent = indent
                style.headIndent = indent + 18
                style.tabStops = [NSTextTab(textAlignment: .left, location: indent + 18)]
                style.defaultTabInterval = 18
                style.paragraphSpacing = (depth == 0 && index == items.count - 1) ? 12 : 6
                line.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: line.length))
                out.append(line)
                for child in item.children {
                    append(child, depth: depth + 1, indent: indent + 18, dimmed: dimmed, spacing: 6, pointSize: pointSize, mentions: mentions, targets: targets, typefaces: typefaces, to: out)
                }
            }
        case .quote(let inner):
            for child in inner {
                append(child, depth: depth, indent: indent + 14, dimmed: true, spacing: spacing, pointSize: pointSize, mentions: mentions, targets: targets, typefaces: typefaces, to: out)
            }
        case .code, .table, .rule:
            break
        }
    }

    @MainActor
    private static func separate(to out: NSMutableAttributedString, font: NSFont, color: NSColor) {
        guard out.length > 0 else { return }
        out.append(NSAttributedString(string: "\n", attributes: [
            .font: font,
            .foregroundColor: color,
        ]))
    }
}
