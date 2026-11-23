// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// Hands one armed drop from the picker to the chip that is about to appear for it.
/// The row arms the drop before its popover closes; the new chip's arrival wrapper
/// takes it once, draws its screen frame, and the flight hands over to the chip.
@MainActor
final class AttachmentDrops {
    static let shared = AttachmentDrops()

    struct Drop {
        let image: NSImage
        let fromScreen: CGRect
        let radius: CGFloat
    }

    private var drop: Drop?

    /// Arms the drop the next arriving chip will pick up. Cleared if nothing claims
    /// it within two seconds, so a stale flight never starts.
    func arm(image: NSImage, fromScreen: CGRect, radius: CGFloat = 6) {
        let armed = Drop(image: image, fromScreen: fromScreen, radius: radius)
        drop = armed
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard let self, self.drop?.image === armed.image else { return }
            self.drop = nil
        }
    }

    func take() -> Drop? {
        let taken = drop
        drop = nil
        return taken
    }
}

/// One-shot ghost flight between two screen rects: a small borderless window holding
/// a single layer, three Core Animation springs moving it from the row's rect to the
/// chip's rect, and nothing left running afterwards. Removed when it lands.
@MainActor
final class AttachmentFlight {
    static let shared = AttachmentFlight()

    private var window: FlightWindow?

    func fly(
        image: NSImage,
        fromScreen: CGRect,
        toScreen: CGRect,
        fromRadius: CGFloat,
        toRadius: CGFloat,
        onLanded: @escaping () -> Void = {}
    ) {
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        if let existing = window {
            existing.orderOut(nil)
            window = nil
        }
        guard let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
              let screen = NSScreen.main else { return }

        let frame = fromScreen.union(toScreen).insetBy(dx: -2, dy: -2)
        let flightWindow = FlightWindow(contentRect: frame, screen: screen)
        window = flightWindow

        let content = NSView()
        content.wantsLayer = true
        flightWindow.contentView = content

        // Window coordinates: the screen origin inside the window's own space.
        let origin = screen.frame.origin
        let start = CGRect(
            x: fromScreen.minX - origin.x, y: fromScreen.minY - origin.y,
            width: fromScreen.width, height: fromScreen.height
        )
        let end = CGRect(
            x: toScreen.minX - origin.x, y: toScreen.minY - origin.y,
            width: toScreen.width, height: toScreen.height
        )

        let layer = CALayer()
        layer.contents = cgImage
        layer.contentsGravity = .resizeAspect
        layer.masksToBounds = true
        layer.cornerCurve = .continuous
        layer.backgroundColor = NSColor.clear.cgColor
        // Model values at the end state; the springs carry it there from the start.
        layer.bounds.size = end.size
        layer.position = CGPoint(x: end.midX, y: end.midY)
        layer.cornerRadius = toRadius

        func spring(from: NSValue) -> CASpringAnimation {
            // `perceptualDuration` and `bounce` are read-only: the SDK takes them only
            // through the initializer that derives the coefficients. The pace comes from
            // the coefficients instead, and the running time from the settling duration
            // they give (mass 1, stiffness 170, damping 26: critically damped, ~0.3 s).
            let animation = CASpringAnimation()
            animation.mass = 1
            animation.stiffness = 170
            animation.damping = 26
            animation.duration = animation.settlingDuration
            animation.fromValue = from
            return animation
        }
        let springs = [
            ("bounds.size", spring(from: NSValue(size: start.size))),
            ("position", spring(from: NSValue(point: CGPoint(x: start.midX, y: start.midY)))),
            ("cornerRadius", spring(from: fromRadius as NSNumber)),
        ]
        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak self] in
            flightWindow.orderOut(nil)
            self?.window = nil
            onLanded()
        }
        for (keyPath, animation) in springs {
            layer.add(animation, forKey: keyPath)
        }
        content.layer?.addSublayer(layer)
        CATransaction.commit()
        flightWindow.orderFrontRegardless()
    }
}

/// The flight's surface. Never key, never main, never clickable.
private final class FlightWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    init(contentRect: CGRect, screen: NSScreen) {
        super.init(contentRect: contentRect, styleMask: .borderless, backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        ignoresMouseEvents = true
        isMovable = false
        level = .popUpMenu
        collectionBehavior = [.canJoinAllSpaces, .transient]
    }
}

/// Reports the frame the wrapped view draws on screen, whenever it lays out.
struct ScreenFrameReader: NSViewRepresentable {
    let onChange: (CGRect) -> Void

    private final class FrameView: NSView {
        var onChange: (CGRect) -> Void

        init(onChange: @escaping (CGRect) -> Void) {
            self.onChange = onChange
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError() }

        override func layout() {
            super.layout()
            report()
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            report()
        }

        private func report() {
            guard let window else { return }
            onChange(window.convertToScreen(convert(bounds, to: nil)))
        }
    }

    func makeNSView(context: Context) -> NSView {
        FrameView(onChange: onChange)
    }

    /// The closure has to be refreshed: a cell that has already appeared hands over a
    /// closure whose `isNew` is false, and a stale one would let it claim a fresh drop
    /// while the strip lays its older cells out around the cell that just appeared.
    func updateNSView(_ nsView: NSView, context: Context) {
        (nsView as? FrameView)?.onChange = onChange
    }
}

/// Wraps a newly appearing attachment cell so an armed drop flies into the exact
/// frame the cell draws. `isNew` is true only the first time a given cell appears,
/// so one flight happens per pick and later relayouts take nothing.
struct AttachmentArrival<Content: View>: View {
    let isNew: Bool
    @ViewBuilder var content: Content

    var body: some View {
        content.background {
            ScreenFrameReader { rect in
                guard isNew, let drop = AttachmentDrops.shared.take() else { return }
                AttachmentFlight.shared.fly(
                    image: drop.image,
                    fromScreen: drop.fromScreen,
                    toScreen: rect,
                    fromRadius: drop.radius,
                    toRadius: 10
                )
            }
        }
    }
}
