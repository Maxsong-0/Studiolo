// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import QuartzCore
import SwiftUI

/// A hand's mark: its own signal flag, in its colour. The same glyph stands for the hand
/// everywhere it shows: the panel, the popover, the tool row that sent it out and the
/// report it hands back. The twenty-five hands are template images, so one colour tints
/// each whole.
struct CrewGlyph: View {
    let persona: CrewPersona
    var size: CGFloat = 20
    /// A soft ring breathes around a hand still at work.
    var isRunning = false
    /// A finished hand wears its outcome on the glyph's bottom-right corner: a green tick
    /// for done, red for failed, grey for stopped. Nil, or running, shows nothing.
    var status: CrewHandInfo.Status?

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        ZStack {
            if isRunning {
                CrewBreathRing(color: NSColor(persona.color.opacity(0.55)).cgColor, lineWidth: 1.5)
                    .frame(width: size, height: size)
                    .allowsHitTesting(false)
            }
            Image(persona.asset)
                .resizable()
                .renderingMode(.template)
                .aspectRatio(contentMode: .fit)
                .foregroundStyle(persona.color)
                .frame(width: size, height: size)
        }
        .frame(width: size, height: size)
        .overlay(alignment: .bottomTrailing) {
            if let status, status.isFinished {
                badge(for: status)
            }
        }
        .accessibilityLabel(Text(status.map { "\(persona.name), \($0.rawValue)" } ?? persona.name))
    }

    /// The outcome as a small disc riding the corner, cut out from the glyph by a ring in
    /// the surface's colour, the way a presence badge sits on an avatar.
    private func badge(for status: CrewHandInfo.Status) -> some View {
        let diameter = max(8, size * 0.5)
        let (symbol, color): (String, Color) = switch status {
        case .completed: ("checkmark", Chrome.success)
        case .failed: ("xmark", Chrome.danger)
        default: ("stop.fill", Chrome.secondaryText)
        }
        return ZStack {
            Circle()
                .fill(color)
            Image(systemName: symbol)
                .font(.system(size: diameter * 0.55, weight: .heavy))
                .foregroundStyle(.white)
        }
        .frame(width: diameter, height: diameter)
        .overlay {
            Circle().strokeBorder(colorScheme == .dark ? Color.black.opacity(0.55) : Color.white.opacity(0.95), lineWidth: max(1, diameter * 0.12))
        }
        .offset(x: diameter * 0.3, y: diameter * 0.3)
        .transition(.scale.combined(with: .opacity))
    }
}

/// The ring around a working hand: it swells and fades, over and over. Drawn by Core
/// Animation, so the breath runs on the render server and no frame of it re-renders
/// the pill or the panel the glyph sits in (a SwiftUI `repeatForever` did, at the
/// display's full rate, once per hand at work).
struct CrewBreathRing: NSViewRepresentable {
    let color: CGColor
    let lineWidth: CGFloat

    func makeNSView(context: Context) -> CrewBreathRingView {
        let view = CrewBreathRingView()
        view.configure(color: color, lineWidth: lineWidth)
        return view
    }

    func updateNSView(_ view: CrewBreathRingView, context: Context) {
        view.configure(color: color, lineWidth: lineWidth)
    }
}

final class CrewBreathRingView: NSView {
    private let ring = CAShapeLayer()
    private var lineWidth: CGFloat = 0

    private static let animationKey = "breath"
    /// One breath: the ring grows from just outside the glyph to half again its size
    /// while it fades out, then starts over.
    private static let period: CFTimeInterval = 1.6
    private static let restingScale: CGFloat = 1.05
    private static let swollenScale: CGFloat = 1.5
    private static let restingOpacity: Float = 0.9

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        // The ring swells past the glyph's frame: nothing here may clip it.
        layer?.masksToBounds = false
        ring.fillColor = nil
        ring.anchorPoint = CGPoint(x: 0.5, y: 0.5)
        // Never implicitly animated: the path and colour are set once, and the breath is
        // an explicit animation of its own.
        ring.actions = [
            "path": NSNull(), "strokeColor": NSNull(), "lineWidth": NSNull(), "bounds": NSNull(),
            "position": NSNull(), "opacity": NSNull(), "transform": NSNull(), "contentsScale": NSNull(),
        ]
        layer?.addSublayer(ring)
        // The render server keeps an animation only while its layer is on screen; the app
        // coming back to the front is a moment a ring may have lost its breath.
        NotificationCenter.default.addObserver(self, selector: #selector(applicationDidBecomeActive), name: NSApplication.didBecomeActiveNotification, object: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func configure(color: CGColor, lineWidth: CGFloat) {
        let resized = lineWidth != self.lineWidth
        self.lineWidth = lineWidth
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        ring.strokeColor = color
        ring.lineWidth = lineWidth
        CATransaction.commit()
        if resized { placeRing() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // A ring that joins a window late (a row built off screen, a panel just opened)
        // needs its breath running from the shared clock, and its stroke at the window's scale.
        guard window != nil else { return }
        matchBackingScale()
        installAnimation()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        matchBackingScale()
    }

    @objc private func applicationDidBecomeActive() {
        guard window != nil else { return }
        installAnimation()
    }

    override func layout() {
        super.layout()
        placeRing()
    }

    /// The stroke is rasterised at the layer's scale; a Retina window needs it told.
    private func matchBackingScale() {
        guard let scale = window?.backingScaleFactor else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        ring.contentsScale = scale
        CATransaction.commit()
    }

    /// The ring fills the view, centred, so the breath scales it about the glyph's middle.
    private func placeRing() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        ring.bounds = bounds
        ring.position = CGPoint(x: bounds.midX, y: bounds.midY)
        ring.path = CGPath(ellipseIn: bounds.insetBy(dx: lineWidth / 2, dy: lineWidth / 2), transform: nil)
        CATransaction.commit()
    }

    private func installAnimation() {
        ring.removeAnimation(forKey: Self.animationKey)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        // The resting frame: what shows under reduced motion, and between breaths.
        ring.opacity = Self.restingOpacity
        ring.transform = CATransform3DMakeScale(Self.restingScale, Self.restingScale, 1)
        CATransaction.commit()
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        let swell = CABasicAnimation(keyPath: "transform.scale")
        swell.fromValue = Self.restingScale
        swell.toValue = Self.swollenScale
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = Self.restingOpacity
        fade.toValue = 0
        for animation in [swell, fade] {
            animation.duration = Self.period
            animation.timingFunction = CAMediaTimingFunction(name: .easeOut)
        }
        let breath = CAAnimationGroup()
        breath.animations = [swell, fade]
        breath.duration = Self.period
        breath.repeatCount = .infinity
        // One fixed origin on the shared clock, so every ring in the app breathes in step
        // and a restart picks the breath up where it is rather than snapping it back.
        breath.beginTime = ring.convertTime(1, from: nil)
        breath.isRemovedOnCompletion = false
        ring.add(breath, forKey: Self.animationKey)
    }
}

/// The Crew mark: an anchor, a template that takes whatever colour it
/// is given. Sized by its frame.
struct CrewMarkImage: View {
    var body: some View {
        Image("crew-mark")
            .resizable()
            .renderingMode(.template)
            .aspectRatio(contentMode: .fit)
    }
}

enum CrewNameStyle {
    /// The hand's name as Text, all in one colour: "Otto 2" reads as a single name, its
    /// round number never picked out in the hand's own colour the glyph beside it wears.
    static func text(_ persona: CrewPersona, base: Color) -> Text {
        Text(verbatim: persona.name).foregroundColor(base)
    }
}

struct CrewNameText: View {
    let persona: CrewPersona
    var size: CGFloat = 13
    var weight: Font.Weight = .medium
    var color: Color = Chrome.primaryText.opacity(0.92)
    var body: some View {
        CrewNameStyle.text(persona, base: color).zoomedFont(size: size, weight: weight)
    }
}
