// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// A member of the bus as a letter: C, X, A, G for the four agents, D for the dispatcher. Their shades differ in
/// lightness as well, so they do not rest on colour. The user is a person, LMAS a gear, the Planner a P.
struct MemberGlyph: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.colorScheme) private var scheme
    let name: String
    var size: CGFloat = 20

    var body: some View {
        let look = look
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.28, style: .continuous).fill(background(look.shade))
            if let symbol = look.symbol {
                Image(systemName: symbol).font(.system(size: size * 0.52, weight: .semibold))
            } else {
                Text(verbatim: look.letter).font(.system(size: size * 0.6, weight: .semibold))
            }
        }
        .foregroundStyle(look.shade < 0.5 ? Color.white : .black)
        .frame(width: size, height: size)
        .help(Text(verbatim: UntrustedText.line(name, limit: 60)))
        .accessibilityLabel(Text(verbatim: UntrustedText.line(name, limit: 60)))
    }

    private struct Look {
        var letter = ""
        var symbol: String?
        /// 0 black, 1 white.
        var shade: Double
    }

    private var look: Look {
        if name == Session.user { return Look(symbol: "person.fill", shade: 0.42) }
        if name == "coordinator" { return Look(symbol: "gearshape.fill", shade: 0.62) }
        switch model.state.vendors[name] {
        case "claude": return Look(letter: "C", shade: 0.2)
        case "codex": return Look(letter: "X", shade: 0.36)
        case "agy": return Look(letter: "A", shade: 0.66)
        case "grok": return Look(letter: "G", shade: 0.82)
        default: break
        }
        switch model.state.role(of: name) {
        case "dispatcher": return Look(letter: "D", shade: 0.05)
        case "planner": return Look(letter: "P", shade: 0.92)
        default:
            let first = UntrustedText.line(name, limit: 1).first.map { String($0).uppercased() } ?? "?"
            return Look(letter: first, shade: 0.74)
        }
    }

    /// In the dark the whole scale lifts off the background, keeping its order.
    private func background(_ shade: Double) -> Color {
        Color(white: scheme == .dark ? 0.24 + shade * 0.7 : shade)
    }
}

/// A member's letter and name together.
struct MemberName: View {
    let name: String
    var size: CGFloat = 18

    var body: some View {
        HStack(spacing: 5) {
            MemberGlyph(name: name, size: size)
            Text(verbatim: UntrustedText.line(name, limit: 40)).lineLimit(1)
        }
    }
}

/// A state: always a symbol and a word, never just a colour.
struct StatusChip: View {
    let text: String
    let symbol: String
    var waits = false

    var body: some View {
        Label {
            Text(verbatim: text).lineLimit(2)
        } icon: {
            Image(systemName: symbol)
        }
        .font(.caption)
        .foregroundStyle(waits ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Shimmers while an agent works; still for those who asked for less motion.
struct WorkingLabel: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var text = "Working…"

    var body: some View {
        if reduceMotion {
            label
        } else {
            TimelineView(.animation(minimumInterval: 1.0 / 30)) { context in
                let phase = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2) / 2
                label.mask {
                    LinearGradient(
                        stops: [
                            .init(color: .black.opacity(0.55), location: phase * 1.6 - 0.6),
                            .init(color: .black, location: phase * 1.6 - 0.3),
                            .init(color: .black.opacity(0.55), location: phase * 1.6),
                        ],
                        startPoint: .leading, endPoint: .trailing)
                }
            }
        }
    }

    private var label: some View {
        Label(text, systemImage: "circle.lefthalf.filled").font(.caption).foregroundStyle(.secondary)
    }
}
