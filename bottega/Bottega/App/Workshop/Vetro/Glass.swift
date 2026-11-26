// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The photo at the very back of the window, blurred and grained as set in the backdrop panel.
struct BackdropView: View {
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            Color.black
            Image(look.photo.asset)
                .resizable()
                .scaledToFill()
                .blur(radius: look.blur, opaque: true)
                .id(look.photo)
                .transition(.opacity)
            Grain.image
                .resizable(resizingMode: .tile)
                .blendMode(.overlay)
                .opacity(look.grainOpacity)
        }
        .animation(reduceMotion ? .linear(duration: 0.2) : .easeInOut(duration: 0.6), value: look.photo)
        .clipped()
        .ignoresSafeArea()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// Grey noise, made once; tiled over the photo and blended so dark areas do not get snow.
enum Grain {
    static let image: Image = {
        let side = 160
        var pixels = [UInt8](repeating: 0, count: side * side)
        // A fixed seed: the same grain every launch, and no flicker when the view is redrawn.
        var state: UInt64 = 0x9E37_79B9_7F4A_7C15
        for index in pixels.indices {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            pixels[index] = UInt8(truncatingIfNeeded: state >> 56)
        }
        let provider = CGDataProvider(data: Data(pixels) as CFData)
        guard let provider,
              let cg = CGImage(width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: side,
                               space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: 0),
                               provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        else { return Image(systemName: "circle") }
        return Image(decorative: cg, scale: 2)
    }()
}

extension View {
    /// One of the big floating panes: Liquid Glass tinted as the transparency slider says.
    func vetroPane(cornerRadius: CGFloat, isSidebar: Bool = false) -> some View {
        modifier(VetroPane(cornerRadius: cornerRadius, isSidebar: isSidebar))
    }

    /// Something that sits on a pane: a card, the inspector, a list of facts.
    func vetroSurface(cornerRadius: CGFloat) -> some View {
        modifier(VetroSurface(shape: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)))
    }

    func vetroSurface(in shape: some InsettableShape) -> some View {
        modifier(VetroSurface(shape: shape))
    }

    /// Text on glass, kept apart from the photo by a narrow edge rather than a glow.
    func legibleOnGlass() -> some View { modifier(LegibleOnGlass()) }
}

private struct VetroPane: ViewModifier {
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    let cornerRadius: CGFloat
    let isSidebar: Bool

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        if reduceTransparency || contrast == .increased {
            content
                .background(solid, in: shape)
                .overlay { shape.strokeBorder(contrast == .increased ? look.line.opacity(3) : look.line) }
                .clipShape(shape)
                .shadow(color: .black.opacity(0.3), radius: 20, y: 12)
        } else {
            content
                .legibleOnGlass()
                .background { if isSidebar { shape.fill(look.sidebarShade) } }
                .clipShape(shape)
                .glassEffect(.clear.tint(look.glassTint), in: shape)
        }
    }

    private var solid: Color {
        look.isDark ? Color(red: 22 / 255, green: 24 / 255, blue: 30 / 255).opacity(0.96)
            : Color(red: 246 / 255, green: 246 / 255, blue: 248 / 255).opacity(0.96)
    }
}

private struct VetroSurface<S: InsettableShape>: ViewModifier {
    @Environment(Look.self) private var look
    let shape: S

    func body(content: Content) -> some View {
        content
            .background(look.surface, in: shape)
            .overlay {
                shape.strokeBorder(look.line, lineWidth: 1)
                // Light catching the top edge.
                shape.inset(by: 0.5).stroke(
                    LinearGradient(colors: [look.rim, .clear], startPoint: .top, endPoint: .center), lineWidth: 1)
            }
    }
}

private struct LegibleOnGlass: ViewModifier {
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    func body(content: Content) -> some View {
        if reduceTransparency {
            content
        } else {
            let shadow = look.textShadow
            content.shadow(color: shadow.color, radius: shadow.radius, y: shadow.y)
        }
    }
}
