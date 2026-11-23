// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// An sRGB colour as 0xRRGGBB, with an opacity: how the built-in palettes write their
/// colours (always opaque), and what theme.json's `#RRGGBB` and `#RRGGBBAA` strings are read
/// into (see `CustomTheme`). Plain numbers, so two palettes compare by value and a spec is
/// built from them without parsing anything.
struct ThemeRGB: Hashable, Sendable, ExpressibleByIntegerLiteral {
    let value: UInt32
    /// 0xFF is opaque, the stock colours' only value.
    let alpha: UInt8

    init(_ value: UInt32, alpha: UInt8 = 0xFF) {
        self.value = value & 0xFF_FFFF
        self.alpha = alpha
    }

    init(integerLiteral value: UInt32) {
        self.init(value)
    }

    /// `#RRGGBB`, `#RRGGBBAA` with the last two digits for the opacity, or `#RGB` with each
    /// digit doubled (`#FB0` is `#FFBB00`); the `#` may be left out. Nil for anything else.
    init?(hex text: String) {
        var digits = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if digits.hasPrefix("#") { digits.removeFirst() }
        // ASCII only: `isHexDigit` also passes the fullwidth forms, which the parse refuses.
        guard [3, 6, 8].contains(digits.count),
              digits.allSatisfy({ $0.isASCII && $0.isHexDigit }) else { return nil }
        if digits.count == 3 { digits = digits.map { "\($0)\($0)" }.joined() }
        guard let packed = UInt32(digits, radix: 16) else { return nil }
        if digits.count == 8 {
            self.init(packed >> 8, alpha: UInt8(packed & 0xFF))
        } else {
            self.init(packed)
        }
    }

    /// An AppKit colour as the nearest one of these, resolved in the current drawing
    /// appearance; nil for a colour with no sRGB form (a pattern).
    init?(_ color: NSColor) {
        guard let rgb = color.usingColorSpace(.sRGB) else { return nil }
        func channel(_ component: CGFloat) -> UInt32 {
            UInt32((min(max(component, 0), 1) * 255).rounded())
        }
        self.init(channel(rgb.redComponent) << 16 | channel(rgb.greenComponent) << 8 | channel(rgb.blueComponent), alpha: UInt8(channel(rgb.alphaComponent)))
    }

    var red: UInt8 { UInt8((value >> 16) & 0xFF) }
    var green: UInt8 { UInt8((value >> 8) & 0xFF) }
    var blue: UInt8 { UInt8(value & 0xFF) }
    var opacity: Double { Double(alpha) / 255 }

    var nsColor: NSColor {
        NSColor(srgbRed: CGFloat(red) / 255, green: CGFloat(green) / 255, blue: CGFloat(blue) / 255, alpha: CGFloat(opacity))
    }

    var color: Color {
        Color(.sRGB, red: Double(red) / 255, green: Double(green) / 255, blue: Double(blue) / 255, opacity: opacity)
    }

    /// `#RRGGBB`, the way theme.json writes it, or `#RRGGBBAA` for a colour that is not
    /// opaque.
    var hexString: String {
        alpha == 0xFF ? String(format: "#%06X", value) : String(format: "#%06X%02X", value, alpha)
    }

    /// The colour a fraction of the way to black, at the same opacity.
    func darkened(by fraction: Double) -> ThemeRGB {
        func channel(_ component: UInt8) -> UInt32 {
            UInt32((Double(component) * (1 - fraction)).rounded())
        }
        return ThemeRGB(channel(red) << 16 | channel(green) << 8 | channel(blue), alpha: alpha)
    }
}

/// One side of a theme's colours. A nil hue keeps the system's own; the terminal's colours,
/// left out, are worked out from the surface (see `TerminalPalette`).
struct ThemePalette: Equatable, Sendable {
    var accent: ThemeRGB? = nil
    var surface: ThemeRGB
    var success: ThemeRGB? = nil
    var warning: ThemeRGB? = nil
    var danger: ThemeRGB? = nil
    var terminalBackground: ThemeRGB? = nil
    var terminalForeground: ThemeRGB? = nil
    /// The terminal's sixteen ANSI colours, black to bright white; nil keeps SwiftTerm's own.
    var ansi: [ThemeRGB]? = nil

    /// The surfaces the built-in themes tint the glass with, one per side of the Mac.
    static let darkSurface: ThemeRGB = 0x1E1E20
    static let lightSurface: ThemeRGB = 0xFFFFFF
}

/// A theme's colours on both sides of the Mac's appearance, and the side it keeps to, if any.
/// A theme that keeps to one side holds the same palette on both.
struct ThemeColors: Equatable, Sendable {
    var scheme: ColorScheme?
    var dark: ThemePalette
    var light: ThemePalette
    /// Crew hands' colours by lowercased roster name, in place of the roster's own.
    var crew: [String: ThemeRGB] = [:]
    /// Brand colours in place of the built-in ones.
    var brands = ThemeBrandColors()

    static func following(dark: ThemePalette, light: ThemePalette) -> ThemeColors {
        ThemeColors(scheme: nil, dark: dark, light: light)
    }

    static func fixed(_ scheme: ColorScheme, _ palette: ThemePalette) -> ThemeColors {
        ThemeColors(scheme: scheme, dark: palette, light: palette)
    }

    /// The colours as the views read them. A hue that differs between the sides becomes one
    /// colour that picks its side as it draws, the way the system's own colours do.
    func makeSpec() -> ThemeSpec {
        ThemeSpec(
            scheme: scheme,
            accent: Self.color(dark.accent, light.accent, system: .controlAccentColor),
            surface: Self.color(dark.surface, light.surface),
            success: Self.color(dark.success, light.success, system: .systemGreen) ?? .green,
            warning: Self.color(dark.warning, light.warning, system: .systemOrange) ?? .orange,
            danger: Self.color(dark.danger, light.danger, system: .systemRed) ?? .red,
            terminal: ThemeTerminalColors(
                dark: TerminalPalette(dark, isDark: scheme.map { $0 == .dark } ?? true),
                light: TerminalPalette(light, isDark: scheme.map { $0 == .dark } ?? false)
            ),
            crew: crew,
            brands: brands
        )
    }

    /// One colour when both sides agree, else one that picks its side as it draws.
    private static func color(_ dark: ThemeRGB, _ light: ThemeRGB) -> Color {
        guard dark != light else { return dark.color }
        return Color(nsColor: dynamic(dark.nsColor, light.nsColor))
    }

    /// The same for a hue either side may leave to the system (`system`); nil when both do.
    private static func color(_ dark: ThemeRGB?, _ light: ThemeRGB?, system: NSColor) -> Color? {
        if dark == nil, light == nil { return nil }
        if let dark, dark == light { return dark.color }
        return Color(nsColor: dynamic(dark?.nsColor ?? system, light?.nsColor ?? system))
    }

    private static func dynamic(_ dark: NSColor, _ light: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
        }
    }
}

/// The terminal drawer's colours on each side of the Mac's appearance. The drawer picks the
/// side it shows (see `TerminalAppearance`); a theme that keeps to one side gives both the same.
struct ThemeTerminalColors: Sendable {
    var dark: TerminalPalette
    var light: TerminalPalette

    func palette(isDark: Bool) -> TerminalPalette {
        isDark ? dark : light
    }
}

/// One side's terminal colours.
struct TerminalPalette: Equatable, Sendable {
    var background: ThemeRGB
    var foreground: ThemeRGB
    /// The sixteen ANSI colours, or nil for SwiftTerm's own palette.
    var ansi: [ThemeRGB]?
}

extension TerminalPalette {
    /// A side's terminal colours: the theme's own where it gives them, else a background a
    /// shade deeper than the surface, so the drawer reads as a well under the chat, and the
    /// grey the terminal has always drawn its text in.
    init(_ palette: ThemePalette, isDark: Bool) {
        background = palette.terminalBackground ?? palette.surface.darkened(by: isDark ? 0.33 : 0.015)
        foreground = palette.terminalForeground ?? ThemeRGB(isDark ? 0xE6E6E6 : 0x1F1F1F)
        ansi = palette.ansi
    }
}

/// The colours that say who is at work rather than which theme is on: each provider's brand,
/// which a pair's fused icon and the hands' end of a pair's effort slider wear (see
/// `EffortBrand`), the effort slider's colour at maximum and fast mode's gold. Empty keeps
/// the built-in ones; the custom theme may set any of them (theme.json's `brands`).
struct ThemeBrandColors: Equatable, Sendable {
    /// A provider's colour in place of its brand.
    var providers: [ProviderKind: ThemeRGB] = [:]
    /// The effort slider at maximum, in place of the theme's accent (and of the stock purple
    /// a slider with no brand said wears).
    var maximum: ThemeRGB?
    /// Fast mode, in place of the gold.
    var fast: ThemeRGB?

    /// The keys theme.json's `brands` takes: every provider, then the slider's own two.
    static var keys: [String] {
        ProviderKind.allCases.map(\.rawValue) + ["maximum", "fast"]
    }
}
