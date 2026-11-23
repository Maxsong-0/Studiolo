// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// Tokens and building blocks shared with Droppy's settings panel, so the two apps read as one family.
enum Chrome {
    // MARK: Window

    static let windowCornerRadius: CGFloat = 26
    /// The detail sheet runs to the window's edge: inset, it drew a second, tighter corner
    /// inside the window's own, a clipped double border wherever the sidebar left it bare.
    static let sheetInset: CGFloat = 0
    /// The rounding of surfaces that sit on the sheet, like the terminal's top corners.
    static let sheetCornerRadius: CGFloat = 16

    static let trafficLightDiameter: CGFloat = 14
    static let trafficLightSpacing: CGFloat = 9
    static var trafficLightPitch: CGFloat { trafficLightDiameter + trafficLightSpacing }
    static let trafficLightLeading: CGFloat = 18
    static let trafficLightTop: CGFloat = 16
    static var trafficLightsWidth: CGFloat { trafficLightDiameter * 3 + trafficLightSpacing * 2 }
    static let trafficLightClearance: CGFloat = 10

    // MARK: Sticky chrome row

    /// One weight and size for every symbol that is a control on its own: chrome buttons,
    /// the composer's paperclip and send, the terminal strip. Symbols beside text step down
    /// a point so they sit level with the type; menu chevrons are small and bold.
    /// A control's own symbol keeps this size at every text size, the way a toolbar's
    /// icons do under Dynamic Type: its capsule or circle is a fixed size.
    static let iconFont: Font = .system(size: 12, weight: .semibold)
    static let inlineIconFont: Font = .system(size: 11, weight: .semibold)
    static let chevronFont: Font = .system(size: 8, weight: .bold)
    /// The two fonts above that sit beside text, at the window's text size (see `ChatZoom`),
    /// so a symbol stays level with the type it sits in: the same sizes and weights as the
    /// stock fonts, exactly those at the stock step. Views reach them through
    /// `chromeInlineIconFont()` and `chromeChevronFont()`.
    static func inlineIconFont(zoom: CGFloat) -> Font { .zoomed(size: 11, weight: .semibold, zoom: zoom) }
    static func chevronFont(zoom: CGFloat) -> Font { .zoomed(size: 8, weight: .bold, zoom: zoom) }
    static let chromeHorizontalPadding: CGFloat = 14
    static let chromeTopPadding: CGFloat = 12
    static let capsuleContentHeight: CGFloat = 28
    static let capsuleVerticalPadding: CGFloat = 2
    static let capsuleHorizontalPadding: CGFloat = 12
    static let iconCapsuleInnerPadding: CGFloat = 3
    static let dividerHeight: CGFloat = 14
    static var capsuleHeight: CGFloat { capsuleContentHeight + capsuleVerticalPadding * 2 }
    static let contentBreathBelowChrome: CGFloat = 16
    static var contentTopInset: CGFloat { chromeTopPadding + capsuleHeight + contentBreathBelowChrome }
    static let veilHeight: CGFloat = 72
    static let titleStartOffset: CGFloat = 10
    static let titleEndOffset: CGFloat = 44
    static let progressQuantum: CGFloat = 0.04

    // MARK: Sidebar

    /// A sidebar row at the stock text size; the rows scale it with their titles (see
    /// `ChatZoom.scaled`).
    static let rowHeight: CGFloat = 28
    static let rowCornerRadius: CGFloat = 7
    static let rowHorizontalPadding: CGFloat = 8
    static let iconSize: CGFloat = 20
    static let symbolSize: CGFloat = 14
    static let groupGap: CGFloat = 10
    static let listInset: CGFloat = 10

    // MARK: Cards

    /// The conversation column and the chat box share this ceiling: wide enough for code,
    /// narrow enough that a line of prose is still one glance.
    static let columnMaxWidth: CGFloat = 960
    static let cardCornerRadius: CGFloat = 16
    /// The remaining radii, one name each, so the whole app's roundness is tuned here.
    static let composerCornerRadius: CGFloat = 22
    static let paletteCornerRadius: CGFloat = 24
    static let blockCornerRadius: CGFloat = 12
    static let chipCornerRadius: CGFloat = 10
    static let menuRowCornerRadius: CGFloat = 6
    static let sectionSpacing: CGFloat = 20
    static let sectionHeaderSpacing: CGFloat = 10
    static let contentHorizontalPadding: CGFloat = 16
    /// The gap between a row's trailing control and the card edge.
    static let rowControlTrailingPadding: CGFloat = 10
    /// The height a settings row's trailing button label is held to. A label in another
    /// face (a monospaced command once stood beside the plain "Sign in") draws a taller
    /// capsule, so the two read as two sizes; one clamped label height keeps the controls
    /// in a row even.
    static let rowButtonLabelHeight: CGFloat = 16

    // MARK: Color and motion

    static func overlay(_ opacity: Double) -> Color {
        primaryText.opacity(opacity)
    }

    // Dynamic system colors resolve at draw time, so one wrapped value serves every appearance.
    static let primaryText = Color(nsColor: .labelColor)
    static let secondaryText = Color(nsColor: .secondaryLabelColor)
    private static let systemAccent = Color(nsColor: .controlAccentColor)

    /// The theme's accent, or the system control accent for System/Light/Dark.
    /// The window root also applies it as the view tint, so prominent glass
    /// buttons, toggles and progress indicators follow the theme.
    static var accent: Color { ThemeManager.spec.accent ?? systemAccent }
    /// The same accent for AppKit: text view link attributes and layer colours.
    static var accentNSColor: NSColor { ThemeManager.spec.accent.map { NSColor($0) } ?? .controlAccentColor }

    /// Status hues: system green/orange/red for the built-in themes, the
    /// custom theme's own where its file gives them.
    static var success: Color { ThemeManager.spec.success }
    static var warning: Color { ThemeManager.spec.warning }
    static var danger: Color { ThemeManager.spec.danger }

    /// The theme's glass tint, washed over capsules, sheets and the window.
    static var glassTint: Color { ThemeManager.spec.surface }

    /// How strongly that tint and the scrims sit on each glass surface: the capsules, the
    /// window, the detail sheet, the floating panels and the veil (see `GlassTokens`).
    static var glass: GlassTokens { ThemeManager.spec.glass }

    static var hover: Animation { .easeOut(duration: 0.1) }

    static var panelSlide: Animation {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            ? .easeOut(duration: 0.18)
            : .spring(response: 0.32, dampingFraction: 0.9)
    }

    /// The motion of a settling row's glide and of the rows making room for it, so they
    /// land together.
    static let glideSpring = Spring(response: 0.6, dampingRatio: 0.86)

    /// The rows making room as a thread settles to the bottom of the sidebar or comes back
    /// up: the glide's own spring, so the ghost and the rows land together.
    static var settleFlight: Animation {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            ? .easeOut(duration: 0.2)
            : .spring(glideSpring)
    }

}

// MARK: - Symbol fonts

extension View {
    /// `Chrome.inlineIconFont` at the window's text size, read from the environment.
    func chromeInlineIconFont() -> some View {
        modifier(ChromeSymbolFontModifier(isChevron: false))
    }

    /// `Chrome.chevronFont` at the window's text size, read from the environment.
    func chromeChevronFont() -> some View {
        modifier(ChromeSymbolFontModifier(isChevron: true))
    }
}

private struct ChromeSymbolFontModifier: ViewModifier {
    let isChevron: Bool
    @Environment(\.chatZoom) private var zoom

    func body(content: Content) -> some View {
        content.font(isChevron ? Chrome.chevronFont(zoom: zoom) : Chrome.inlineIconFont(zoom: zoom))
    }
}

// MARK: - Surfaces

extension View {
    /// Capsule buttons stay Liquid Glass, washed with a tad of the theme. On a glass panel
    /// they draw flat instead (see `isOnGlassPanel`): glass on glass samples the same
    /// pixels twice, once for the panel and once for the button.
    func chromeGlassCapsule() -> some View {
        modifier(ChromeGlassSurface(shape: .capsule))
    }

    /// The capsule for a field rather than a button: the same glass, without the press
    /// response. A field is clicked to place the caret, and interactive glass answered
    /// the click with its press bounce at the same moment the field took focus, so the
    /// capsule flickered on every tap.
    func chromeGlassFieldCapsule() -> some View {
        modifier(ChromeGlassSurface(shape: .capsule, isInteractive: false))
    }

    func chromeGlassCircle() -> some View {
        modifier(ChromeGlassSurface(shape: .circle))
    }

    /// The inset content sheet the detail pane floats on.
    func detailSheet() -> some View {
        modifier(DetailSheetModifier())
    }

    /// A floating panel's surface, shared by every panel that floats over the chat (the
    /// helper's, the usage panel, the sidebar's floating list): one glass surface with the
    /// theme's tint, clipped to `shape`, over a scrim that carries the panel's shadow.
    func floatingPanelSurface<S: Shape>(in shape: S) -> some View {
        modifier(FloatingPanelSurface(shape: shape))
    }
}

/// A control's surface: interactive Liquid Glass, or, inside a glass panel, the panel's
/// flat control fill, a shade brighter under the pointer the way the glass would be.
private struct ChromeGlassSurface: ViewModifier {
    enum Kind { case capsule, circle }
    let shape: Kind
    /// Whether the glass answers a press; off for a field that takes the caret instead.
    var isInteractive = true
    @Environment(\.isOnGlassPanel) private var isOnGlassPanel
    @Environment(\.colorScheme) private var colorScheme
    @State private var isHovered = false

    func body(content: Content) -> some View {
        switch shape {
        case .capsule:
            surface(content, in: Capsule(style: .continuous))
        case .circle:
            surface(content, in: Circle())
        }
    }

    @ViewBuilder
    private func surface<S: InsettableShape>(_ content: Content, in shape: S) -> some View {
        if isOnGlassPanel {
            content
                .background(shape.fill(Chrome.panelControlFill(isDark: colorScheme == .dark, hovered: isHovered)))
                .contentShape(shape)
                .onHover { isHovered = $0 }
                .animation(Chrome.hover, value: isHovered)
        } else if isInteractive {
            content.glassEffect(.regular.tint(Chrome.glassTint.opacity(Chrome.glass.capsuleTintOpacity)).interactive(), in: shape)
        } else {
            content.glassEffect(.regular.tint(Chrome.glassTint.opacity(Chrome.glass.capsuleTintOpacity)), in: shape)
        }
    }
}

private struct DetailSheetModifier: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        let isDark = colorScheme == .dark
        let glass = Chrome.glass
        // Square: the window's own clip rounds the sheet's outer corners with everything
        // else, so the sheet never draws a corner of its own inside the window's.
        content
            .background {
                Rectangle().fill((isDark ? Color.black : Color.white).opacity(glass.sheetScrimOpacity.value(isDark: isDark)))
                Rectangle().fill(Chrome.glassTint.opacity(glass.sheetTintOpacity.value(isDark: isDark)))
            }
    }
}

/// The recipe behind `floatingPanelSurface(in:)`. The glass and its tint are clipped to the
/// shape, and the glass supplies the edge (as in `WindowBackdrop`): a hairline on top of it
/// read as a second ring around the panel whenever its window was key. The scrim sits under
/// the glass and carries the shadow: a plain filled shape, so the shadow is drawn once and
/// kept. A shadow on the whole panel was blurred again with every token the content streamed.
private struct FloatingPanelSurface<S: Shape>: ViewModifier {
    let shape: S
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        let isDark = colorScheme == .dark
        let glass = Chrome.glass
        content
            .background {
                shape
                    .fill(.clear)
                    .glassEffect(.regular, in: shape)
                    .overlay {
                        shape.fill(Chrome.glassTint.opacity(glass.panelTintOpacity.value(isDark: isDark)))
                    }
            }
            .clipShape(shape)
            .background {
                shape
                    .fill((isDark ? Color.black : Color.white).opacity(glass.panelScrimOpacity.value(isDark: isDark)))
                    .shadow(
                        color: .black.opacity(glass.panelShadowOpacity.value(isDark: isDark)),
                        radius: glass.panelShadowRadius,
                        y: glass.panelShadowOffset
                    )
            }
    }
}

/// One shaped surface for the whole window: glass or a solid fill, a legibility tint and a hairline.
struct WindowBackdrop: View {
    @Environment(\.colorScheme) private var colorScheme
    /// The backdrop setting, 0 for bare glass and 1 for a solid base.
    let opacity: Double
    /// The user's wallpaper, painted edge to edge at the transparency setting's own opacity; nil for the stock glass.
    var wallpaper: NSImage? = nil

    /// The scrim over the glass for a backdrop setting: the theme's window scrim at the
    /// midpoint, none at 0, a solid base at 1, linear either side.
    static func scrim(for opacity: Double, isDark: Bool) -> Double {
        let stock = Chrome.glass.windowScrimOpacity.value(isDark: isDark)
        let t = min(max(opacity, 0), 1)
        return t <= 0.5 ? stock * (t / 0.5) : stock + (1 - stock) * ((t - 0.5) / 0.5)
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous)
        let isDark = colorScheme == .dark
        // At the solid end of the slider the scrim covers the glass completely, and a
        // Liquid Glass surface the size of the window is a full-window sample on every
        // frame anything over it moves. There it draws as a plain fill instead; the
        // scrim, the tint and the hairline stay exactly as they are.
        let isSolid = opacity >= 0.98
        Group {
            if let wallpaper {
                // The picture rides over the desktop rather than in place of it: the
                // transparency setting is the picture's own opacity, so a clear window
                // shows the desktop through the picture and a solid one shows the picture
                // alone. No scrim in this branch: the scrim is the window's base colour,
                // and the picture is what that base is when one is set.
                shape.fill(.clear)
                    .overlay {
                        Image(nsImage: wallpaper)
                            .resizable()
                            .interpolation(.high)
                            .aspectRatio(contentMode: .fill)
                            .opacity(opacity)
                    }
                    .clipShape(shape)
                    .overlay {
                        if !isSolid {
                            shape
                                .fill(.clear)
                                .glassEffect(in: shape)
                        }
                    }
                    .transition(.opacity)
            } else if isSolid {
                shape.fill(isDark ? Color.black : Color.white)
            } else {
                shape
                    .fill(.clear)
                    .glassEffect(in: shape)
                    .overlay {
                        shape
                            .fill(isDark ? Color.black : Color.white)
                            .opacity(Self.scrim(for: opacity, isDark: isDark))
                    }
            }
        }
        .animation(.easeInOut(duration: 0.3), value: wallpaper != nil)
        .overlay {
            shape.fill(Chrome.glassTint.opacity(Chrome.glass.windowTintOpacity))
        }
        // The hairline sits on the glass as well as on the solid fill, the way Droppy's
        // settings window draws its edge: on a clear window the glass rim alone reads as
        // a second line outside the curve.
        .overlay {
            shape.strokeBorder(Chrome.overlay(0.14), lineWidth: 1)
        }
    }
}

// MARK: - Chrome controls

struct ChromeCapsule<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: 0) {
            content
        }
        .padding(.horizontal, Chrome.iconCapsuleInnerPadding)
        .padding(.vertical, Chrome.capsuleVerticalPadding)
        .chromeGlassCapsule()
        // The capsule is a control wherever it sits: presses on it work the control and
        // never drag the window.
        .background { NoWindowDragArea() }
    }
}

struct ChromeDivider: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 0.5, style: .continuous)
            .fill(Chrome.primaryText.opacity(0.22))
            .frame(width: 1, height: Chrome.dividerHeight)
            .accessibilityHidden(true)
    }
}

struct ChromeIconLabel: View {
    let symbol: String
    /// An asset drawn in place of the symbol, as a template: the Crew toggle wears the
    /// anchor, so the row says Crew the way the composer button does.
    var asset: String? = nil
    var isEnabled = true
    var isActive = false
    var isHovering = false

    var body: some View {
        Group {
            if let asset {
                Image(asset)
                    .resizable()
                    .renderingMode(.template)
                    .aspectRatio(contentMode: .fit)
                    .frame(width: Chrome.symbolSize, height: Chrome.symbolSize)
            } else {
                Image(systemName: symbol)
                    .font(Chrome.iconFont)
            }
        }
            .foregroundStyle(foreground)
            .frame(width: Chrome.capsuleContentHeight, height: Chrome.capsuleContentHeight)
            .background {
                if isActive {
                    Circle()
                        .fill(Chrome.overlay(0.12))
                        .padding(1)
                }
            }
            .contentShape(.rect)
    }

    private var foreground: Color {
        guard isEnabled else { return Chrome.primaryText.opacity(0.32) }
        if isActive { return Chrome.primaryText }
        return Chrome.primaryText.opacity(isHovering ? 1 : 0.92)
    }
}

struct ChromeIconButton: View {
    let symbol: String
    var asset: String? = nil
    var isEnabled = true
    var isActive = false
    let help: String
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            ChromeIconLabel(symbol: symbol, asset: asset, isEnabled: isEnabled, isActive: isActive, isHovering: isHovering)
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .accessibilityLabel(Text(help))
        .background { NoWindowDragArea() }
    }
}

struct ChromeMenuButton<Content: View>: View {
    let symbol: String
    let help: String
    @ViewBuilder var content: Content

    @State private var isHovering = false
    @State private var isPresented = false

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            ChromeIconLabel(symbol: symbol, isActive: isPresented, isHovering: isHovering)
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .accessibilityLabel(Text(help))
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu { content }
        }
        .background { NoWindowDragArea() }
    }
}

/// A round glass button the height of a chrome capsule.
struct ChromeCircleButton: View {
    let symbol: String
    let help: String
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(Chrome.iconFont)
                .foregroundStyle(Chrome.primaryText.opacity(isHovering ? 1 : 0.92))
                .frame(width: Chrome.capsuleHeight, height: Chrome.capsuleHeight)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .chromeGlassCircle()
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .accessibilityLabel(Text(help))
        .background { NoWindowDragArea() }
    }
}

/// A round glass button that opens a popover menu, such as the permissions picker.
struct ChromeCircleMenu<Content: View>: View {
    let symbol: String
    let help: String
    @ViewBuilder var content: Content

    @State private var isHovering = false
    @State private var isPresented = false

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            Image(systemName: symbol)
                .font(Chrome.iconFont)
                .foregroundStyle(Chrome.primaryText.opacity(isHovering || isPresented ? 1 : 0.92))
                .frame(width: Chrome.capsuleHeight, height: Chrome.capsuleHeight)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .chromeGlassCircle()
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .accessibilityLabel(Text(help))
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu { content }
        }
        .background { NoWindowDragArea() }
    }
}

/// A text button shaped like a chrome capsule that opens a popover, such as the branch picker.
struct ChromeTextMenu<Content: View>: View {
    let symbol: String
    let title: String
    let help: String
    @ViewBuilder var content: Content

    @State private var isHovering = false
    @State private var isPresented = false

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .chromeInlineIconFont()
                Text(verbatim: title)
                    .zoomedFont(size: 12.5, weight: .medium)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .chromeChevronFont()
                    .foregroundStyle(Chrome.secondaryText)
            }
            .foregroundStyle(Chrome.primaryText.opacity(isHovering || isPresented ? 1 : 0.92))
            .padding(.horizontal, Chrome.capsuleHorizontalPadding)
            .frame(height: Chrome.capsuleContentHeight)
            .padding(.vertical, Chrome.capsuleVerticalPadding)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .fixedSize()
        .chromeGlassCapsule()
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu { content }
        }
        .background { NoWindowDragArea() }
    }
}

/// A text button shaped like a chrome capsule, such as the archive page's delete all. A
/// destructive one reads as an ordinary chrome control at rest and turns red under the pointer.
struct ChromeTextButton: View {
    let symbol: String
    let title: String
    let help: String
    var isEnabled = true
    var isDestructive = false
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .chromeInlineIconFont()
                Text(verbatim: title)
                    .zoomedFont(size: 12.5, weight: .medium)
                    .lineLimit(1)
            }
            .foregroundStyle(foreground)
            .padding(.horizontal, Chrome.capsuleHorizontalPadding)
            .frame(height: Chrome.capsuleContentHeight)
            .padding(.vertical, Chrome.capsuleVerticalPadding)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .fixedSize()
        .chromeGlassCapsule()
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .help(help)
        .accessibilityLabel(Text(help))
        .background { NoWindowDragArea() }
    }

    private var foreground: Color {
        guard isEnabled else { return Chrome.primaryText.opacity(0.32) }
        if isDestructive { return Chrome.danger.opacity(isHovering ? 1 : 0.92) }
        return Chrome.primaryText.opacity(isHovering ? 1 : 0.92)
    }
}

// MARK: - Scroll morph

/// Scroll progress for the sticky chrome. Only the veil and the compact title observe it,
/// so scrolling never re-renders the page underneath.
@MainActor
@Observable
final class ChromeScrollModel {
    private(set) var progress: CGFloat = 0

    func update(travel: CGFloat) {
        let raw = (travel - Chrome.titleStartOffset) / (Chrome.titleEndOffset - Chrome.titleStartOffset)
        let clamped = min(max(raw, 0), 1)
        let quantized = (clamped / Chrome.progressQuantum).rounded() * Chrome.progressQuantum
        if quantized != progress { progress = quantized }
    }
}

/// The veil that settles over the top of a pane as its content scrolls under the chrome: a
/// scrim and the theme's tint, fading out downward.
struct PaneTopVeil: View {
    let model: ChromeScrollModel
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let progress = Double(model.progress)
        let isDark = colorScheme == .dark
        let glass = Chrome.glass
        // Lighter than it was: at full scroll the dark scrim read as a black bar across
        // the top of the chat, well past the backdrop's own tone. The theme's surface
        // now carries more of the veil and the black less, so what scrolls under the
        // chrome dims toward the window's colour instead of toward black.
        let scrim = glass.veilScrimOpacity.value(isDark: isDark) + glass.veilScrimRise.value(isDark: isDark) * progress
        let base = isDark ? Color.black : Color.white
        let tint = Chrome.glassTint.opacity(glass.veilTintOpacity.value(isDark: isDark))
        ZStack {
            // Only once content has scrolled under the chrome; at rest there is nothing to cover.
            if progress > 0 {
                // Two gradient fills and nothing else: the scrim settles the content sliding
                // under the chrome toward the scheme's base and the theme's tint colours it.
                // This was liquid glass under a gradient mask, which cost a backdrop sample
                // and an offscreen mask pass on every scrolled frame of every chat (a
                // bottom-anchored conversation keeps the veil fully on); the chrome's own
                // capsules still refract what passes under them.
                Self.fade(base.opacity(scrim))
                Self.fade(tint)
            }
        }
        .opacity(0.08 + 0.92 * progress)
        .frame(height: Chrome.veilHeight)
        .allowsHitTesting(false)
    }

    /// A colour held over the top 28% of the veil and eased out by its bottom edge, the
    /// mask the glass used to wear: a shorter plateau and a mid stop, so the veil reads
    /// as a soft falloff rather than a band with an edge.
    private static func fade(_ color: Color) -> LinearGradient {
        LinearGradient(
            stops: [
                .init(color: color, location: 0),
                .init(color: color, location: 0.28),
                .init(color: color.opacity(0.45), location: 0.62),
                .init(color: color.opacity(0), location: 1),
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }
}

/// The title that fades into the chrome row once the page's own title scrolls away.
struct ChromeCompactTitle: View {
    let title: String
    let model: ChromeScrollModel

    var body: some View {
        let progress = model.progress
        Text(verbatim: title)
            .zoomedFont(size: 15, weight: .semibold)
            .foregroundStyle(Chrome.primaryText)
            .lineLimit(1)
            .opacity(Double(progress))
            .offset(y: (1 - progress) * 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .allowsHitTesting(false)
            .accessibilityHidden(progress < 0.5)
    }
}

// MARK: - Sidebar pieces

struct SidebarSymbol: View {
    let name: String
    var scale: CGFloat = 1

    init(_ name: String, scale: CGFloat = 1) {
        self.name = name
        self.scale = scale
    }

    var body: some View {
        Image(systemName: name)
            .resizable()
            .interpolation(.high)
            .aspectRatio(contentMode: .fit)
            .frame(width: Chrome.symbolSize * scale, height: Chrome.symbolSize * scale)
            .symbolRenderingMode(.monochrome)
    }
}

/// A sidebar glyph on its own: a provider logo or an SF Symbol, with nothing drawn behind it.
struct SidebarIconBadge<Glyph: View>: View {
    @ViewBuilder var glyph: Glyph

    /// The window's text size: the glyph's slot grows with the row's type (see `SidebarRow`).
    @Environment(\.chatZoom) private var zoom

    var body: some View {
        let side = ChatZoom.scaled(Chrome.iconSize, by: zoom)
        glyph
            .foregroundStyle(Chrome.primaryText.opacity(0.85))
            .frame(width: side, height: side)
    }
}

struct SidebarRow<Icon: View, Accessory: View>: View {
    let title: String
    let isSelected: Bool
    let isEmphasized: Bool
    let accessoryWidth: CGFloat
    let action: () -> Void
    let icon: Icon
    let accessory: (Bool) -> Accessory

    /// The window's text size: the row grows with its title (see `Chrome.rowHeight`).
    @Environment(\.chatZoom) private var zoom
    @State private var isHovering = false

    init(
        title: String,
        isSelected: Bool = false,
        isEmphasized: Bool = false,
        accessoryWidth: CGFloat = 0,
        action: @escaping () -> Void,
        @ViewBuilder icon: () -> Icon,
        @ViewBuilder accessory: @escaping (Bool) -> Accessory
    ) {
        self.title = title
        self.isSelected = isSelected
        self.isEmphasized = isEmphasized
        self.accessoryWidth = accessoryWidth
        self.action = action
        self.icon = icon()
        self.accessory = accessory
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Chrome.rowCornerRadius, style: .continuous)
        Button(action: action) {
            HStack(spacing: 8) {
                icon
                    .frame(width: ChatZoom.scaled(Chrome.iconSize, by: zoom), alignment: .center)
                Text(verbatim: title)
                    .zoomedFont(size: 13, weight: isSelected || isEmphasized ? .medium : .regular)
                    .foregroundStyle(Chrome.primaryText.opacity(isSelected ? 1 : 0.92))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 4)
            }
            .padding(.leading, Chrome.rowHorizontalPadding)
            .padding(.trailing, Chrome.rowHorizontalPadding + accessoryWidth)
            .frame(height: ChatZoom.scaled(Chrome.rowHeight, by: zoom))
            .frame(maxWidth: .infinity, alignment: .leading)
            .background { shape.fill(fill).animation(Chrome.hover, value: isSelected) }
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .overlay(alignment: .trailing) {
            accessory(isHovering)
                .padding(.trailing, 6)
        }
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .accessibilityAddTraits(isSelected ? [.isSelected, .isButton] : .isButton)
    }

    private var fill: Color {
        if isSelected { return Chrome.overlay(0.12) }
        if isHovering { return Chrome.overlay(0.06) }
        return .clear
    }
}

extension SidebarRow where Accessory == EmptyView {
    init(
        title: String,
        isSelected: Bool = false,
        action: @escaping () -> Void,
        @ViewBuilder icon: () -> Icon
    ) {
        self.init(title: title, isSelected: isSelected, action: action, icon: icon) { _ in EmptyView() }
    }
}

struct RowAccessoryIcon: View {
    let symbol: String

    init(_ symbol: String) {
        self.symbol = symbol
    }

    var body: some View {
        Image(systemName: symbol)
            .font(Chrome.inlineIconFont)
            .foregroundStyle(Chrome.secondaryText)
            .frame(width: 20, height: 20)
            .contentShape(.rect)
    }
}

struct SidebarSearchField: View {
    @Binding var text: String
    var prompt = "Search"
    /// Bumped to put the caret in the field (the Search threads command); the field never
    /// takes the keyboard on its own.
    var focusRequest = 0
    var onSubmit: () -> Void = {}
    /// Up and down walk the results, one step per press. Last, so a bare trailing closure
    /// still means `onSubmit`. Left out where there is no list under the field, and the
    /// arrows are then the field's own again.
    var onMove: ((Int) -> Void)?

    @FocusState private var isFocused: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .zoomedFont(size: 12, weight: .medium)
                .foregroundStyle(Chrome.secondaryText.opacity(0.85))
            TextField(prompt, text: $text)
                .textFieldStyle(.plain)
                .zoomedFont(size: 13)
                .frame(maxWidth: .infinity)
                .onSubmit(onSubmit)
                .onKeyPress(.upArrow) { move(-1) }
                .onKeyPress(.downArrow) { move(1) }
                .focused($isFocused)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.secondaryText.opacity(0.75))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text("Clear search"))
            }
        }
        .padding(.horizontal, 9)
        .frame(height: 30)
        .background {
            Capsule(style: .continuous).fill(Chrome.overlay(0.07))
        }
        .contentShape(Capsule(style: .continuous))
        // Escape empties the field, the way the chrome's search capsule does.
        .onExitCommand { text = "" }
        .onChange(of: focusRequest) { _, _ in isFocused = true }
    }

    private func move(_ offset: Int) -> KeyPress.Result {
        guard let onMove else { return .ignored }
        onMove(offset)
        return .handled
    }
}

/// A glass search capsule for a pane's chrome row. Fixed size so it sits aligned with the
/// other capsules instead of pushing them or clipping the sheet edge.
struct ChromeSearchField: View {
    static let width: CGFloat = 200

    @Binding var query: String
    var prompt = "Search"

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .zoomedFont(size: 12, weight: .semibold)
                .foregroundStyle(Chrome.secondaryText)
                .accessibilityHidden(true)
            TextField(prompt, text: $query)
                .textFieldStyle(.plain)
                // The capsule is the field's edge; the focus ring flashed around the text
                // alone as the caret landed.
                .focusEffectDisabled()
                .zoomedFont(size: 12.5)
                .foregroundStyle(Chrome.primaryText)
            Button {
                query = ""
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.secondaryText.opacity(0.8))
            }
            .buttonStyle(.plain)
            // Always laid out, so the field's text never shifts when the button comes;
            // invisible and out of the accessibility tree while there is nothing to clear.
            .opacity(query.isEmpty ? 0 : 1)
            .disabled(query.isEmpty)
            .accessibilityHidden(query.isEmpty)
            .accessibilityLabel(Text("Clear search"))
        }
        .padding(.horizontal, 10)
        .frame(width: Self.width, height: Chrome.capsuleContentHeight)
        .padding(.vertical, Chrome.capsuleVerticalPadding)
        .chromeGlassFieldCapsule()
        .fixedSize()
        .onExitCommand { query = "" }
        .background { NoWindowDragArea() }
    }
}

/// A glass capsule showing the current choice; clicking it opens the choices in a native popover.
struct GlassPickerButton<Value: Hashable>: View {
    let options: [(value: Value, title: String)]
    @Binding var selection: Value
    var asset: (Value) -> String? = { _ in nil }
    /// An SF Symbol per value, shown before the title and on each row, for pickers whose
    /// values have no image asset.
    var symbol: (Value) -> String? = { _ in nil }
    var maxWidth: CGFloat? = nil

    @State private var isPresented = false
    @State private var isHovering = false

    var body: some View {
        let title = options.first { $0.value == selection }?.title ?? ""
        Button {
            isPresented.toggle()
        } label: {
            HStack(spacing: 6) {
                if let name = asset(selection) {
                    Image(name)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(width: 14, height: 14)
                }
                if let symbol = symbol(selection) {
                    Image(systemName: symbol)
                        .zoomedFont(size: 12, weight: .medium)
                        .foregroundStyle(Chrome.secondaryText)
                }
                Text(verbatim: title)
                    .zoomedFont(size: 12.5, weight: .medium)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Image(systemName: "chevron.down")
                    .chromeChevronFont()
                    .foregroundStyle(Chrome.secondaryText)
            }
            .foregroundStyle(Chrome.primaryText.opacity(isHovering || isPresented ? 1 : 0.92))
            .padding(.horizontal, Chrome.capsuleHorizontalPadding)
            .frame(height: Chrome.capsuleContentHeight)
            .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .fixedSize(horizontal: maxWidth == nil, vertical: true)
        .chromeGlassCapsule()
        .frame(maxWidth: maxWidth, alignment: .trailing)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu {
                ForEach(Array(options.enumerated()), id: \.offset) { _, option in
                    PopoverItem(option.title, symbol: symbol(option.value), asset: asset(option.value), isChecked: option.value == selection) {
                        selection = option.value
                    }
                }
            }
        }
        .background { NoWindowDragArea() }
    }
}

/// A soft appearance: fades in with a short rise. Opacity and offset are plain layer
/// properties, so a view in its settled state costs nothing extra; a blur here would leave
/// every timeline row and markdown block with a filter of its own, an offscreen pass each.
struct SoftAppearModifier: ViewModifier {
    let isVisible: Bool

    func body(content: Content) -> some View {
        content
            .opacity(isVisible ? 1 : 0)
            .offset(y: isVisible ? 0 : 4)
    }
}

extension AnyTransition {
    /// Arrives softly; leaves with a quick fade so removals never drag.
    static var softAppear: AnyTransition {
        .asymmetric(
            insertion: .modifier(active: SoftAppearModifier(isVisible: false), identity: SoftAppearModifier(isVisible: true))
                .animation(.softAppear),
            removal: .opacity.animation(.easeOut(duration: 0.12))
        )
    }
}

extension Animation {
    /// The timing every soft appearance shares.
    static var softAppear: Animation { .smooth(duration: 0.32) }
}

/// A departing badge's face along its glide: `progress` runs from 0, in place, to 1, gone.
/// The face holds through the first half of the flight and fades over the second, the way
/// the sidebar's ghost slips out of the list's edge (see `RowGlideAnimator`); with Reduce
/// Motion on it fades where it stands.
private struct SettleGlideModifier: ViewModifier, Animatable {
    var progress: Double

    var animatableData: Double {
        get { progress }
        set { progress = newValue }
    }

    func body(content: Content) -> some View {
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        let fade = reduceMotion ? progress : Self.smoothstep(0.45, 0.92, progress)
        content
            .opacity(1 - fade)
            .offset(y: reduceMotion ? 0 : 22 * progress)
            .scaleEffect(reduceMotion ? 1 : 1 - 0.04 * progress, anchor: .top)
    }

    private static func smoothstep(_ edge0: Double, _ edge1: Double, _ x: Double) -> Double {
        let t = min(max((x - edge0) / (edge1 - edge0), 0), 1)
        return t * t * (3 - 2 * t)
    }
}

extension AnyTransition {
    /// How a badge whose work is done leaves the timeline: the way a settled chat's row
    /// leaves the sidebar. It glides down a little on the glide spring and fades out on the
    /// way, gone before it would land. It arrives softly, like everything else.
    static var settleGlide: AnyTransition {
        .asymmetric(
            insertion: .modifier(active: SoftAppearModifier(isVisible: false), identity: SoftAppearModifier(isVisible: true))
                .animation(.softAppear),
            removal: .modifier(active: SettleGlideModifier(progress: 1), identity: SettleGlideModifier(progress: 0))
                .animation(.spring(Chrome.glideSpring))
        )
    }
}

// MARK: - Cards

struct ChromeCard<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        // Rows are built as they scroll into view, and the card is a filled shape rather than a
        // clip, so a long page costs neither every row up front nor a mask per card while scrolling.
        LazyVStack(alignment: .leading, spacing: 0) {
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: Chrome.cardCornerRadius, style: .continuous)
                .fill(Chrome.overlay(0.03))
        )
    }
}

struct ChromeSection<Content: View>: View {
    let title: String
    /// Sits right after the title: an info button, a count.
    var accessory: AnyView? = nil
    /// Sits at the header's far right: a control for the whole section, out of its cards.
    var trailing: AnyView? = nil
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: Chrome.sectionHeaderSpacing) {
            HStack(spacing: 4) {
                Text(verbatim: title)
                    .zoomedFont(size: 13, weight: .semibold)
                    .foregroundStyle(.secondary)
                if let accessory { accessory }
                if let trailing {
                    Spacer(minLength: 8)
                    trailing
                }
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct ChromeRow<Control: View>: View {
    let title: String
    var detail: String?
    @ViewBuilder var control: Control

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: title)
                    .zoomedFont(size: 13)
                    .foregroundStyle(Chrome.primaryText)
                if let detail, !detail.isEmpty {
                    Text(verbatim: detail)
                        .zoomedFont(size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 12)
            control
        }
        .padding(.leading, 16)
        .padding(.trailing, Chrome.rowControlTrailingPadding)
        .padding(.vertical, 11)
    }
}

struct ChromeRowDivider: View {
    var body: some View {
        Divider()
            .padding(.leading, 16)
    }
}

// MARK: - Window drag

/// Lets a stretch of chrome move the window, the way the sidebar does in Droppy.
struct WindowDragArea: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowDragView {
        WindowDragView()
    }

    func updateNSView(_ nsView: WindowDragView, context: Context) {}
}

final class WindowDragView: NSView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.performDrag(with: event)
    }

    override func mouseUp(with event: NSEvent) {}
}

/// Marks a chrome control's hit area as never dragging the window.
///
/// The chrome row sits inside the title bar's reach (see `WindowChrome.titlebarHeight`), where
/// AppKit moves the window for any view that lets it. Native controls refuse on their own, but a
/// control SwiftUI renders itself — a drag track, say — lands on the shared hosting view, which
/// lets the window move: dragging such a control moved the window with it. This view sits
/// behind the control's content, so it wins the hit test for presses no narrower control claims and
/// refuses the window drag for them; the presses themselves travel on to SwiftUI, so gestures,
/// popovers and buttons work exactly as before. Empty chrome around the controls has no such view
/// behind it, and still drags the window like a title bar.
struct NoWindowDragArea: NSViewRepresentable {
    func makeNSView(context: Context) -> NoWindowDragView {
        NoWindowDragView(frame: .zero)
    }

    func updateNSView(_ nsView: NoWindowDragView, context: Context) {}
}

final class NoWindowDragView: NSView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// A press here works the control, never the window.
    override var mouseDownCanMoveWindow: Bool { false }

    // The view draws nothing and keeps nothing: it only takes the hit so the window does not
    // drag. Each press travels on to the superview, which routes it to the control under the
    // pointer the way a directly dispatched press would.
    override func mouseDown(with event: NSEvent) { superview?.mouseDown(with: event) }

    override func mouseDragged(with event: NSEvent) { superview?.mouseDragged(with: event) }

    override func mouseUp(with event: NSEvent) { superview?.mouseUp(with: event) }

    override func rightMouseDown(with event: NSEvent) { superview?.rightMouseDown(with: event) }

    override func rightMouseUp(with event: NSEvent) { superview?.rightMouseUp(with: event) }
}
