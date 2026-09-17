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
    static let iconFont: Font = .system(size: 12, weight: .semibold)
    static let inlineIconFont: Font = .system(size: 11, weight: .semibold)
    static let chevronFont: Font = .system(size: 8, weight: .bold)
    static let chromeHorizontalPadding: CGFloat = 14
    static let chromeTopPadding: CGFloat = 12
    static let capsuleContentHeight: CGFloat = 28
    static let capsuleVerticalPadding: CGFloat = 2
    static let capsuleHorizontalPadding: CGFloat = 12
    static let iconCapsuleInnerPadding: CGFloat = 3
    static let dividerHeight: CGFloat = 14
    static var capsuleHeight: CGFloat { capsuleContentHeight + capsuleVerticalPadding * 2 }
    static let contentBreathBelowChrome: CGFloat = 28
    static var contentTopInset: CGFloat { chromeTopPadding + capsuleHeight + contentBreathBelowChrome }
    static let veilHeight: CGFloat = 72
    static let titleStartOffset: CGFloat = 10
    static let titleEndOffset: CGFloat = 44
    static let progressQuantum: CGFloat = 0.04

    // MARK: Sidebar

    static let rowHeight: CGFloat = 28
    static let rowCornerRadius: CGFloat = 7
    static let rowHorizontalPadding: CGFloat = 8
    static let iconSize: CGFloat = 20
    static let iconCornerRadius: CGFloat = 5
    static let symbolSize: CGFloat = 14
    static let groupGap: CGFloat = 10
    static let listInset: CGFloat = 10

    // MARK: Cards

    static let cardCornerRadius: CGFloat = 16
    static let sectionSpacing: CGFloat = 20
    static let sectionHeaderSpacing: CGFloat = 10
    static let contentHorizontalPadding: CGFloat = 16
    /// The gap between a row's trailing control and the card edge.
    static let rowControlTrailingPadding: CGFloat = 10

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

    /// Status hues: system green/orange/red for System/Light/Dark, the
    /// palette's own hues for every named theme.
    static var success: Color { ThemeManager.spec.success }
    static var warning: Color { ThemeManager.spec.warning }
    static var danger: Color { ThemeManager.spec.danger }

    /// The theme's glass tint, washed over capsules, sheets and the window.
    static var glassTint: Color { ThemeManager.spec.surface }

    static var hover: Animation { .easeOut(duration: 0.1) }

    static var panelSlide: Animation {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            ? .easeOut(duration: 0.18)
            : .spring(response: 0.32, dampingFraction: 0.9)
    }

    /// The rows making room as a thread settles to the bottom of the sidebar or comes back
    /// up: the glide's own spring, so the ghost and the rows land together.
    static var settleFlight: Animation {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            ? .easeOut(duration: 0.2)
            : .spring(RowGlideAnimator.spring)
    }

    static let gray = Color(red: 0.556, green: 0.557, blue: 0.576)
    static let blue = Color(red: 0.040, green: 0.478, blue: 1.000)
    static let orange = Color(red: 1.000, green: 0.584, blue: 0.000)

    /// The section tile hues Droppy's settings sidebar uses.
    static let tileHues: [Color] = [
        Color(red: 0.345, green: 0.337, blue: 0.839),
        Color(red: 1.000, green: 0.584, blue: 0.000),
        Color(red: 0.188, green: 0.690, blue: 0.780),
        Color(red: 0.686, green: 0.322, blue: 0.871),
        Color(red: 0.204, green: 0.780, blue: 0.349),
        Color(red: 0.040, green: 0.478, blue: 1.000),
        Color(red: 0.925, green: 0.282, blue: 0.600),
        Color(red: 0.800, green: 0.565, blue: 0.145),
        Color(red: 0.353, green: 0.400, blue: 0.459),
    ]

    static func tileHue(for id: UUID) -> Color {
        let total = withUnsafeBytes(of: id.uuid) { bytes in
            bytes.reduce(0) { sum, byte in sum &+ Int(byte) }
        }
        return tileHues[total % tileHues.count]
    }
}

extension Color {
    func darkened(by fraction: Double) -> Color {
        let color = NSColor(self).usingColorSpace(.sRGB) ?? NSColor(self)
        var hue: CGFloat = 0, saturation: CGFloat = 0, brightness: CGFloat = 0, alpha: CGFloat = 0
        color.getHue(&hue, saturation: &saturation, brightness: &brightness, alpha: &alpha)
        let scaled = max(0, min(1, brightness * CGFloat(1 - fraction)))
        return Color(hue: hue, saturation: saturation, brightness: scaled, opacity: alpha)
    }

    func lightened(by fraction: Double) -> Color {
        let color = NSColor(self).usingColorSpace(.sRGB) ?? NSColor(self)
        var hue: CGFloat = 0, saturation: CGFloat = 0, brightness: CGFloat = 0, alpha: CGFloat = 0
        color.getHue(&hue, saturation: &saturation, brightness: &brightness, alpha: &alpha)
        let clamped = CGFloat(max(0, min(1, fraction)))
        return Color(
            hue: hue,
            saturation: saturation * (1 - clamped * 0.35),
            brightness: brightness + (1 - brightness) * clamped,
            opacity: alpha
        )
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

    func chromeGlassCircle() -> some View {
        modifier(ChromeGlassSurface(shape: .circle))
    }

    /// The inset content sheet the detail pane floats on.
    func detailSheet() -> some View {
        modifier(DetailSheetModifier())
    }
}

/// A control's surface: interactive Liquid Glass, or, inside a glass panel, the panel's
/// flat control fill, a shade brighter under the pointer the way the glass would be.
private struct ChromeGlassSurface: ViewModifier {
    enum Kind { case capsule, circle }
    let shape: Kind
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
        } else {
            content.glassEffect(.regular.tint(Chrome.glassTint.opacity(0.3)).interactive(), in: shape)
        }
    }
}

private struct DetailSheetModifier: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        // Square: the window's own clip rounds the sheet's outer corners with everything
        // else, so the sheet never draws a corner of its own inside the window's.
        content
            .background {
                Rectangle().fill(colorScheme == .dark ? Color.black.opacity(0.22) : Color.white.opacity(0.3))
                Rectangle().fill(Chrome.glassTint.opacity(colorScheme == .dark ? 0.22 : 0.16))
            }
    }
}

/// One shaped surface for the whole window: glass or a solid fill, a legibility tint and a hairline.
struct WindowBackdrop: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(AppModel.self) private var model

    /// The scrim over the glass for a backdrop setting: the stock scrim at the
    /// midpoint, none at 0, a solid base at 1, linear either side.
    static func scrim(for opacity: Double, isDark: Bool) -> Double {
        let stock = isDark ? 0.26 : 0.18
        let t = min(max(opacity, 0), 1)
        return t <= 0.5 ? stock * (t / 0.5) : stock + (1 - stock) * ((t - 0.5) / 0.5)
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous)
        let isDark = colorScheme == .dark
        let opacity = model.settings.backdropOpacity
        // At the solid end of the slider the scrim covers the glass completely, and a
        // Liquid Glass surface the size of the window is a full-window sample on every
        // frame anything over it moves. There it draws as a plain fill instead; the
        // scrim, the tint and the hairline stay exactly as they are.
        let isSolid = opacity >= 0.98
        Group {
            if isSolid {
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
        .overlay {
            shape.fill(Chrome.glassTint.opacity(0.12))
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
    var isEnabled = true
    var isActive = false
    var isHovering = false

    var body: some View {
        Image(systemName: symbol)
            .font(Chrome.iconFont)
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
    var isEnabled = true
    var isActive = false
    let help: String
    let action: () -> Void

    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            ChromeIconLabel(symbol: symbol, isEnabled: isEnabled, isActive: isActive, isHovering: isHovering)
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
                    .font(Chrome.inlineIconFont)
                Text(verbatim: title)
                    .font(.system(size: 12.5, weight: .medium))
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(Chrome.chevronFont)
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
                    .font(Chrome.inlineIconFont)
                Text(verbatim: title)
                    .font(.system(size: 12.5, weight: .medium))
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
        // Lighter than it was: at full scroll the dark scrim read as a black bar across
        // the top of the chat, well past the backdrop's own tone. The theme's surface
        // now carries more of the veil and the black less, so what scrolls under the
        // chrome dims toward the window's colour instead of toward black.
        let scrim = isDark ? 0.14 + 0.18 * progress : 0.30 + 0.26 * progress
        let base = isDark ? Color.black : Color.white
        let tint = Chrome.glassTint.opacity(isDark ? 0.30 : 0.18)
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
            .font(.system(size: 15, weight: .semibold))
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

    var body: some View {
        glyph
            .foregroundStyle(Chrome.primaryText.opacity(0.85))
            .frame(width: Chrome.iconSize, height: Chrome.iconSize)
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
                    .frame(width: Chrome.iconSize, alignment: .center)
                Text(verbatim: title)
                    .font(.system(size: 13, weight: isSelected || isEmphasized ? .medium : .regular))
                    .foregroundStyle(Chrome.primaryText.opacity(isSelected ? 1 : 0.92))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 4)
            }
            .padding(.leading, Chrome.rowHorizontalPadding)
            .padding(.trailing, Chrome.rowHorizontalPadding + accessoryWidth)
            .frame(height: Chrome.rowHeight)
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
    var onSubmit: () -> Void = {}
    /// Up and down walk the results, one step per press. Last, so a bare trailing closure
    /// still means `onSubmit`. Left out where there is no list under the field, and the
    /// arrows are then the field's own again.
    var onMove: ((Int) -> Void)?

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Chrome.secondaryText.opacity(0.85))
            TextField(prompt, text: $text)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .frame(maxWidth: .infinity)
                .onSubmit(onSubmit)
                .onKeyPress(.upArrow) { move(-1) }
                .onKeyPress(.downArrow) { move(1) }
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 12))
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
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Chrome.secondaryText)
                .accessibilityHidden(true)
            TextField(prompt, text: $query)
                .textFieldStyle(.plain)
                .font(.system(size: 12.5))
                .foregroundStyle(Chrome.primaryText)
            Button {
                query = ""
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 12))
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
        .chromeGlassCapsule()
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
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Chrome.secondaryText)
                }
                Text(verbatim: title)
                    .font(.system(size: 12.5, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Image(systemName: "chevron.down")
                    .font(Chrome.chevronFont)
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

    static var searchResult: AnyTransition { softAppear }
}

extension Animation {
    /// The timing every soft appearance shares.
    static var softAppear: Animation { .smooth(duration: 0.32) }
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
    var accessory: AnyView? = nil
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: Chrome.sectionHeaderSpacing) {
            HStack(spacing: 4) {
                Text(verbatim: title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.secondary)
                if let accessory { accessory }
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
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.primaryText)
                if let detail, !detail.isEmpty {
                    Text(verbatim: detail)
                        .font(.system(size: 11))
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
