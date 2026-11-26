// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// A member of the bus. The four agents by their makers' real icons on a solid tile (colour sits on something solid,
/// not straight on the glass); the dispatcher and the Planner as a letter in a circle; the user a person; LMAS a gear.
struct MemberGlyph: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.colorScheme) private var scheme
    let name: String
    var size: CGFloat = 20

    var body: some View {
        glyph
            .frame(width: size, height: size)
            .help(Text(verbatim: UntrustedText.line(name, limit: 60)))
            .accessibilityLabel(Text(verbatim: UntrustedText.line(name, limit: 60)))
    }

    @ViewBuilder private var glyph: some View {
        if name == Session.user {
            symbol("person.fill", circle: true)
        } else if name == "coordinator" {
            symbol("gearshape.fill", circle: false)
        } else if let vendor = model.state.vendors[name], let maker = Maker(vendor: vendor) {
            MakerTile(maker: maker, size: size)
        } else {
            switch model.state.role(of: name) {
            case "dispatcher": letter("D", filled: true)
            case "planner": letter("P", filled: false)
            default: letter(UntrustedText.line(name, limit: 1).first.map { String($0).uppercased() } ?? "?", filled: false, square: true)
            }
        }
    }

    private func symbol(_ name: String, circle: Bool) -> some View {
        ZStack {
            if circle { Circle().fill(.quaternary) } else { RoundedRectangle(cornerRadius: size * 0.3, style: .continuous).fill(.quaternary) }
            Image(systemName: name).font(.system(size: size * 0.5, weight: .semibold)).foregroundStyle(.secondary)
        }
    }

    private func letter(_ text: String, filled: Bool, square: Bool = false) -> some View {
        ZStack {
            if square {
                RoundedRectangle(cornerRadius: size * 0.3, style: .continuous).fill(.quaternary)
            } else if filled {
                Circle().fill(.primary.opacity(0.85))
            } else {
                Circle().strokeBorder(.secondary, lineWidth: 1.5)
            }
            Text(verbatim: text)
                .font(.system(size: size * 0.5, weight: .bold))
                // The letter in the colour of the glass, cut out of the circle.
                .foregroundStyle(filled ? AnyShapeStyle(scheme == .dark ? Color.black : .white) : AnyShapeStyle(.secondary))
        }
    }
}

/// The four makers whose agents LMAS runs, with the icon Bottega ships for each.
enum Maker: CaseIterable {
    case claude, codex, antigravity, grok

    init?(vendor: String) {
        switch vendor {
        case "claude": self = .claude
        case "codex": self = .codex
        case "agy": self = .antigravity
        case "grok": self = .grok
        default: return nil
        }
    }

    var title: String {
        switch self {
        case .claude: "Claude"
        case .codex: "Codex"
        case .antigravity: "Antigravity"
        case .grok: "Grok"
        }
    }

    var vendor: String {
        switch self {
        case .claude: "claude"
        case .codex: "codex"
        case .antigravity: "agy"
        case .grok: "grok"
        }
    }
}

/// A maker's icon on its tile, as in the design: Claude's own orange on cream, Grok white on black, the rest black on white.
struct MakerTile: View {
    let maker: Maker
    var size: CGFloat = 20

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous).fill(tile)
            RoundedRectangle(cornerRadius: size * 0.3, style: .continuous).strokeBorder(edge, lineWidth: 0.5)
            icon.frame(width: size * scale, height: size * scale)
        }
        .frame(width: size, height: size)
        .accessibilityLabel(maker.title)
    }

    @ViewBuilder private var icon: some View {
        switch maker {
        case .claude: Image("provider-claude").resizable().scaledToFit()
        case .codex: Image("provider-codex").resizable().renderingMode(.template).scaledToFit().foregroundStyle(.black)
        case .antigravity: Image("provider-antigravity").resizable().renderingMode(.template).scaledToFit().foregroundStyle(.black)
        case .grok: Image("provider-grok").resizable().renderingMode(.template).scaledToFit().foregroundStyle(.white)
        }
    }

    private var tile: Color {
        switch maker {
        case .claude: Color(red: 240 / 255, green: 238 / 255, blue: 230 / 255)
        case .codex, .antigravity: .white
        case .grok: .black
        }
    }

    private var edge: Color { maker == .grok ? .white.opacity(0.24) : .black.opacity(0.12) }

    private var scale: CGFloat {
        switch maker {
        case .claude: 0.7
        case .codex: 0.66
        case .antigravity: 0.74
        case .grok: 0.62
        }
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
    var text = "Working"

    /// An AI at work, in a row of other facts: the small square of amber dots and a word.
    var body: some View {
        HStack(spacing: 6) {
            WorkingMatrix(side: 3)
            Text(verbatim: text)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .accessibilityElement(children: .combine)
    }
}
