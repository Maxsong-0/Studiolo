// Working indicators, ported from Zeron (https://github.com/zeronsh/zeron), MIT License,
// Copyright (c) 2026 Wing. Source: apps/ios/Zeron/Views/Loaders.swift and apps/ios/Zeron/Theme/Motion.swift.
//
// gradient-spin-pulse: a 3×3 cell grid with per-row "sunrise" tints, here drawn from the theme's
// accent; each cell pulses once per 750ms with phase = distance from bottom-center, so the wave
// travels upward. The mini variant (2×3) snakes clockwise around the perimeter and marks working
// threads in the sidebar.
//
// The pulse is a repeating Core Animation keyframe on each cell's opacity. Once a spinner is on
// screen the render server plays it on its own: no timer, no view update and no layout on the
// main thread for as long as it runs, however many threads are working at once.

import AppKit
import QuartzCore
import SwiftUI

enum GradientSpin {
    /// Row tints drawn from the theme's accent (the system accent for System, Light and
    /// Dark): a lighter shade on top, the accent itself in the middle and a deeper shade
    /// below, so the wave keeps Zeron's sunrise gradient in whatever colour the theme wears.
    /// A dynamic accent resolves for the appearance current when this is called.
    static func rowTints(accent: NSColor) -> [NSColor] {
        let base = accent.usingColorSpace(.sRGB) ?? accent
        return [
            base.blended(withFraction: 0.38, of: .white) ?? base,
            base,
            base.blended(withFraction: 0.22, of: .black) ?? base,
        ]
    }
    static let dim = 0.1
    static let period: Double = 0.75

    /// Full at 0, eases down to dim by 45%, holds to 92%, rises to full by 100%.
    static func opacity(phase: Double) -> Double {
        var p = phase.truncatingRemainder(dividingBy: 1)
        if p < 0 { p += 1 }
        if p < 0.45 {
            let t = p / 0.45
            return 1 - (1 - dim) * (t * t * (3 - 2 * t))
        }
        if p < 0.92 { return dim }
        let t = (p - 0.92) / 0.08
        return dim + (1 - dim) * t
    }
}

/// One cell of a spinner: its row (for the tint) and how far behind the wave it pulses,
/// as a fraction of the period.
struct SpinnerCell {
    var row: Int
    var column: Int
    var lag: Double
}

/// The 3×3 working indicator shown while a reply is being written.
///
/// Hosted as layers, exactly like the sidebar's mini spinners: the pulse is a repeating
/// Core Animation keyframe the render server plays on its own. It used to be a Canvas on a
/// 30 Hz `TimelineView`, which woke SwiftUI thirty times a second for the whole length of a
/// turn — inside the lazy stack, so every one of those ticks landed on the main thread
/// while the reader scrolled. The layers take the row's opacity like any other view, so the
/// working line still fades and moves with the words beside it.
struct WorkingSpinner: View {
    var cellSize: CGFloat = 3.5
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Phase = distance from bottom-center, so the wave travels upward.
    static let cells: [SpinnerCell] = (0..<3).flatMap { row in
        (0..<3).map { column in
            let dx = Double(column - 1)
            let dy = Double(2 - row)
            return SpinnerCell(row: row, column: column, lag: (dx * dx + dy * dy).squareRoot() / 2.5)
        }
    }

    var body: some View {
        // The theme is read here, so a theme change reaches the cells at once.
        SpinnerCells(cells: Self.cells, columns: 3, cellSize: cellSize, animated: !reduceMotion, theme: ThemeManager.current)
            .accessibilityHidden(true)
    }
}

/// The 2×3 mini indicator whose cells snake clockwise, for working threads in lists.
struct MiniSpinner: View {
    var cellSize: CGFloat = 2.4
    /// Draws one still frame with plain shapes instead of hosting a layer view. For image
    /// renderers, which cannot capture AppKit views: the archive ghost is rendered this way.
    var isStill = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private static let ring: [(row: Int, column: Int)] = [
        (0, 0), (0, 1), (1, 1), (2, 1), (2, 0), (1, 0),
    ]

    static let cells: [SpinnerCell] = (0..<3).flatMap { row in
        (0..<2).map { column in
            let index = ring.firstIndex { $0.row == row && $0.column == column } ?? 0
            return SpinnerCell(row: row, column: column, lag: Double(index) / Double(ring.count))
        }
    }

    var body: some View {
        Group {
            if isStill {
                StillSpinnerCells(cells: Self.cells, columns: 2, cellSize: cellSize, tints: GradientSpin.rowTints(accent: Chrome.accentNSColor))
            } else {
                SpinnerCells(cells: Self.cells, columns: 2, cellSize: cellSize, animated: !reduceMotion, theme: ThemeManager.current)
            }
        }
        .accessibilityLabel(Text("Working"))
    }
}

/// A spinner's cells as plain shapes, frozen at the start of the wave.
private struct StillSpinnerCells: View {
    let cells: [SpinnerCell]
    let columns: Int
    let cellSize: CGFloat
    /// One tint per row, resolved by the caller.
    let tints: [NSColor]

    var body: some View {
        let rows = (cells.map(\.row).max() ?? 0) + 1
        VStack(spacing: cellSize * 0.8) {
            ForEach(0..<rows, id: \.self) { row in
                HStack(spacing: cellSize * 0.8) {
                    ForEach(0..<columns, id: \.self) { column in
                        let lag = cells.first { $0.row == row && $0.column == column }?.lag ?? 0
                        Rectangle()
                            .fill(Color(nsColor: tints[row]))
                            .frame(width: cellSize, height: cellSize)
                            .opacity(GradientSpin.opacity(phase: -lag))
                    }
                }
            }
        }
    }
}

/// The cells as layers, each pulsing on a repeating keyframe animation.
private struct SpinnerCells: NSViewRepresentable {
    let cells: [SpinnerCell]
    let columns: Int
    let cellSize: CGFloat
    let animated: Bool
    /// The theme whose accent the rows' tints are drawn from. Passed in so a re-render
    /// under a new theme reaches the cells at once, without waiting for the notification.
    let theme: AppTheme

    func makeNSView(context: Context) -> SpinnerLayerView {
        let view = SpinnerLayerView()
        view.configure(cells: cells, columns: columns, cellSize: cellSize, animated: animated, theme: theme)
        return view
    }

    func updateNSView(_ view: SpinnerLayerView, context: Context) {
        view.configure(cells: cells, columns: columns, cellSize: cellSize, animated: animated, theme: theme)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: SpinnerLayerView, context: Context) -> CGSize? {
        nsView.contentSize
    }
}

final class SpinnerLayerView: NSView {
    private var cells: [SpinnerCell] = []
    private var columns = 0
    private var cellSize: CGFloat = 0
    private var animated = true
    private var theme: AppTheme?
    private var cellLayers: [CALayer] = []

    private static let animationKey = "pulse"
    /// Samples per period. The curve has one soft edge and one sharp one; forty points
    /// keep both without a visible step.
    private static let sampleCount = 40

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        // The cells' colours are set once, not resolved on every draw, so a theme change
        // has to reach them here: a row that never re-renders (a thread working away in
        // the sidebar) would otherwise keep the old theme's colours.
        NotificationCenter.default.addObserver(self, selector: #selector(themeDidChange), name: ThemeManager.didChange, object: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    var contentSize: CGSize {
        let rows = (cells.map(\.row).max() ?? -1) + 1
        let gap = cellSize * 0.8
        return CGSize(
            width: CGFloat(columns) * cellSize + CGFloat(max(0, columns - 1)) * gap,
            height: CGFloat(rows) * cellSize + CGFloat(max(0, rows - 1)) * gap
        )
    }

    override var intrinsicContentSize: NSSize { contentSize }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func configure(cells: [SpinnerCell], columns: Int, cellSize: CGFloat, animated: Bool, theme: AppTheme) {
        let rebuild = cells.count != cellLayers.count
        let retint = rebuild || theme != self.theme
        guard retint || columns != self.columns || cellSize != self.cellSize || animated != self.animated else { return }
        self.cells = cells
        self.columns = columns
        self.cellSize = cellSize
        self.animated = animated
        self.theme = theme
        if rebuild {
            for layer in cellLayers { layer.removeFromSuperlayer() }
            cellLayers = cells.map { _ in
                let layer = CALayer()
                // Never implicitly animated: frames are set once, colours on a theme change
                // and opacity belongs to the keyframes.
                layer.actions = ["opacity": NSNull(), "bounds": NSNull(), "position": NSNull(), "backgroundColor": NSNull()]
                self.layer?.addSublayer(layer)
                return layer
            }
        }
        if retint { applyTints() }
        placeCells()
        installAnimations()
        invalidateIntrinsicContentSize()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // A layer that left the window loses nothing, but one that joins it late (a row
        // built off screen) needs its animation running from the shared clock, and its
        // tints resolved for the window's appearance.
        guard window != nil else { return }
        applyTints()
        installAnimations()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        // The system accent is dynamic; re-resolve it for the new appearance.
        applyTints()
    }

    @objc private func themeDidChange() {
        applyTints()
    }

    /// Colours every cell for its row from the current theme, resolved in this view's
    /// appearance so the system accent picks its light or dark variant.
    private func applyTints() {
        theme = ThemeManager.current
        var tints: [CGColor] = []
        effectiveAppearance.performAsCurrentDrawingAppearance {
            tints = GradientSpin.rowTints(accent: Chrome.accentNSColor).map(\.cgColor)
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (cell, layer) in zip(cells, cellLayers) {
            layer.backgroundColor = tints[cell.row]
        }
        CATransaction.commit()
    }

    override func layout() {
        super.layout()
        placeCells()
    }

    private func placeCells() {
        let rows = (cells.map(\.row).max() ?? -1) + 1
        let gap = cellSize * 0.8
        let content = contentSize
        // Centred in whatever frame SwiftUI hands out; the origin is at the top-left of the grid.
        let originX = ((bounds.width - content.width) / 2).rounded()
        let originY = ((bounds.height - content.height) / 2).rounded()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (cell, layer) in zip(cells, cellLayers) {
            // AppKit's coordinates start at the bottom; row 0 sits at the top of the grid.
            let x = originX + CGFloat(cell.column) * (cellSize + gap)
            let y = originY + CGFloat(rows - 1 - cell.row) * (cellSize + gap)
            layer.frame = CGRect(x: x, y: y, width: cellSize, height: cellSize)
        }
        CATransaction.commit()
    }

    private func installAnimations() {
        for (cell, layer) in zip(cells, cellLayers) {
            layer.removeAnimation(forKey: Self.animationKey)
            guard animated else {
                layer.opacity = Float(GradientSpin.opacity(phase: -cell.lag))
                continue
            }
            layer.opacity = 1
            let animation = CAKeyframeAnimation(keyPath: "opacity")
            // The curve, shifted by the cell's lag, so every cell shares one clock and the
            // wave comes from the offsets alone.
            animation.values = (0...Self.sampleCount).map { step in
                GradientSpin.opacity(phase: Double(step) / Double(Self.sampleCount) - cell.lag)
            }
            animation.calculationMode = .linear
            animation.duration = GradientSpin.period
            animation.repeatCount = .infinity
            // One fixed origin on the shared clock, so every spinner in the app pulses in step.
            animation.beginTime = layer.convertTime(1, from: nil)
            animation.isRemovedOnCompletion = false
            layer.add(animation, forKey: Self.animationKey)
        }
    }
}

/// The word beside the working indicator, rotated every few seconds and seeded per thread,
/// so two threads working at once do not say the same thing.
enum WorkingWords {
    static let words = [
        "Thinking", "Pondering", "Scheming", "Brewing", "Weaving", "Tinkering",
        "Musing", "Composing", "Sifting", "Untangling", "Distilling", "Sketching",
        "Plotting", "Riffing", "Combobulating", "Percolating", "Marinating",
        "Noodling", "Puzzling", "Conjuring", "Creating", "Dancing",
    ]
    static let rotateSeconds: Int64 = 7

    /// FNV-1a over the thread identifier.
    static func seed(_ identifier: String) -> UInt64 {
        var hash: UInt64 = 0xcbf29ce484222325
        for byte in identifier.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x100000001b3
        }
        return hash
    }

    static func word(seed: UInt64, elapsedSeconds: Int64) -> String {
        let step = UInt64(max(0, elapsedSeconds) / rotateSeconds)
        return words[Int((seed &+ step) % UInt64(words.count))]
    }
}
