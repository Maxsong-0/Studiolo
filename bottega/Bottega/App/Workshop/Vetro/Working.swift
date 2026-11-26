// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// What "an AI is working" looks like (Bottega-5 §4 and §7, ADR-047 items 15 and 16): amber dots that light up and
/// fade back, here and there. Amber means only this; a program running (a command, a background script) keeps the
/// grey spinner.
enum WorkingAmber {
    /// On the dark capsule, and on dark glass.
    static let bright = Color(red: 1, green: 194 / 255, blue: 71 / 255)
    static let brightGlow = Color(red: 1, green: 166 / 255, blue: 51 / 255).opacity(0.85)
    /// On light glass: deeper, and without the glow.
    static let deep = Color(red: 194 / 255, green: 116 / 255, blue: 0)
    /// The step being done in a timeline: a still dot.
    static let step = Color(red: 233 / 255, green: 162 / 255, blue: 59 / 255)
}

/// A square of tiny dots, `side` by `side` (7 in the capsule, 3 for a status dot), each lighting up and dimming on its
/// own beat. With less motion it holds one frame with a few lit.
struct WorkingMatrix: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme
    var side = 7
    /// The capsule is always dark, whatever the glass.
    var onDark = false

    private static let dot: CGFloat = 2
    private static let gap: CGFloat = 1

    var body: some View {
        let length = CGFloat(side) * Self.dot + CGFloat(side - 1) * Self.gap
        Group {
            if reduceMotion {
                Canvas { context, _ in draw(in: &context, at: nil) }
            } else {
                TimelineView(.animation(minimumInterval: 1.0 / 30)) { timeline in
                    Canvas { context, _ in draw(in: &context, at: timeline.date.timeIntervalSinceReferenceDate) }
                }
            }
        }
        .frame(width: length, height: length)
        .accessibilityHidden(true)
    }

    private var isBright: Bool { onDark || scheme == .dark }

    private func draw(in context: inout GraphicsContext, at time: TimeInterval?) {
        let color = isBright ? WorkingAmber.bright : WorkingAmber.deep
        if isBright { context.addFilter(.shadow(color: WorkingAmber.brightGlow, radius: 1.25)) }
        for index in 0..<(side * side) {
            let opacity = time.map { WorkingPattern.opacity(of: index, count: side * side, at: $0) } ?? WorkingPattern.still(of: index, side: side)
            let x = CGFloat(index % side) * (Self.dot + Self.gap)
            let y = CGFloat(index / side) * (Self.dot + Self.gap)
            context.fill(
                Path(roundedRect: CGRect(x: x, y: y, width: Self.dot, height: Self.dot), cornerRadius: 0.6),
                with: .color(color.opacity(opacity)))
        }
    }
}

/// A turn running: a black capsule, the dots on the left, and the words with a warm band of light passing over them.
struct WorkingCapsule: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let text: String

    private static let words = Color(red: 142 / 255, green: 138 / 255, blue: 132 / 255)
    private static let band = Color(red: 1, green: 243 / 255, blue: 217 / 255)
    private static let still = Color(red: 234 / 255, green: 223 / 255, blue: 200 / 255)

    var body: some View {
        HStack(spacing: 8) {
            WorkingMatrix(side: 7, onDark: true).padding(.horizontal, 1)
            label
        }
        .font(.system(size: 13.5, weight: .medium))
        .lineLimit(1)
        .frame(height: 30)
        .padding(.leading, 6)
        .padding(.trailing, 13)
        .background(Color(red: 11 / 255, green: 10 / 255, blue: 8 / 255).opacity(0.94), in: .capsule)
        .overlay { Capsule().strokeBorder(.white.opacity(0.08)) }
        .shadow(color: .black.opacity(0.55), radius: 8, y: 6)
        .environment(\.colorScheme, .dark)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
    }

    @ViewBuilder private var label: some View {
        if reduceMotion {
            Text(verbatim: text).foregroundStyle(Self.still)
        } else {
            Text(verbatim: text)
                .foregroundStyle(Self.words)
                .overlay {
                    TimelineView(.animation(minimumInterval: 1.0 / 30)) { timeline in
                        let phase = timeline.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2.4) / 2.4
                        // The band runs from left to right; the glow is drawn from the lit letters only, so it moves with them.
                        Text(verbatim: text)
                            .foregroundStyle(Self.band)
                            .mask {
                                GeometryReader { proxy in
                                    let width = proxy.size.width
                                    LinearGradient(colors: [.clear, .black, .clear], startPoint: .leading, endPoint: .trailing)
                                        .frame(width: width * 0.36)
                                        .offset(x: -width * 0.36 + phase * width * 1.36)
                                }
                            }
                            .shadow(color: Color(red: 1, green: 178 / 255, blue: 70 / 255).opacity(0.9), radius: 1)
                            .shadow(color: Color(red: 1, green: 140 / 255, blue: 30 / 255).opacity(0.5), radius: 3.5)
                    }
                }
        }
    }
}

/// The step being done right now in a timeline: amber, and still. The capsule is the one thing on screen that moves.
struct RunningStepDot: View {
    var body: some View {
        Circle()
            .fill(WorkingAmber.step)
            .frame(width: 9, height: 9)
            .background { Circle().fill(WorkingAmber.step.opacity(0.22)).frame(width: 15, height: 15) }
            .accessibilityLabel("Now")
    }
}
