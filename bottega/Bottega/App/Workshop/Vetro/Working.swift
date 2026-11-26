// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// What "an AI is working" looks like (Bottega-5 §4 and §7, ADR-047 items 15 and 16, canvas version 8): amber dots
/// that light up and fade back, here and there, straight on the glass. Amber means only this; a program running (a
/// command, a background script) keeps the grey spinner. The colours are here and nowhere else.
enum WorkingAmber {
    /// On dark glass: bright, with a strong glow.
    static let bright = Color(red: 1, green: 194 / 255, blue: 71 / 255)
    static let brightGlow = Color(red: 1, green: 166 / 255, blue: 51 / 255).opacity(0.85)
    /// On light glass: deeper, and the glow faint.
    static let deep = Color(red: 194 / 255, green: 116 / 255, blue: 0)
    static let deepGlow = Color(red: 214 / 255, green: 128 / 255, blue: 0).opacity(0.4)
    /// The step being done in a timeline: a still dot.
    static let step = Color(red: 233 / 255, green: 162 / 255, blue: 59 / 255)

    /// The band of light passing over the words, and the glow that goes only where it is.
    static func band(isDark: Bool) -> (color: Color, near: Color, far: Color?) {
        isDark
            ? (Color(red: 1, green: 243 / 255, blue: 217 / 255),
               Color(red: 1, green: 178 / 255, blue: 70 / 255).opacity(0.9), Color(red: 1, green: 140 / 255, blue: 30 / 255).opacity(0.5))
            : (Color(red: 217 / 255, green: 134 / 255, blue: 15 / 255), Color(red: 1, green: 160 / 255, blue: 40 / 255).opacity(0.55), nil)
    }
}

/// A square of tiny dots, `side` by `side` (7 in a turn's status line, 3 for a status dot), each lighting up and
/// dimming on its own beat. With less motion it holds one frame with a few lit.
struct WorkingMatrix: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme
    var side = 7

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

    private func draw(in context: inout GraphicsContext, at time: TimeInterval?) {
        let isDark = scheme == .dark
        let color = isDark ? WorkingAmber.bright : WorkingAmber.deep
        context.addFilter(.shadow(color: isDark ? WorkingAmber.brightGlow : WorkingAmber.deepGlow, radius: 1.25))
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

/// A turn running, at the top of its summary: the dots where a spinner would be, and the words with a warm band of
/// light passing over them. No background of its own: it sits on the glass (canvas version 8).
struct WorkingStatus: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            WorkingMatrix(side: 7).padding(.horizontal, 1)
            label
        }
        .font(.system(size: 13))
        .lineLimit(1)
        .frame(minHeight: 30)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
    }

    @ViewBuilder private var label: some View {
        let isDark = scheme == .dark
        if reduceMotion {
            Text(verbatim: text).foregroundStyle(.secondary)
        } else {
            let band = WorkingAmber.band(isDark: isDark)
            Text(verbatim: text)
                // Dark glass: the words at half strength; light glass: the quiet grey.
                .foregroundStyle(isDark ? AnyShapeStyle(.primary.opacity(0.52)) : AnyShapeStyle(.secondary))
                .overlay {
                    TimelineView(.animation(minimumInterval: 1.0 / 30)) { timeline in
                        let phase = timeline.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2.4) / 2.4
                        // The band runs from left to right; the glow is drawn from the lit letters only, so it moves with them.
                        Text(verbatim: text)
                            .foregroundStyle(band.color)
                            .mask {
                                GeometryReader { proxy in
                                    let width = proxy.size.width
                                    LinearGradient(colors: [.clear, .black, .clear], startPoint: .leading, endPoint: .trailing)
                                        .frame(width: width * 0.36)
                                        .offset(x: -width * 0.36 + phase * width * 1.36)
                                }
                            }
                            .shadow(color: band.near, radius: isDark ? 1 : 1.5)
                            .shadow(color: band.far ?? .clear, radius: 3.5)
                    }
                }
        }
    }
}

/// The step being done right now in a timeline: amber, and still. The dots by the status line are the one thing on
/// screen that moves.
struct RunningStepDot: View {
    var body: some View {
        Circle()
            .fill(WorkingAmber.step)
            .frame(width: 9, height: 9)
            .background { Circle().fill(WorkingAmber.step.opacity(0.22)).frame(width: 15, height: 15) }
            .accessibilityLabel("Now")
    }
}
