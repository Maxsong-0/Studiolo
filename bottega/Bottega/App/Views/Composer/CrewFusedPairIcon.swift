// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// A pair as one mark: the left half of the lead's provider icon and the right half of
/// the hands', a hairline between them, filled with one gradient that runs from the
/// lead's brand colour into the hands': two models fused into one team. On appear the
/// halves slide in from either side and meet at the line.
struct CrewFusedPairIcon: View {
    let lead: ProviderKind
    let hands: ProviderKind
    var size: CGFloat = 15
    /// Whether the halves fuse on appear; rows in a list draw the mark at rest.
    var animates: Bool = true

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isFused = false

    /// The clear space between each half and the line.
    private let gap: CGFloat = 1
    private var halfWidth: CGFloat { (size - 1) / 2 - gap }

    var body: some View {
        let stops = Self.gradient(lead: lead, hands: hands)
        let fused = isFused || !animates
        ZStack {
            half(of: lead, stops: stops, alignment: .leading)
                .offset(x: fused ? 0 : -size * 0.35)
                .opacity(fused ? 1 : 0)
            half(of: hands, stops: stops, alignment: .trailing)
                .offset(x: fused ? 0 : size * 0.35)
                .opacity(fused ? 1 : 0)
            Capsule()
                .fill(Chrome.secondaryText.opacity(0.55))
                .frame(width: 1, height: size * 0.85)
                .scaleEffect(y: fused ? 1 : 0.15)
        }
        .frame(width: size, height: size)
        .task(id: "\(lead.rawValue)/\(hands.rawValue)") {
            guard animates else { return }
            isFused = false
            try? await Task.sleep(for: .milliseconds(30))
            withAnimation(reduceMotion ? nil : .spring(duration: 0.55, bounce: 0.25)) { isFused = true }
        }
        .accessibilityHidden(true)
    }

    /// One provider's icon as a silhouette, one half of it kept, wearing the gradient.
    private func half(of provider: ProviderKind, stops: [Gradient.Stop], alignment: Alignment) -> some View {
        LinearGradient(stops: stops, startPoint: .leading, endPoint: .trailing)
            .mask {
                Image(provider.iconName)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: size, height: size)
            }
            .mask(alignment: alignment) {
                Rectangle().frame(width: halfWidth, height: size)
            }
            .frame(width: size, height: size)
    }

    /// The lead's brand colours holding the left half and the hands' the right, blending
    /// only across the line between them: the hands' provider's brand when they run on
    /// another provider, else a lighter shade of the lead's own, one model over its
    /// smaller self. Read as `Gradient.Stop`s so the colours sit where the halves are.
    static func gradient(lead: ProviderKind, hands: ProviderKind) -> [Gradient.Stop] {
        let leadBrand = EffortBrand(provider: lead)
        let handColors: [Color] = if hands == lead {
            [leadBrand.isLight ? leadBrand.titleColor.opacity(0.55) : leadBrand.titleColor.mix(with: .white, by: 0.4)]
        } else {
            brandColors(hands)
        }
        return spread(brandColors(lead), from: 0, to: 0.42) + spread(handColors, from: 0.58, to: 1)
    }

    private static func brandColors(_ provider: ProviderKind) -> [Color] {
        let brand = EffortBrand(provider: provider)
        return brand == .antigravity ? EffortBrand.gemini : [brand.titleColor]
    }

    /// The colours laid evenly between two points of the track; one colour holds both.
    private static func spread(_ colors: [Color], from start: Double, to end: Double) -> [Gradient.Stop] {
        guard colors.count > 1 else {
            let color = colors.first ?? .clear
            return [.init(color: color, location: start), .init(color: color, location: end)]
        }
        return colors.enumerated().map { index, color in
            .init(color: color, location: start + (end - start) * Double(index) / Double(colors.count - 1))
        }
    }
}
