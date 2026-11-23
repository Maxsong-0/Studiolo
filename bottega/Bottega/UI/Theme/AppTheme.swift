// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Observation
import SwiftUI

/// Every theme Bottega offers: its own brass on glass (the default), the plain system
/// look, Light and Dark, and one of your own, read from a JSON file (see `CustomThemeStore`).
///
/// The app stays Liquid Glass throughout: a theme tints the glass and recolors the accent
/// and status hues, never paints opaque surfaces.
enum AppTheme: String, Codable, CaseIterable, Identifiable, Sendable {
    /// This build's own look: the System theme's glass and dynamic surface, with a brass
    /// accent in place of the Mac's control colour. The default for a fresh install.
    case bottega
    case system
    case light
    case dark
    /// The one custom slot: the colours in theme.json, in the app's support folder, read
    /// again whenever the file is saved.
    case custom

    var id: String { rawValue }

    /// A theme only an earlier build had (the named palettes that were dropped) decodes as
    /// the default rather than failing whatever value holds it; the stored setting falls back
    /// the same way (see `AppSettings`).
    init(from decoder: any Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = AppTheme(rawValue: raw) ?? .bottega
    }

    var displayName: String {
        switch self {
        case .system: "System"
        case .bottega: "Bottega"
        case .light: "Light"
        case .dark: "Dark"
        case .custom: CustomThemeSlot.shared.theme?.name ?? "Custom"
        }
    }

    /// Short picker subtitle, e.g. "Light · System accent".
    var detail: String {
        switch self {
        case .system: "Follows your Mac"
        case .bottega: "Brass on glass, follows your Mac"
        case .light: "Light · System accent"
        case .dark: "Dark · System accent"
        case .custom: CustomThemeSlot.shared.theme?.detail ?? "Your own colours, from theme.json"
        }
    }

    /// Each side's colours. The custom slot shows the brass look until its file has been read.
    var colors: ThemeColors {
        switch self {
        case .bottega:
            .following(
                dark: ThemePalette(accent: Self.brass, surface: ThemePalette.darkSurface),
                light: ThemePalette(accent: Self.brass, surface: ThemePalette.lightSurface)
            )
        case .system:
            .following(dark: ThemePalette(surface: ThemePalette.darkSurface), light: ThemePalette(surface: ThemePalette.lightSurface))
        case .light: .fixed(.light, ThemePalette(surface: ThemePalette.lightSurface))
        case .dark: .fixed(.dark, ThemePalette(surface: ThemePalette.darkSurface))
        case .custom: CustomThemeSlot.shared.theme?.colors ?? AppTheme.bottega.colors
        }
    }

    /// The default theme's accent, in place of the Mac's control colour.
    static let brass: ThemeRGB = 0xC8A96A

    /// The scheme the theme fixes; nil for a theme that follows the Mac.
    var scheme: ColorScheme? { colors.scheme }

    /// Whether the theme draws dark while the Mac itself is dark or not.
    func showsDark(macIsDark: Bool) -> Bool {
        scheme.map { $0 == .dark } ?? macIsDark
    }

    /// The themes the menus, the command palette and Next theme go through: the four built
    /// in, and the custom slot once its file has been read (or while it is the theme in use,
    /// so a menu always has a check to show).
    static func available(current: AppTheme? = nil) -> [AppTheme] {
        allCases.filter { $0 != .custom || current == .custom || CustomThemeSlot.shared.theme != nil }
    }

    /// The theme after this one in `themes`, round to the first after the last.
    func next(in themes: [AppTheme]) -> AppTheme {
        guard let index = themes.firstIndex(of: self) else { return themes.first ?? self }
        return themes[(index + 1) % themes.count]
    }

    /// Whether the Mac itself is dark. A theme that keeps to one side sets the app's own
    /// appearance, so the app's effective appearance cannot tell the Mac's any more.
    static var macIsDark: Bool {
        UserDefaults.standard.string(forKey: "AppleInterfaceStyle") == "Dark"
    }

    /// The picker's badge. A theme that follows the Mac splits into the Mac's light and dark
    /// halves with its own accent between them; one that keeps to a side runs from its surface
    /// into its accent, so dark themes read dark and light themes light at a glance.
    @ViewBuilder
    var swatch: some View {
        let colors = colors
        let rim = Circle().strokeBorder(Color.primary.opacity(0.22), lineWidth: 0.5)
        if colors.scheme == nil {
            Circle()
                .fill(LinearGradient(stops: Self.bands(colors), startPoint: .leading, endPoint: .trailing))
                .overlay(rim)
        } else {
            let accent = colors.dark.accent?.color ?? Color.accentColor
            Circle()
                .fill(LinearGradient(
                    stops: [
                        .init(color: colors.dark.surface.color, location: 0),
                        .init(color: colors.dark.surface.color, location: 0.42),
                        .init(color: accent, location: 1),
                    ],
                    startPoint: .topLeading, endPoint: .bottomTrailing
                ))
                .overlay(rim)
        }
    }

    /// Hard-edged bands, left to right: the light side's surface, its accent, the dark side's
    /// accent where it differs, and the dark side's surface.
    private static func bands(_ colors: ThemeColors) -> [Gradient.Stop] {
        var fills = [colors.light.surface.color]
        if let accent = colors.light.accent { fills.append(accent.color) }
        if let accent = colors.dark.accent, accent != colors.light.accent { fills.append(accent.color) }
        fills.append(colors.dark.surface.color)
        let width = 1 / Double(fills.count)
        return fills.enumerated().flatMap { index, fill in
            [
                Gradient.Stop(color: fill, location: Double(index) * width),
                Gradient.Stop(color: fill, location: Double(index + 1) * width),
            ]
        }
    }

    /// The colours as the views read them. The window's root and the terminal ask on every
    /// render, so the built-in themes' specs are built once, and the custom theme's once per
    /// read of its file.
    var spec: ThemeSpec {
        if self == .custom, let custom = CustomThemeSlot.shared.theme { return custom.spec }
        let theme = self == .custom ? AppTheme.bottega : self
        return Self.builtInSpecs[theme] ?? theme.colors.makeSpec()
    }

    private static let builtInSpecs: [AppTheme: ThemeSpec] = Dictionary(
        uniqueKeysWithValues: allCases.filter { $0 != .custom }.map { ($0, $0.colors.makeSpec()) }
    )
}

/// What a theme paints: an optional fixed color scheme, an optional accent
/// (nil keeps the system control accent), the glass tint, and status hues.
struct ThemeSpec: Sendable {
    var scheme: ColorScheme?
    var accent: Color?
    var surface: Color
    /// How strongly `surface` and the scrims sit on each glass surface: the stock strengths
    /// unless the theme sets its own (see `GlassTokens`).
    var glass: GlassTokens = .standard
    var success: Color
    var warning: Color
    var danger: Color
    /// The terminal drawer's colours, for either side of the Mac's appearance.
    var terminal: ThemeTerminalColors
    /// Crew hands' colours by lowercased roster name, in place of the roster's own; empty
    /// keeps the roster's. Handed to the roster on every theme change (see `ThemeManager`).
    var crew: [String: ThemeRGB] = [:]
    /// Brand colours in place of the built-in ones; empty keeps them (see `ThemeBrandColors`).
    var brands = ThemeBrandColors()
}

/// The theme every static `Chrome` color reads. Mirrored from `AppSettings.theme`, which is
/// the source of truth. The spec is observed as an `@Observable` model's property is: a view
/// whose body reads a theme colour (through `Chrome` or `spec`) draws again when the colours
/// change, whether the theme was switched or the custom theme's file was saved under it,
/// which changes no setting any view observes. Equatable rows that skip their body for
/// unchanged inputs follow too, their dependency being their own.
enum ThemeManager {
    /// Posted after the theme changes, for layer-backed views whose colours are set once
    /// rather than resolved on every draw (the spinners' cells), so they follow the theme
    /// even when nothing re-renders them.
    static let didChange = Notification.Name("ThemeManager.didChange")

    nonisolated(unsafe) static var current: AppTheme = .bottega {
        didSet { refresh() }
    }

    /// The current theme's colors, built once per theme change. `Chrome.accent` and friends
    /// are read in hundreds of view bodies per frame, so the spec is kept rather than made
    /// again on every read; a read inside a body records the view's dependency on it.
    static var spec: ThemeSpec {
        ThemeObservation.shared.access()
        return storedSpec
    }

    nonisolated(unsafe) private static var storedSpec: ThemeSpec = AppTheme.bottega.spec

    /// Takes the current theme's colours again and hands them on: to every view that read
    /// them, to the Crew roster, whose hands a custom theme may recolour, and through
    /// `didChange` to the layer-backed views. Run by a theme change, and by the custom
    /// theme's file being saved while it is in use, which changes the colours without
    /// changing `current`.
    static func refresh() {
        let fresh = current.spec
        ThemeObservation.shared.change { storedSpec = fresh }
        CrewRoster.setColors(fresh.crew.mapValues(\.value))
        NotificationCenter.default.post(name: didChange, object: nil)
    }
}

/// What makes `ThemeManager.spec` observable: a static cannot take part in Observation on
/// its own, so its reads and writes are registered against this one object.
private final class ThemeObservation: Observable, Sendable {
    static let shared = ThemeObservation()

    private let registrar = ObservationRegistrar()

    /// Stands for the spec in the registrar; never read for its value.
    var colors: Int { 0 }

    func access() {
        registrar.access(self, keyPath: \.colors)
    }

    func change(_ mutation: () -> Void) {
        registrar.withMutation(of: self, keyPath: \.colors, mutation)
    }
}
