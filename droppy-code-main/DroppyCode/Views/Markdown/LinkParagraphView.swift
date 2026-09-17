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

    /// Receives the text view, so the hover that sets the cursor can ask it what is under
    /// the pointer. The closure lives as long as the representable: capture only a weak
    /// box in it, never the paragraph's view value, or the text view keeps its row alive.
    var onHost: ((LinkTextView) -> Void)?

    func makeNSView(context: Context) -> LinkTextView {
        let view = LinkTextView()
        view.delegate = context.coordinator
        view.mergeTarget = context.environment.mergeRequestTarget
        view.passesProseThrough = !context.environment.markdownBlockSelection
        onHost?(view)
        return view
    }

    func updateNSView(_ view: LinkTextView, context: Context) {
        view.passesProseThrough = !context.environment.markdownBlockSelection
        // Read so a favicon-load bump rebuilds the string with icons.
        _ = revision
        let coordinator = context.coordinator
        if coordinator.lastSource != source || coordinator.lastPointSize != pointSize
            || coordinator.lastDimmed != dimmed || coordinator.lastStreaming != streaming
            || coordinator.lastRevision != revision {
            coordinator.lastSource = source
            coordinator.lastPointSize = pointSize
            coordinator.lastDimmed = dimmed
            coordinator.lastStreaming = streaming
            coordinator.lastRevision = revision
            view.render(Self.attributed(source: source, pointSize: pointSize, dimmed: dimmed, streaming: streaming))
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
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        // The last inputs built into a string, so repeat updates with nothing new skip it.
        var lastSource: String?
        var lastPointSize: CGFloat = 0
        var lastDimmed = false
        var lastStreaming = false
        var lastRevision: Int = 0
        func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
            if let url = link as? URL {
                NSWorkspace.shared.open(url)
                return true
            }
            if let string = link as? String, let url = URL(string: string) {
                NSWorkspace.shared.open(url)
                return true
            }
            return false
        }
    }

    /// Same pretty string the `Text` path renders (short titles, bold
    /// links), rebuilt on AppKit types with an explicit base font, adaptive
    /// colors and favicon attachments.
    /// The four faces per point size, looked up once: the italic conversion goes through
    /// the font manager, and a streaming link paragraph rebuilds its string every flush.
    @MainActor private static var fonts: [CGFloat: (base: NSFont, bold: NSFont, italic: NSFont, mono: NSFont)] = [:]

    @MainActor
    static func attributed(source: String, pointSize: CGFloat, dimmed: Bool, streaming: Bool = false) -> NSAttributedString {
        let pretty = RichLink.prettyAttributed(source, streaming: streaming)
        let (base, bold, italic, mono) = fonts[pointSize] ?? {
            let base = NSFont.systemFont(ofSize: pointSize)
            let set = (base, NSFont.boldSystemFont(ofSize: pointSize),
                       NSFontManager.shared.convert(base, toHaveTrait: .italicFontMask),
                       NSFont.monospacedSystemFont(ofSize: max(9, pointSize - 1), weight: .regular))
            fonts[pointSize] = set
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
                out.append(NSAttributedString(string: string, attributes: [
                    .font: font,
                    .foregroundColor: textColor,
                ]))
            }
        }
        return out
    }
}

/// What a merge or pull request link in a chat hands its request to, set by a chat that can
/// spawn a merge helper; a link then offers "Merge" at the top of its menu. Compared by the
/// chat it stands for, so a chat setting it on every render never re-renders a link paragraph.
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

    /// Whether `point`, in this view's coordinates, is over a link.
    func hasLink(at point: CGPoint) -> Bool {
        link(at: point) != nil
    }

    /// The link under `point`, in this view's coordinates.
    func link(at point: CGPoint) -> URL? {
        linkAndRange(at: point)?.url
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
        return linkAndRange(at: local) == nil ? nil : super.hitTest(point)
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
