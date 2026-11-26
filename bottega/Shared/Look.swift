// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The photo behind the window and how much of it the glass lets through (Vetro, ADR-047).
/// Kept on this Mac only; nothing here is sent anywhere.
@Observable
final class Look {
    enum Photo: String, CaseIterable, Identifiable {
        case rings, coast, shore, forest, aurora

        var id: Self { self }

        var title: String {
            switch self {
            case .rings: "Rings"
            case .coast: "Coast"
            case .shore: "Shore"
            case .forest: "Forest"
            case .aurora: "Aurora"
            }
        }

        var asset: String { "backdrop-" + rawValue }

        /// Only the forest is light enough for dark text; the rest take white text.
        var isLight: Bool { self == .forest }
    }

    enum Appearance: String, CaseIterable, Identifiable {
        case auto, light, dark

        var id: Self { self }

        var title: String {
            switch self {
            case .auto: "Match the Photo"
            case .light: "Light"
            case .dark: "Dark"
            }
        }
    }

    static let blurRange: ClosedRange<Double> = 0...48
    static let percentRange: ClosedRange<Double> = 0...100

    var photo: Photo { didSet { save(photo.rawValue, Key.photo) } }
    var appearance: Appearance { didSet { save(appearance.rawValue, Key.appearance) } }
    /// How blurred the photo is, in points.
    var blur: Double { didSet { save(blur, Key.blur) } }
    /// Film grain over the photo, 0 to 100.
    var grain: Double { didSet { save(grain, Key.grain) } }
    /// 100 is clear glass, only its edge and shadow left; 0 is solid.
    var transparency: Double { didSet { save(transparency, Key.transparency) } }

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        photo = defaults.string(forKey: Key.photo).flatMap(Photo.init(rawValue:)) ?? .coast
        appearance = defaults.string(forKey: Key.appearance).flatMap(Appearance.init(rawValue:)) ?? .auto
        blur = Self.read(defaults, Key.blur, in: Self.blurRange) ?? 4
        grain = Self.read(defaults, Key.grain, in: Self.percentRange) ?? 24
        transparency = Self.read(defaults, Key.transparency, in: Self.percentRange) ?? 92
    }

    // MARK: What the rest of the window reads

    var isDark: Bool {
        switch appearance {
        case .auto: !photo.isLight
        case .light: false
        case .dark: true
        }
    }

    var colorScheme: ColorScheme { isDark ? .dark : .light }

    /// How much colour the glass carries over the photo (the canvas's `--glo`).
    var tintOpacity: Double {
        let base = (100 - transparency) / 100
        // Dark glass on a light photo, or light glass on a dark one, needs more body to keep text apart from it.
        let mismatched = isDark == photo.isLight
        return min(1, base * (mismatched ? (isDark ? 1.8 : 1.7) : 1))
    }

    /// Strength of everything glass adds besides its tint (`--gk`): 1 at 82 % and below, 0 when fully clear.
    var strength: Double { min(1, max(0, (100 - transparency) / 18)) }

    var glassTint: Color {
        isDark ? Color(red: 12 / 255, green: 14 / 255, blue: 20 / 255).opacity(tintOpacity) : .white.opacity(tintOpacity)
    }

    /// The sidebar sits one shade deeper than the main pane.
    var sidebarShade: Color {
        isDark ? .black.opacity(0.22 * strength) : Color(red: 20 / 255, green: 24 / 255, blue: 34 / 255).opacity(0.08 * strength)
    }

    /// Cards, the inspector, anything that sits on the glass.
    var surface: Color { .white.opacity(isDark ? 0.03 + 0.07 * strength : 0.04 + 0.2 * strength) }

    /// The selected row and the switcher's pill.
    var selection: Color { .white.opacity(isDark ? 0.12 + 0.06 * strength : 0.16 + 0.18 * strength) }

    var hover: Color { .white.opacity(isDark ? 0.06 + 0.02 * strength : 0.1 + 0.1 * strength) }

    /// Thin lines between things.
    var line: Color { isDark ? .white.opacity(0.16) : .black.opacity(0.12) }

    /// The bright inner edge along the top of a surface.
    var rim: Color { .white.opacity(isDark ? 0.26 : 0.5) }

    /// What keeps text apart from the photo: a 1 pt white edge on light glass, a 2 pt drop shadow on dark. No glow.
    var textShadow: (color: Color, radius: CGFloat, y: CGFloat) {
        isDark ? (.black.opacity(0.75 - 0.35 * strength), 1, 1) : (.white.opacity(0.6 - 0.4 * strength), 0.5, 0)
    }

    var grainOpacity: Double { grain * 0.6 / 100 }

    // MARK: Storage

    private enum Key {
        static let photo = "vetro.photo"
        static let appearance = "vetro.appearance"
        static let blur = "vetro.blur"
        static let grain = "vetro.grain"
        static let transparency = "vetro.transparency"
    }

    private func save(_ value: Any, _ key: String) { defaults.set(value, forKey: key) }

    private static func read(_ defaults: UserDefaults, _ key: String, in range: ClosedRange<Double>) -> Double? {
        guard defaults.object(forKey: key) != nil else { return nil }
        let value = defaults.double(forKey: key)
        return value.isFinite ? min(range.upperBound, max(range.lowerBound, value)) : nil
    }
}
