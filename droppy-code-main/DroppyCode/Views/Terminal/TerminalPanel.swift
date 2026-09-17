import AppKit
import SwiftUI

struct TerminalPanel: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    let runtime: ThreadRuntime
    let directory: String

    @State private var dragOrigin: Double?
    /// The height while the edge is being dragged. Settings take it on release, so a drag
    /// never writes user defaults on every frame.
    @State private var dragHeight: Double?

    var body: some View {
        let terminals = model.terminals
        let sessions = terminals.sessions(for: runtime.threadID)
        let selected = terminals.selected(for: runtime.threadID)
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 4) {
                        ForEach(sessions) { session in
                            TerminalTab(
                                session: session,
                                isSelected: session.id == selected?.id,
                                select: { terminals.selection[runtime.threadID] = session.id },
                                restart: { terminals.restart(session) },
                                close: { terminals.close(session) }
                            )
                        }
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                Button {
                    terminals.open(threadID: runtime.threadID, directory: directory)
                } label: {
                    Image(systemName: "plus")
                        .font(Chrome.iconFont)
                }
                .buttonStyle(.chip)
                .help("New terminal")
                .accessibilityLabel(Text("New terminal"))
                Spacer(minLength: 0)
                Button {
                    runtime.isTerminalVisible = false
                } label: {
                    Image(systemName: "chevron.down")
                        .font(Chrome.iconFont)
                }
                .buttonStyle(.chip)
                .help("Hide terminal\(ShortcutStore.hint(for: .toggleTerminal))")
                .accessibilityLabel(Text("Hide terminal"))
            }
            .padding(.horizontal, 10)
            .frame(height: 38)

            if let selected {
                TerminalHost(session: selected, isDark: colorScheme == .dark)
                    .id(selected.id)
                    .padding(.leading, 12)
                    .padding(.trailing, 6)
                    .padding(.bottom, 6)
            } else {
                // Every tab closed leaves the drawer open with nothing in it; the way
                // back is here rather than only on the plus in the corner.
                VStack(spacing: 10) {
                    Text("No shell open in this thread")
                        .font(.system(size: 13))
                        .foregroundStyle(Chrome.secondaryText)
                    Button("New shell") {
                        terminals.open(threadID: runtime.threadID, directory: directory)
                    }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(height: dragHeight ?? model.settings.terminalHeight)
        // The drag lives on a strip of its own at the very top edge, overlaid on the empty
        // band above the tabs: the whole tab row used to resize, so a sideways drag across
        // the tabs moved the drawer instead of scrolling them.
        .overlay(alignment: .top) {
            TerminalResizeHandle(
                onBegin: { dragOrigin = model.settings.terminalHeight; runtime.isTerminalResizing = true },
                onChange: { delta in
                    let origin = dragOrigin ?? model.settings.terminalHeight
                    dragHeight = min(760, max(140, origin + delta))
                },
                onEnd: {
                    if let dragHeight { model.settings.terminalHeight = dragHeight }
                    dragHeight = nil
                    dragOrigin = nil
                    runtime.isTerminalResizing = false
                }
            )
            .frame(height: 8)
        }
        // Rounded top corners, so the panel reads as a sheet rising into the conversation.
        .background(
            UnevenRoundedRectangle(
                topLeadingRadius: Chrome.sheetCornerRadius,
                topTrailingRadius: Chrome.sheetCornerRadius,
                style: .continuous
            )
            .fill(colorScheme == .dark ? Color(white: 0.08) : Color(white: 0.985))
        )
        .onAppear { terminals.ensureTerminal(threadID: runtime.threadID, directory: directory) }
    }

}

/// The drawer's resize grip: a thin strip along its top edge with an up/down cursor and a
/// faint centred bar that grows on hover, the same affordance the sidebar's edge has
/// (see `SidebarResizeHandleView`). AppKit rather than a SwiftUI gesture, because only a
/// cursor rect can tell the pointer what the strip does before it is pressed.
private struct TerminalResizeHandle: NSViewRepresentable {
    let onBegin: () -> Void
    /// How far the pointer has travelled since the press, positive upward, which is the
    /// direction the drawer grows.
    let onChange: (CGFloat) -> Void
    let onEnd: () -> Void

    func makeNSView(context: Context) -> TerminalResizeHandleView {
        let view = TerminalResizeHandleView()
        apply(to: view)
        return view
    }

    func updateNSView(_ nsView: TerminalResizeHandleView, context: Context) {
        apply(to: nsView)
    }

    private func apply(to view: TerminalResizeHandleView) {
        view.onBegin = onBegin
        view.onChange = onChange
        view.onEnd = onEnd
    }
}

final class TerminalResizeHandleView: NSView {
    var onBegin: (() -> Void)?
    var onChange: ((CGFloat) -> Void)?
    var onEnd: (() -> Void)?

    private enum Grip {
        static let height: CGFloat = 2.5
        static let restWidth: CGFloat = 12
        static let shownWidth: CGFloat = 28
        static let alpha: Float = 0.6
        static let duration: CFTimeInterval = 0.18
    }

    private let gripLayer = CALayer()
    private var trackingArea: NSTrackingArea?
    private var pressOriginY: CGFloat = 0
    private var isHovering = false
    private var isDragging = false

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        gripLayer.cornerRadius = Grip.height / 2
        gripLayer.opacity = 0
        applyColors()
        layer?.addSublayer(gripLayer)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    private func applyColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            gripLayer.backgroundColor = NSColor.labelColor.cgColor
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyColors()
    }

    override func layout() {
        super.layout()
        updateGrip(animated: false)
    }

    private var showsGrip: Bool { isHovering || isDragging }

    private func updateGrip(animated: Bool) {
        let width = showsGrip ? Grip.shownWidth : Grip.restWidth
        CATransaction.begin()
        if animated {
            CATransaction.setAnimationDuration(Grip.duration)
            CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeOut))
        } else {
            CATransaction.setDisableActions(true)
        }
        gripLayer.frame = CGRect(
            x: (bounds.width - width) / 2,
            y: (bounds.height - Grip.height) / 2,
            width: width,
            height: Grip.height
        )
        gripLayer.opacity = showsGrip ? Grip.alpha : 0
        CATransaction.commit()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override var mouseDownCanMoveWindow: Bool { false }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .resizeUpDown)
    }

    override func mouseDown(with event: NSEvent) {
        pressOriginY = event.locationInWindow.y
        isDragging = true
        updateGrip(animated: true)
        onBegin?()
    }

    override func mouseDragged(with event: NSEvent) {
        onChange?(event.locationInWindow.y - pressOriginY)
    }

    override func mouseUp(with event: NSEvent) {
        onChange?(event.locationInWindow.y - pressOriginY)
        isDragging = false
        updateGrip(animated: true)
        onEnd?()
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let trackingArea { removeTrackingArea(trackingArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        isHovering = true
        updateGrip(animated: true)
    }

    override func mouseExited(with event: NSEvent) {
        isHovering = false
        updateGrip(animated: true)
    }
}

private struct TerminalTab: View {
    let session: TerminalSession
    let isSelected: Bool
    let select: () -> Void
    let restart: () -> Void
    let close: () -> Void

    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "terminal")
                .font(.caption)
            Text(session.title)
                .lineLimit(1)
            // An exited shell used to be a dead tab that could only be closed.
            if !session.isRunning {
                Button(action: restart) {
                    Image(systemName: "arrow.clockwise")
                        .font(.caption2.weight(.bold))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("Start a new shell in this tab")
                .accessibilityLabel(Text("Restart \(session.title)"))
            }
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.caption2.weight(.bold))
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .opacity(isHovering || isSelected ? 1 : 0)
            .help("Close terminal")
            .accessibilityLabel(Text("Close \(session.title)"))
            // Hidden means unclickable, so a tab that shows no cross takes no tap
            // where its cross would be and selects like the rest of the tab.
            .allowsHitTesting(isHovering || isSelected)
        }
        .font(.callout)
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .foregroundStyle(isSelected ? .primary : .secondary)
        .background(isSelected ? AnyShapeStyle(.primary.opacity(0.08)) : AnyShapeStyle(Color.clear), in: .capsule)
        .contentShape(.capsule)
        .onTapGesture(perform: select)
        .onHover { isHovering = $0 }
        .opacity(session.isRunning ? 1 : 0.55)
        .help(session.isRunning ? session.title : "\(session.title) · the shell has exited")
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(session.isRunning ? session.title : "\(session.title), the shell has exited"))
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
}

/// Hosts a long-lived terminal view, moving it between containers as threads change.
/// The container coalesces the terminal's own re-wrap while the window resizes (see
/// `TerminalHostContainer`); outside a resize the view resizes exactly as before.
struct TerminalHost: NSViewRepresentable {
    let session: TerminalSession
    let isDark: Bool

    func makeNSView(context: Context) -> NSView {
        let container = TerminalHostContainer()
        attach(to: container)
        return container
    }

    func updateNSView(_ container: NSView, context: Context) {
        session.applyAppearance(isDark: isDark)
        if session.view.superview !== container { attach(to: container) }
        (container as? TerminalHostContainer)?.hostedView = session.view
    }

    private func attach(to container: NSView) {
        let view = session.view
        view.removeFromSuperview()
        view.frame = container.bounds
        view.autoresizingMask = [.width, .height]
        container.addSubview(view)
        session.applyAppearance(isDark: isDark)
        (container as? TerminalHostContainer)?.hostedView = view
    }
}

/// Holds the terminal view while the window resizes. SwiftTerm re-wraps its buffer on every
/// frame-size change, so a live resize that moved the frame every frame reflowed per frame.
/// While the drag lasts the view's autoresizing is parked, and its frame (hence one re-wrap)
/// lands at most every beat; the settled size lands once at the end.
final class TerminalHostContainer: NSView {
    /// The terminal view being held. Set on attach; nil only before the first one.
    weak var hostedView: NSView?
    /// A coalesced frame sync after the last beat, cancelled by the next one.
    private var pendingSync: DispatchWorkItem?
    /// Resize observers for this container's window, re-registered on window moves.
    private var resizeObservers: [NSObjectProtocol] = []

    // Isolated, so the main-actor state can be torn down here at all.
    isolated deinit {
        pendingSync?.cancel()
        for observer in resizeObservers { NotificationCenter.default.removeObserver(observer) }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        for observer in resizeObservers { NotificationCenter.default.removeObserver(observer) }
        resizeObservers = []
        guard let window else { return }
        let center = NotificationCenter.default
        resizeObservers.append(center.addObserver(
            forName: NSWindow.willStartLiveResizeNotification, object: window, queue: .main
        ) { [weak self] _ in
            self?.hostedView?.autoresizingMask = []
        })
        resizeObservers.append(center.addObserver(
            forName: NSWindow.didEndLiveResizeNotification, object: window, queue: .main
        ) { [weak self] _ in
            self?.finishResize()
        })
    }

    override func resizeSubviews(withOldSize oldSize: NSSize) {
        super.resizeSubviews(withOldSize: oldSize)
        guard hostedView != nil, window?.inLiveResize == true else { return }
        // Autoresizing is parked, so without this the view would sit at its old size for
        // the whole drag; with it the frame (and its one re-wrap) follows at most per beat.
        guard pendingSync == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            self?.pendingSync = nil
            self?.syncFrame()
        }
        pendingSync = work
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(50), execute: work)
    }

    /// The settled size, once: autoresizing back on and the frame replayed through the
    /// terminal view, so SwiftTerm recomputes its columns and rows a final time.
    private func finishResize() {
        pendingSync?.cancel()
        pendingSync = nil
        hostedView?.autoresizingMask = [.width, .height]
        syncFrame()
    }

    private func syncFrame() {
        guard let hostedView, hostedView.superview === self else { return }
        let bounds = self.bounds
        guard bounds.width > 0, bounds.height > 0, hostedView.frame != bounds else { return }
        hostedView.frame = bounds
    }
}
