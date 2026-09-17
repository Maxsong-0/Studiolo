import AppKit
import QuartzCore
import SwiftUI

/// The composer's working animation: a field of accent-tinted dots along the pill's
/// bottom, pulsing in a wave that flows left to right and fades out upward.
///
/// One dot layer carries one repeating opacity keyframe; two replicator layers stamp it
/// across the columns and rows with a per-instance delay, which is what makes the wave
/// travel. Once on screen the render server plays it alone: no timer, no view update
/// and no drawing on the main thread, however long a turn runs.
struct ComposerWorkingDots: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        DotFieldLayerView.Representable(color: Chrome.accentNSColor, animated: !reduceMotion)
            .accessibilityHidden(true)
            .allowsHitTesting(false)
    }
}

/// The same field over a whole surface, once: one wave of dots in `tint` flows through it
/// from left to right and is gone, the way a head's panel marks its head finishing. With
/// reduced motion nothing is drawn.
struct DotFieldSweep: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let tint: NSColor

    var body: some View {
        if !reduceMotion {
            DotFieldLayerView.Representable(color: tint, animated: true, sweeps: true)
                .accessibilityHidden(true)
                .allowsHitTesting(false)
        }
    }
}

/// The same field through a whole surface, endlessly: the pill's wave over every row at
/// one strength, for the timeline's working badge. Still with reduced motion.
struct DotFieldFill: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        DotFieldLayerView.Representable(color: Chrome.accentNSColor, animated: !reduceMotion, fills: true)
            .accessibilityHidden(true)
            .allowsHitTesting(false)
    }
}

final class DotFieldLayerView: NSView {
    struct Representable: NSViewRepresentable {
        let color: NSColor
        let animated: Bool
        /// One pass over every row instead of the pill's endless wave along the bottom.
        var sweeps = false
        var fills = false

        func makeNSView(context: Context) -> DotFieldLayerView {
            let view = DotFieldLayerView()
            view.sweeps = sweeps
            view.fills = fills
            view.configure(color: color, animated: animated)
            return view
        }

        func updateNSView(_ view: DotFieldLayerView, context: Context) {
            view.configure(color: color, animated: animated)
        }
    }

    private let rows = CAReplicatorLayer()
    private let columns = CAReplicatorLayer()
    private let dot = CALayer()
    private var color: NSColor = .controlAccentColor
    private var animated = true
    private var laidOutSize: CGSize = .zero
    /// Set once, before the view is configured: a sweep fills every row of its surface
    /// with one wave that starts when the view appears and ends faded out.
    fileprivate var sweeps = false
    /// The pill's endless wave over every row of the surface at one strength, for a badge the field runs through.
    fileprivate var fills = false

    private static let dotSize: CGFloat = 2
    private static let pitch: CGFloat = 8
    /// The field never grows past this many rows however tall the pill gets, so a
    /// five-line draft does not multiply the layers the render server composites.
    private static let maxRows = 3
    /// One pulse per dot; the column delay spreads it into a travelling wave.
    private static let period: Double = 2.2
    private static let columnDelay: Double = 0.045
    private static let rowDelay: Double = 0.09
    private static let animationKey = "pulse"
    /// The sweep's single pulse, and the delay between its columns: a wave that crosses a
    /// panel of 400 points in about a second.
    private static let sweepPeriod: Double = 0.8
    private static let sweepColumnDelay: Double = 0.022
    /// How long a sweep takes over a surface `width` wide, from the first column's rise
    /// to the last one's fade.
    static func sweepDuration(width: CGFloat) -> TimeInterval {
        Double(Int(ceil(width / pitch)) + 1) * sweepColumnDelay + sweepPeriod
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        dot.actions = ["opacity": NSNull(), "bounds": NSNull(), "position": NSNull(), "backgroundColor": NSNull()]
        dot.cornerRadius = Self.dotSize / 2
        dot.bounds = CGRect(x: 0, y: 0, width: Self.dotSize, height: Self.dotSize)
        for replicator in [columns, rows] {
            replicator.actions = ["bounds": NSNull(), "position": NSNull(), "instanceCount": NSNull(), "instanceTransform": NSNull()]
        }
        columns.addSublayer(dot)
        rows.addSublayer(columns)
        layer?.addSublayer(rows)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func configure(color: NSColor, animated: Bool) {
        let colorChanged = color != self.color
        let motionChanged = animated != self.animated
        self.color = color
        self.animated = animated
        if colorChanged { dot.backgroundColor = color.cgColor }
        if motionChanged { installAnimation() }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { installAnimation() }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        // The system accent is dynamic; re-resolve it for the new appearance.
        dot.backgroundColor = color.cgColor
    }

    override func layout() {
        super.layout()
        let size = bounds.size
        guard size != laidOutSize, size.width > 0, size.height > 0 else { return }
        laidOutSize = size
        let columnCount = Int(ceil(size.width / Self.pitch)) + 1
        let rowCount = (sweeps || fills)
            ? max(1, Int(ceil(size.height / Self.pitch)))
            : min(Self.maxRows, max(1, Int(ceil(size.height / Self.pitch))))
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        rows.frame = bounds
        columns.frame = CGRect(x: 0, y: 0, width: size.width, height: Self.pitch)
        // The base dot sits at the bottom-left; columns march right and rows stack upward,
        // each row a little fainter, so the field dissolves before it reaches the text.
        dot.position = CGPoint(x: Self.pitch / 2, y: Self.pitch / 2)
        columns.instanceCount = columnCount
        columns.instanceTransform = CATransform3DMakeTranslation(Self.pitch, 0, 0)
        columns.instanceDelay = sweeps ? Self.sweepColumnDelay : Self.columnDelay
        rows.instanceCount = rowCount
        rows.instanceTransform = CATransform3DMakeTranslation(0, Self.pitch, 0)
        // A sweep's rows move as one front, all the same, so the wave is a wall crossing
        // the surface rather than the pill's fade-out upward.
        rows.instanceDelay = sweeps ? 0 : Self.rowDelay
        rows.instanceAlphaOffset = (sweeps || fills) ? 0 : -Float(1) / Float(rowCount)
        CATransaction.commit()
    }

    private func installAnimation() {
        dot.removeAnimation(forKey: Self.animationKey)
        guard animated else {
            dot.opacity = sweeps ? 0 : 0.28
            return
        }
        if sweeps {
            // Invisible before and after: the field only exists while the wave crosses it.
            dot.opacity = 0
            let sweep = CAKeyframeAnimation(keyPath: "opacity")
            sweep.values = [0, 0.7, 0]
            sweep.keyTimes = [0, 0.4, 1]
            sweep.timingFunctions = [
                CAMediaTimingFunction(name: .easeOut),
                CAMediaTimingFunction(name: .easeIn),
            ]
            sweep.duration = Self.sweepPeriod
            sweep.beginTime = dot.convertTime(CACurrentMediaTime(), from: nil)
            // Kept once played: the replicated columns run it late by their delay, and
            // would lose it the moment the base dot's own pass ended.
            sweep.isRemovedOnCompletion = false
            sweep.fillMode = .forwards
            dot.add(sweep, forKey: Self.animationKey)
            return
        }
        let animation = CAKeyframeAnimation(keyPath: "opacity")
        animation.values = [0.08, 0.75, 0.08]
        animation.keyTimes = [0, 0.35, 1]
        animation.timingFunctions = [
            CAMediaTimingFunction(name: .easeOut),
            CAMediaTimingFunction(name: .easeIn),
        ]
        animation.duration = Self.period
        animation.repeatCount = .infinity
        // The shared clock, so every pill's field runs in step with the spinners.
        animation.beginTime = dot.convertTime(1, from: nil)
        animation.isRemovedOnCompletion = false
        dot.add(animation, forKey: Self.animationKey)
    }
}
