import AppKit
import SwiftUI

/// Every theme Droppy Code offers: the system look plus the palettes the big
/// coding agents ship (Catppuccin, Dracula, Tokyo Night, Nord, Gruvbox,
/// One Dark, Everforest, Kanagawa, Rosé Pine, Solarized, GitHub, Ayu,
/// Night Owl, Monokai, Claude, Codex, Cursor, Matrix).
///
/// Hexes follow the canonical upstream palettes (matching what OpenCode's
/// desktop themes, Gemini CLI's `/theme` list and the Claude Code community
/// themes use), so a theme feels like its namesake. The app stays Liquid
/// Glass throughout: a theme tints the glass and recolors the accent and
/// status hues, never paints opaque surfaces.
enum AppTheme: String, Codable, CaseIterable, Identifiable, Sendable {
    case system
    case light
    case dark
    case catppuccinMocha
    case catppuccinLatte
    case dracula
    case tokyoNight
    case nord
    case gruvbox
    case gruvboxLight
    case oneDark
    case everforest
    case kanagawa
    case rosePine
    case solarizedDark
    case solarizedLight
    case githubDark
    case githubLight
    case ayu
    case nightOwl
    case monokai
    case claude
    case claudeLight
    case codex
    case cursor
    case matrix

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .system: "System"
        case .light: "Light"
        case .dark: "Dark"
        case .catppuccinMocha: "Catppuccin Mocha"
        case .catppuccinLatte: "Catppuccin Latte"
        case .dracula: "Dracula"
        case .tokyoNight: "Tokyo Night"
        case .nord: "Nord"
        case .gruvbox: "Gruvbox"
        case .gruvboxLight: "Gruvbox Light"
        case .oneDark: "One Dark"
        case .everforest: "Everforest"
        case .kanagawa: "Kanagawa"
        case .rosePine: "Rosé Pine"
        case .solarizedDark: "Solarized Dark"
        case .solarizedLight: "Solarized Light"
        case .githubDark: "GitHub Dark"
        case .githubLight: "GitHub Light"
        case .ayu: "Ayu"
        case .nightOwl: "Night Owl"
        case .monokai: "Monokai"
        case .claude: "Claude"
        case .claudeLight: "Claude Light"
        case .codex: "Codex"
        case .cursor: "Cursor"
        case .matrix: "Matrix"
        }
    }

    /// Short picker subtitle, e.g. "Dark · Mauve".
    var detail: String {
        switch self {
        case .system: "Follows your Mac"
        case .light: "Light · System accent"
        case .dark: "Dark · System accent"
        case .catppuccinMocha: "Dark · Mauve"
        case .catppuccinLatte: "Light · Mauve"
        case .dracula: "Dark · Purple"
        case .tokyoNight: "Dark · Blue"
        case .nord: "Dark · Frost"
        case .gruvbox: "Dark · Orange"
        case .gruvboxLight: "Light · Rust"
        case .oneDark: "Dark · Blue"
        case .everforest: "Dark · Green"
        case .kanagawa: "Dark · Wave blue"
        case .rosePine: "Dark · Rose"
        case .solarizedDark: "Dark · Blue"
        case .solarizedLight: "Light · Blue"
        case .githubDark: "Dark · Blue"
        case .githubLight: "Light · Blue"
        case .ayu: "Dark · Amber"
        case .nightOwl: "Dark · Blue"
        case .monokai: "Dark · Pink"
        case .claude: "Dark · Terracotta"
        case .claudeLight: "Light · Terracotta"
        case .codex: "Dark · Magenta"
        case .cursor: "Dark · Frost"
        case .matrix: "Dark · Green"
        }
    }

    /// scheme: nil follows the system. accent/success/warning/danger: nil
    /// keeps the system hue; surface always tints the glass.
    private var palette: (
        scheme: ColorScheme?, accent: String?, surface: String,
        success: String?, warning: String?, danger: String?
    ) {
        switch self {
        case .system: (nil, nil, "1E1E20", nil, nil, nil)
        case .light: (.light, nil, "FFFFFF", nil, nil, nil)
        case .dark: (.dark, nil, "1E1E20", nil, nil, nil)
        case .catppuccinMocha: (.dark, "CBA6F7", "1E1E2E", "A6E3A1", "F9E2AF", "F38BA8")
        case .catppuccinLatte: (.light, "8839EF", "EFF1F5", "40A02B", "DF8E1D", "D20F39")
        case .dracula: (.dark, "BD93F9", "282A36", "50FA7B", "FFB86C", "FF5555")
        case .tokyoNight: (.dark, "7AA2F7", "1A1B26", "9ECE6A", "E0AF68", "F7768E")
        case .nord: (.dark, "88C0D0", "2E3440", "A3BE8C", "EBCB8B", "BF616A")
        case .gruvbox: (.dark, "FE8019", "282828", "B8BB26", "FABD2E", "FB4934")
        case .gruvboxLight: (.light, "AF3A03", "FBF1C7", "79740E", "B57614", "9D0006")
        case .oneDark: (.dark, "61AFEF", "282C34", "98C379", "E5C07B", "E06C75")
        case .everforest: (.dark, "A7C080", "2D353B", "A7C080", "DBBC7F", "E67E80")
        case .kanagawa: (.dark, "7E9CD8", "1F1F28", "98BB6C", "D7A657", "E82424")
        case .rosePine: (.dark, "EB6F92", "191724", "9CCFD8", "F6C177", "EB6F92")
        case .solarizedDark: (.dark, "268BD2", "002B36", "859900", "B58900", "DC322F")
        case .solarizedLight: (.light, "268BD2", "FDF6E3", "859900", "B58900", "DC322F")
        case .githubDark: (.dark, "4493F8", "0D1117", "3FB950", "D29922", "F85149")
        case .githubLight: (.light, "0969DA", "FFFFFF", "1A7F37", "9A6700", "CF222E")
        case .ayu: (.dark, "E6B450", "0F1419", "AAD94C", "FFB454", "F58572")
        case .nightOwl: (.dark, "82AAFF", "011627", "C5E478", "ECC48D", "EF5350")
        case .monokai: (.dark, "F92672", "272822", "A6E22E", "FD971F", "F92672")
        case .claude: (.dark, "C15F3C", "1A1816", "51A556", "C9A227", "D97757")
        case .claudeLight: (.light, "C15F3C", "FAF9F5", "2E7D32", "9A6700", "B3261E")
        case .codex: (.dark, "D946EF", "101014", "3FB950", "D29922", "F85149")
        case .cursor: (.dark, "88C0D0", "181818", "3FA266", "F1B467", "E34671")
        case .matrix: (.dark, "00E676", "000000", "00E676", "FFD600", "FF5252")
        }
    }

    /// The scheme the theme fixes; nil for the system look.
    var scheme: ColorScheme? { palette.scheme }

    /// The picker's badge: a circle running from the theme's surface into
    /// its accent, so dark themes read dark and light themes light at a
    /// glance. System splits into the Mac's light and dark halves.
    @ViewBuilder
    var swatch: some View {
        let palette = palette
        let rim = Circle().strokeBorder(Color.primary.opacity(0.22), lineWidth: 0.5)
        if self == .system {
            Circle()
                .fill(LinearGradient(
                    stops: [
                        .init(color: Color(hex: "FFFFFF"), location: 0), .init(color: Color(hex: "FFFFFF"), location: 0.5),
                        .init(color: Color(hex: "1E1E20"), location: 0.5), .init(color: Color(hex: "1E1E20"), location: 1),
                    ],
                    startPoint: .leading, endPoint: .trailing
                ))
                .overlay(rim)
        } else {
            let accent = palette.accent.map(Color.init(hex:)) ?? Color.accentColor
            Circle()
                .fill(LinearGradient(
                    stops: [
                        .init(color: Color(hex: palette.surface), location: 0),
                        .init(color: Color(hex: palette.surface), location: 0.42),
                        .init(color: accent, location: 1),
                    ],
                    startPoint: .topLeading, endPoint: .bottomTrailing
                ))
                .overlay(rim)
        }
    }

    var spec: ThemeSpec {
        let palette = palette
        let surfaceColor: Color
        if self == .system {
            surfaceColor = Color(nsColor: NSColor(name: nil, dynamicProvider: { appearance in
                let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                return isDark
                    ? NSColor(srgbRed: 0x1E / 255.0, green: 0x1E / 255.0, blue: 0x20 / 255.0, alpha: 1.0)
                    : NSColor(srgbRed: 1.0, green: 1.0, blue: 1.0, alpha: 1.0)
            }))
        } else {
            surfaceColor = Color(hex: palette.surface)
        }
        return ThemeSpec(
            scheme: palette.scheme,
            accent: palette.accent.map(Color.init(hex:)),
            surface: surfaceColor,
            success: palette.success.map(Color.init(hex:)) ?? .green,
            warning: palette.warning.map(Color.init(hex:)) ?? .orange,
            danger: palette.danger.map(Color.init(hex:)) ?? .red
        )
    }
}

/// What a theme paints: an optional fixed color scheme, an optional accent
/// (nil keeps the system control accent), the glass tint, and status hues.
struct ThemeSpec {
    var scheme: ColorScheme?
    var accent: Color?
    var surface: Color
    var success: Color
    var warning: Color
    var danger: Color
}

/// The theme every static `Chrome` color reads. Mirrored from
/// `AppSettings.theme`, which is the source of truth; views re-render
/// through the settings they already observe, so the static read is always
/// fresh on the main thread.
enum ThemeManager {
    /// Posted after the theme changes, for layer-backed views whose colours are set once
    /// rather than resolved on every draw (the spinners' cells), so they follow the theme
    /// even when nothing re-renders them.
    static let didChange = Notification.Name("ThemeManager.didChange")

    nonisolated(unsafe) static var current: AppTheme = .system {
        didSet {
            spec = current.spec
            NotificationCenter.default.post(name: didChange, object: nil)
        }
    }

    /// The current theme's colors, built once per theme change. `Chrome.accent` and friends
    /// are read in hundreds of view bodies per frame, and building a spec parses hex colors,
    /// so the parse happens here rather than on every read.
    nonisolated(unsafe) private(set) static var spec: ThemeSpec = AppTheme.system.spec
}

extension Color {
    /// `"CBA6F7"` or `"#CBA6F7"` in sRGB.
    init(hex: String) {
        var text = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("#") { text = String(text.dropFirst()) }
        var value: UInt64 = 0
        Scanner(string: text).scanHexInt64(&value)
        let red = Double((value >> 16) & 0xFF) / 255
        let green = Double((value >> 8) & 0xFF) / 255
        let blue = Double(value & 0xFF) / 255
        self.init(.sRGB, red: red, green: green, blue: blue, opacity: 1)
    }
}
