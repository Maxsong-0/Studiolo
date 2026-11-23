// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// Selectable paragraph with real links. SwiftUI `Text` with selection shows
/// the I-beam cursor over links too; this NSTextView shows the native
/// pointing hand over links and the I-beam over prose, with click-to-open,
/// exactly like every other Mac app. Plain paragraphs without links keep
/// using `InlineText`'s `Text` path, so only link paragraphs pay for this.
struct LinkParagraphView: NSViewRepresentable {
    let source: String
    var pointSize: CGFloat = 13
    var dimmed: Bool = false
    /// Whether the text is still arriving, so its parses bypass the shared caches.
    var streaming: Bool = false
    /// Bumped when favicons finish loading, so the icons appear.
    var revision: Int = 0
    var mentions: [CrewPersona] = []
    var targets: [String: CrewMentionTarget] = [:]
    /// The faces the prose and its inline code are set in (see `ChatTypefaces`).
    var typefaces = ChatTypefaces()
    var onOpenHand: ((URL) -> Void)?
    /// Receives a clicked link: the view's `openURL` action, so a page showing the text
    /// can take links of its own scheme and every other goes out as it always did.
    var onOpenLink: ((URL) -> Void)?

    /// Receives the text view, so the hover that sets the cursor can ask it what is under
    /// the pointer. The closure lives as long as the representable: capture only a weak
    /// box in it, never the paragraph's view value, or the text view keeps its row alive.
    var onHost: ((LinkTextView) -> Void)?

    func makeNSView(context: Context) -> LinkTextView {
        let view = LinkTextView()
        view.delegate = context.coordinator
        context.coordinator.onOpenLink = onOpenLink
        view.mergeTarget = context.environment.mergeRequestTarget
        view.passesProseThrough = !context.environment.markdownBlockSelection
        view.onOpenHand = onOpenHand
        onHost?(view)
        return view
    }

    func updateNSView(_ view: LinkTextView, context: Context) {
        view.passesProseThrough = !context.environment.markdownBlockSelection
        view.onOpenHand = onOpenHand
        context.coordinator.onOpenLink = onOpenLink
        // Read so a favicon-load bump rebuilds the string with icons.
        _ = revision
        let coordinator = context.coordinator
        let veiled = context.environment.markdownVeiled
        if coordinator.lastSource != source || coordinator.lastPointSize != pointSize
            || coordinator.lastDimmed != dimmed || coordinator.lastStreaming != streaming
            || coordinator.lastRevision != revision || coordinator.lastVeiled != veiled
            || coordinator.lastMentions != mentions.map(\.name) || coordinator.lastTargets != targets
            || coordinator.lastTypefaces != typefaces {
            coordinator.lastSource = source
            coordinator.lastPointSize = pointSize
            coordinator.lastDimmed = dimmed
            coordinator.lastStreaming = streaming
            coordinator.lastRevision = revision
            coordinator.lastVeiled = veiled
            coordinator.lastMentions = mentions.map(\.name)
            coordinator.lastTargets = targets
            coordinator.lastTypefaces = typefaces
            view.render(Self.attributed(source: source, pointSize: pointSize, dimmed: dimmed, streaming: streaming, mentions: mentions, targets: targets, typefaces: typefaces))
            // After the text is in place: the veil reads what the storage holds now.
            view.streamVeil.update(view, active: veiled)
        }
        view.mergeTarget = context.environment.mergeRequestTarget
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: LinkTextView, context: Context) -> CGSize? {
        guard let width = proposal.width else { return nil }
        return CGSize(width: width, height: nsView.height(forWidth: width))
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    /// The text view otherwise outlives its row through its menu and its merge
    /// hand-off, which holds the thread's runtime: drop both when it goes away.
    static func dismantleNSView(_ view: LinkTextView, coordinator: Coordinator) {
        view.delegate = nil
        view.mergeTarget = nil
        view.streamVeil.detach(view)
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        // The last inputs built into a string, so repeat updates with nothing new skip it.
        var lastSource: String?
        var lastPointSize: CGFloat = 0
        var lastDimmed = false
        var lastStreaming = false
        var lastRevision: Int = 0
        var lastVeiled = false
        var lastMentions: [String] = []
        var lastTargets: [String: CrewMentionTarget] = [:]
        var lastHands: [String: URL] = [:]
        var lastTypefaces = ChatTypefaces()
        /// The view's `openURL` action (see `LinkParagraphView.onOpenLink`); without one
        /// the link opens the way the system would.
        var onOpenLink: ((URL) -> Void)?

        func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
            let url: URL?
            if let value = link as? URL { url = value } else if let string = link as? String { url = URL(string: string) } else { url = nil }
            guard let url else { return false }
            if let onOpenLink { onOpenLink(url) } else { NSWorkspace.shared.open(url) }
            return true
        }
    }

    /// Same pretty string the `Text` path renders (short titles, bold
    /// links), rebuilt on AppKit types with an explicit base font, adaptive
    /// colors and favicon attachments.
    /// The four faces per point size and pair of families, looked up once: the italic
    /// conversion goes through the font manager, and a streaming link paragraph rebuilds
    /// its string every flush.
    @MainActor private static var fonts: [FontSetKey: (base: NSFont, bold: NSFont, italic: NSFont, mono: NSFont)] = [:]

    private struct FontSetKey: Hashable {
        let pointSize: CGFloat
        let typefaces: ChatTypefaces
    }

    @MainActor
    static func attributed(source: String, pointSize: CGFloat, dimmed: Bool, streaming: Bool = false, mentions: [CrewPersona] = [], targets: [String: CrewMentionTarget] = [:], typefaces: ChatTypefaces = ChatTypefaces()) -> NSAttributedString {
        let pretty = RichLink.prettyAttributed(source, streaming: streaming)
        let key = FontSetKey(pointSize: pointSize, typefaces: typefaces)
        let (base, bold, italic, mono) = fonts[key] ?? {
            // Without a family each face is the system one the prose was always set in.
            let set = (Typefaces.proseFont(family: typefaces.text, size: pointSize),
                       Typefaces.proseFont(family: typefaces.text, size: pointSize, weight: .bold),
                       Typefaces.proseItalicFont(family: typefaces.text, size: pointSize),
                       Typefaces.codeFont(family: typefaces.code, size: max(9, pointSize - 1)))
            fonts[key] = set
            return set
        }()
        let textColor = dimmed ? NSColor.secondaryLabelColor : NSColor.labelColor
        let out = NSMutableAttributedString()
        for run in pretty.runs {
            let string = String(pretty[run.range].characters)
            guard !string.isEmpty else { continue }
            var font = base
            let intent = run.inlinePresentationIntent ?? []
            if intent.contains(.code) {
                font = mono
            } else {
                if intent.contains(.emphasized) { font = italic }
                if intent.contains(.stronglyEmphasized) { font = bold }
            }
            if let url = run.link {
                if let host = url.host?.lowercased(),
                   let icon = FaviconCache.cached(host: host) {
                    let attachment = NSTextAttachment()
                    attachment.image = icon
                    attachment.bounds = NSRect(x: 0, y: base.descender + 1, width: 14, height: 14)
                    out.append(NSAttributedString(attachment: attachment))
                    out.append(NSAttributedString(string: " ", attributes: [
                        .font: base,
                        .foregroundColor: textColor,
                    ]))
                }
                // Colour and underline come from the view's `linkTextAttributes`.
                out.append(NSAttributedString(string: string, attributes: [
                    .font: font,
                    .link: url,
                ]))
            } else {
                appendProse(string, font: font, color: textColor, base: base, bold: bold, mentions: mentions, targets: targets, into: out)
            }
        }
        return out
    }

    @MainActor private static var dragonCache: [String: NSImage] = [:]

    private static func nsColor(_ persona: CrewPersona) -> NSColor {
        NSColor(red: Double((persona.hex >> 16) & 0xFF) / 255, green: Double((persona.hex >> 8) & 0xFF) / 255, blue: Double(persona.hex & 0xFF) / 255, alpha: 1)
    }

    @MainActor private static func dragonImage(_ persona: CrewPersona, size: CGFloat) -> NSImage {
        let key = persona.asset + "@" + String(format: "%06X", persona.hex) + "@\(size)"
        if let cached = dragonCache[key] { return cached }
        let tint = nsColor(persona)
        let image = NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            let source = NSImage(named: persona.asset)
            source?.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
            tint.set()
            rect.fill(using: .sourceAtop)
            return true
        }
        dragonCache[key] = image
        return image
    }

    @MainActor private static func appendProse(_ string: String, font: NSFont, color: NSColor, base: NSFont, bold: NSFont, mentions: [CrewPersona], targets: [String: CrewMentionTarget], into out: NSMutableAttributedString) {
        if mentions.isEmpty {
            out.append(NSAttributedString(string: string, attributes: [
                .font: font,
                .foregroundColor: color,
            ]))
            return
        }
        let sorted = mentions.sorted { $0.name.count > $1.name.count }
        let glyphSize = (base.pointSize * 1.25).rounded()
        var i = string.startIndex
        var runStart = i
        func flush(from: String.Index, to: String.Index) {
            if from < to {
                out.append(NSAttributedString(string: String(string[from ..< to]), attributes: [
                    .font: font,
                    .foregroundColor: color,
                ]))
            }
        }
        while i < string.endIndex {
            var hit: CrewPersona?
            for persona in sorted where string[i...].hasPrefix(persona.name) {
                let beforeOK: Bool = {
                    guard i > string.startIndex else { return true }
                    let c = string[string.index(before: i)]
                    return !c.isLetter && !c.isNumber
                }()
                let after = string.index(i, offsetBy: persona.name.count)
                let afterOK: Bool = {
                    guard after < string.endIndex else { return true }
                    let c = string[after]
                    return !c.isLetter && !c.isNumber
                }()
                if beforeOK && afterOK { hit = persona; break }
            }
            guard let persona = hit else {
                i = string.index(after: i)
                continue
            }
            flush(from: runStart, to: i)
            let attachment = NSTextAttachment()
            attachment.image = dragonImage(persona, size: glyphSize)
            attachment.bounds = NSRect(x: 0, y: base.descender + 1, width: glyphSize, height: glyphSize)
            if let target = targets[persona.name] {
                // Custom attribute, not .link: linkTextAttributes would recolour the name in the accent.
                // No pointing hand either: a hand chip keeps the normal arrow cursor. Its hover
                // popover and its click still work; the hand is what a real link wears.
                let handAttributes: [NSAttributedString.Key: Any] = [.crewHand: target.url]
                let attach = NSMutableAttributedString(attachment: attachment)
                attach.addAttributes(handAttributes, range: NSRange(location: 0, length: attach.length))
                out.append(attach)
            } else {
                out.append(NSAttributedString(attachment: attachment))
            }
            out.append(NSAttributedString(string: " ", attributes: [.font: base, .foregroundColor: color]))
            if let target = targets[persona.name] {
                out.append(NSAttributedString(string: persona.name, attributes: [.font: bold, .foregroundColor: nsColor(persona), .crewHand: target.url]))
            } else {
                out.append(NSAttributedString(string: persona.name, attributes: [.font: bold, .foregroundColor: nsColor(persona)]))
            }
            i = string.index(i, offsetBy: persona.name.count)
            runStart = i
        }
        flush(from: runStart, to: string.endIndex)
    }
}

extension NSAttributedString.Key {
    static let crewHand = NSAttributedString.Key("bottega.crewHand")
}

/// What a merge or pull request link in a chat hands its request to, set by a chat that can
/// spawn a merge helper; a link then offers "Merge" at the top of its menu. Compared by the
struct MergeRequestTarget: Equatable {
    let chatID: UUID
    let merge: @MainActor (MergeRequestLink) -> Void

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.chatID == rhs.chatID
    }
}

private struct MergeRequestTargetKey: EnvironmentKey {
    static let defaultValue: MergeRequestTarget? = nil
}

extension EnvironmentValues {
    var mergeRequestTarget: MergeRequestTarget? {
        get { self[MergeRequestTargetKey.self] }
        set { self[MergeRequestTargetKey.self] = newValue }
    }
}

/// Non-editable selectable text view that sizes itself to its content width.
/// No continuous work: layout runs once per text or width change.
final class LinkTextView: NSTextView {
    private var lastLaidOutWidth: CGFloat = 0
    private var lastMeasuredHeight: CGFloat = 0

    /// Receives a merge or pull request link when its popover's Merge row is chosen. Nil
    /// leaves Merge off every link's popover.
    var mergeTarget: MergeRequestTarget?

    /// Inside a finished reply the prose passes the mouse through to SwiftUI, whose drag
    /// swaps in the whole-reply selection; links still take the click and open.
    var passesProseThrough = false

    var onOpenHand: ((URL) -> Void)?

    /// Fades streamed characters in over the laid-out text (see `StreamVeil`).
    let streamVeil = LinkTextVeil()

    /// The view owns its text storage. `init(frame:textContainer:)` only takes a
    /// container and holds it weakly; a bare container with no layout manager or
    /// storage behind it is released on the spot, leaving the view with no
    /// `textContainer` and no `textStorage`, so `render` had nothing to write into
    /// and every link paragraph came out blank.
    private let storage = NSTextStorage()

    init() {
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 400, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = false
        container.heightTracksTextView = false
        container.lineFragmentPadding = 0
        layoutManager.addTextContainer(container)
        super.init(frame: .zero, textContainer: container)
        // Vertical sizing is driven by `height(forWidth:)`; never clamp the text to
        // the zero frame this view starts with.
        minSize = .zero
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        isEditable = false
        isSelectable = true
        drawsBackground = false
        importsGraphics = true
        allowsUndo = false
        usesFindBar = false
        focusRingType = .none
        textContainerInset = .zero
        isVerticallyResizable = true
        isHorizontallyResizable = false
        autoresizingMask = [.width]
        isAutomaticLinkDetectionEnabled = false
        isAutomaticDataDetectionEnabled = false
        isAutomaticTextCompletionEnabled = false
        isAutomaticSpellingCorrectionEnabled = false
        isAutomaticQuoteSubstitutionEnabled = false
        isAutomaticDashSubstitutionEnabled = false
        isAutomaticTextReplacementEnabled = false
        isContinuousSpellCheckingEnabled = false
        isGrammarCheckingEnabled = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    // MARK: - No per-view tracking

    // A text view normally keeps a tracking area and cursor rects of its own, and AppKit
    // rebuilds every one of them on each frame the content scrolls, which a long thread
    // full of link paragraphs turned into a per-frame tax. These views keep none: the
    // paragraph's SwiftUI hover sets the pointing hand over a link instead (see
    // `InlineText`), and clicks still reach `clickedOnLink` through `mouseDown`.
    override func updateTrackingAreas() {
        for area in trackingAreas { removeTrackingArea(area) }
    }

    override func resetCursorRects() {}

    /// Whether `point`, in this view's coordinates, is over a link or a hand mention.
    /// Whether a real link sits under `point`, for the pointing hand. A hand mention is
    /// clickable too, but it keeps the normal arrow cursor: only links wear the hand.
    func hasLink(at point: CGPoint) -> Bool {
        link(at: point) != nil
    }

    /// The link under `point`, in this view's coordinates.
    func link(at point: CGPoint) -> URL? {
        linkAndRange(at: point)?.url
    }

    /// The hand mention under `point`, and the run of characters it covers.
    func handMention(at point: CGPoint) -> (url: URL, range: NSRange)? {
        guard let layoutManager, let textContainer, let textStorage, textStorage.length > 0 else { return nil }
        let glyph = layoutManager.glyphIndex(for: point, in: textContainer)
        guard glyph < layoutManager.numberOfGlyphs,
              layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer).contains(point) else { return nil }
        let index = layoutManager.characterIndexForGlyph(at: glyph)
        var range = NSRange(location: 0, length: 0)
        guard index < textStorage.length,
              let link = textStorage.attribute(.crewHand, at: index, longestEffectiveRange: &range, in: NSRange(location: 0, length: textStorage.length)) else { return nil }
        let url: URL?
        if let value = link as? URL { url = value } else if let string = link as? String { url = URL(string: string) } else { url = nil }
        guard let url else { return nil }
        return (url, range)
    }

    /// The persona name and the bounding rect of the hand run under `point`, for the popover anchor.
    func handMentionRect(at point: CGPoint) -> (name: String, url: URL, rect: NSRect)? {
        guard let (url, range) = handMention(at: point),
              let layoutManager, let textContainer, let textStorage else { return nil }
        // The glyph is an attachment run of its own, so a hover over it names nothing;
        // step past it (and the space after it) to the name run beside it.
        var nameRange = range
        // The attachment character is what the glyph run is made of; it is no name.
        func runName(_ range: NSRange) -> String {
            (textStorage.string as NSString).substring(with: range)
                .replacingOccurrences(of: "\u{FFFC}", with: "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        var name = runName(nameRange)
        if name.isEmpty {
            let full = NSRange(location: 0, length: textStorage.length)
            var index = range.location + range.length
            while index < textStorage.length,
                  textStorage.attribute(.crewHand, at: index, effectiveRange: nil) == nil {
                index += 1
            }
            guard index < textStorage.length else { return nil }
            var next = NSRange(location: 0, length: 0)
            guard textStorage.attribute(.crewHand, at: index, longestEffectiveRange: &next, in: full) != nil else { return nil }
            nameRange = next
            name = runName(nameRange)
            guard !name.isEmpty else { return nil }
        }
        let glyphs = layoutManager.glyphRange(forCharacterRange: nameRange, actualCharacterRange: nil)
        var rect = layoutManager.boundingRect(forGlyphRange: glyphs, in: textContainer)
        if rect.isEmpty { rect = NSRect(x: point.x, y: point.y, width: 1, height: 1) }
        return (name, url, rect)
    }

    /// The link under `point` and the run of characters it covers.
    private func linkAndRange(at point: CGPoint) -> (url: URL, range: NSRange)? {
        guard let layoutManager, let textContainer, let textStorage, textStorage.length > 0 else { return nil }
        let glyph = layoutManager.glyphIndex(for: point, in: textContainer)
        guard glyph < layoutManager.numberOfGlyphs,
              layoutManager.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: textContainer).contains(point) else { return nil }
        let index = layoutManager.characterIndexForGlyph(at: glyph)
        var range = NSRange(location: 0, length: 0)
        guard index < textStorage.length,
              let link = textStorage.attribute(.link, at: index, longestEffectiveRange: &range, in: NSRange(location: 0, length: textStorage.length)) else { return nil }
        let url: URL?
        if let value = link as? URL { url = value } else if let string = link as? String { url = URL(string: string) } else { url = nil }
        guard let url else { return nil }
        return (url, range)
    }

    // MARK: - Link popover

    /// A right-click on a link opens the link popover under the words, instead of the system's
    /// text menu; the popover puts Merge first on a merge or pull request link. Handled here
    /// rather than in `menu(for:)` so the click never moves the selection. Prose keeps the
    /// system's menu.
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard passesProseThrough, let superview else { return super.hitTest(point) }
        // `hitTest` gets the superview's coordinates; the link lookup wants this view's.
        let local = convert(point, from: superview)
        if linkAndRange(at: local) != nil || handMention(at: local) != nil { return super.hitTest(point) }
        return nil
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let (url, _) = handMention(at: point) {
            onOpenHand?(url)
            return
        }
        super.mouseDown(with: event)
    }

    override func rightMouseDown(with event: NSEvent) {
        guard !presentLinkPopover(for: event) else { return }
        super.rightMouseDown(with: event)
    }

    /// Control-click reaches the menu path; a link opens the popover there too.
    override func menu(for event: NSEvent) -> NSMenu? {
        presentLinkPopover(for: event) ? nil : super.menu(for: event)
    }

    private func presentLinkPopover(for event: NSEvent) -> Bool {
        let point = convert(event.locationInWindow, from: nil)
        guard let (url, range) = linkAndRange(at: point), let layoutManager, let textContainer else { return false }
        let glyphs = layoutManager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
        var rect = layoutManager.boundingRect(forGlyphRange: glyphs, in: textContainer)
        if rect.isEmpty { rect = NSRect(x: point.x, y: point.y, width: 1, height: 1) }
        LinkPopover.shared.show(url: url, mergeTarget: mergeTarget, relativeTo: rect, of: self)
        return true
    }

    /// The last measurement, so the several `sizeThatFits` calls one layout pass makes
    /// with the same width run the text layout once.
    private var measuredWidth: CGFloat = 0
    private var measuredHeight: CGFloat = 0
    private var needsSettleAfterResize = false

    /// New content; skips the layout pass when nothing changed (favicons and
    /// streaming rebuilds both funnel through here).
    func render(_ text: NSAttributedString) {
        // Links wear the theme's accent with no underline, like the rest of
        // the chrome; the system accent for System/Light/Dark. Set per render
        // so a theme change recolours links already on screen.
        linkTextAttributes = [
            .foregroundColor: Chrome.accentNSColor,
            .underlineStyle: 0,
            .cursor: NSCursor.pointingHand,
        ]
        if textStorage?.isEqual(to: text) != true {
            textStorage?.setAttributedString(text)
            lastMeasuredHeight = 0
            measuredWidth = 0
        }
        invalidateIntrinsicContentSize()
    }

    func height(forWidth width: CGFloat) -> CGFloat {
        guard width > 0, let layout = layoutManager, let container = textContainer else { return 0 }
        if width == measuredWidth { return measuredHeight }
        container.containerSize = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        layout.ensureLayout(for: container)
        let height = ceil(layout.usedRect(for: container).height)
        measuredWidth = width
        measuredHeight = height
        return height
    }

    override var intrinsicContentSize: NSSize {
        let width = bounds.width > 0 ? bounds.width : lastLaidOutWidth
        guard width > 0 else { return NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric) }
        return NSSize(width: NSView.noIntrinsicMetric, height: height(forWidth: width))
    }

    override func layout() {
        super.layout()
        let width = bounds.width
        // SwiftUI probes at infinity before settling on a width; make sure the text wraps
        // to the frame it was actually given, not the last probe.
        if width > 0, let container = textContainer, container.containerSize.width != width {
            container.containerSize = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        }
        // While the window is being dragged SwiftUI already sizes the view through
        // `sizeThatFits` for every width it proposes; the second measurement here only
        // landed a frame late and made the paragraph hop between two heights, so it
        // waits for the drag to end.
        if window?.inLiveResize == true {
            needsSettleAfterResize = true
            return
        }
        guard width > 0, abs(width - lastLaidOutWidth) > 0.5 else { return }
        lastLaidOutWidth = width
        let height = height(forWidth: width)
        guard abs(height - lastMeasuredHeight) > 0.5 else { return }
        lastMeasuredHeight = height
        invalidateIntrinsicContentSize()
    }

    override func viewDidEndLiveResize() {
        super.viewDidEndLiveResize()
        // The one measurement the drag skipped, now that the width is final.
        guard needsSettleAfterResize else { return }
        needsSettleAfterResize = false
        lastLaidOutWidth = bounds.width
        let height = height(forWidth: bounds.width)
        guard abs(height - lastMeasuredHeight) > 0.5 else { return }
        lastMeasuredHeight = height
        invalidateIntrinsicContentSize()
    }
}
