// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The photo behind the window and how much of it the glass lets through (Vetro, ADR-047).
/// Kept on this Mac only; nothing here is sent anywhere.
@Observable
final class Look {
    enum Photo: String, CaseIterable, Identifiable {
        case rings, coast, shore, forest, aurora
        case window, comet, sun, eclipse, meadow, hills, dunes

        var id: Self { self }

        var title: String {
            switch self {
            case .rings: "Rings"
            case .coast: "Coast"
            case .shore: "Shore"
            case .forest: "Forest"
            case .aurora: "Aurora"
            case .window: "Window"
            case .comet: "Comet"
            case .sun: "Sun"
            case .eclipse: "Eclipse"
            case .meadow: "Meadow"
            case .hills: "Hills"
            case .dunes: "Dunes"
            }
        }

        var asset: String { "backdrop-" + rawValue }

        /// Light enough for dark text; the rest take white text.
        var isLight: Bool {
            switch self {
            case .forest, .hills, .dunes: true
            case .rings, .coast, .shore, .aurora, .window, .comet, .sun, .eclipse, .meadow: false
            }
        }

        /// How light the window is with this photo behind fully clear glass, 0 black to 1 white: the mean of its
        /// pixels, measured 2026-11-28 with the blur and grain as they start. Glass tinted against the photo is
        /// laid over this much light to tell which colour of text can be read on it (T-254). A photo added later
        /// is measured the same way; a light one comes out above a half, and the tests hold it to that.
        var lightness: Double {
            switch self {
            case .rings: 0.32
            case .coast: 0.44
            case .shore: 0.38
            case .forest: 0.66
            case .aurora: 0.39
            case .window: 0.49
            case .comet: 0.34
            case .sun: 0.32
            case .eclipse: 0.28
            case .meadow: 0.33
            case .hills: 0.63
            case .dunes: 0.72
            }
        }

        /// Which part of the photo stays in view when the window is not the photo's shape: 0 its top or left, 1 its
        /// bottom or right. Hills is a tall photo with its moon and cloud above the middle; the other tall one,
        /// Meadow, has its horizon in the middle.
        var focus: UnitPoint {
            switch self {
            case .hills: UnitPoint(x: 0.5, y: 0.3)
            case .rings, .coast, .shore, .forest, .aurora, .window, .comet, .sun, .eclipse, .meadow, .dunes: .center
            }
        }

        /// Where along itself a photo `content` long is pinned to the middle of a frame `frame` long that it
        /// overflows, so that `focus` of what does not fit is cut off before it and the rest after.
        static func guide(content: Double, frame: Double, focus: Double) -> Double {
            frame / 2 + (content - frame) * focus
        }
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

    /// What a Mac that has never opened the backdrop panel shows (the user's own settings, 2026-11-27).
    static let startingPhoto = Photo.coast
    static let startingBlur = 3.0
    static let startingGrain = 31.0
    static let startingTransparency = 97.0

    var photo: Photo { didSet { save(photo.rawValue, Key.photo) } }
    var appearance: Appearance { didSet { save(appearance.rawValue, Key.appearance) } }
    /// How blurred the photo is, in points.
    var blur: Double { didSet { save(blur, Key.blur) } }
    /// Film grain over the photo, 0 to 100.
    var grain: Double { didSet { save(grain, Key.grain) } }
    /// 100 is clear glass, only its edge and shadow left; 0 is solid.
    var transparency: Double { didSet { save(transparency, Key.transparency) } }

    /// The Mac's Increase Contrast and Reduce Transparency. They are the system's settings, not this app's: the
    /// window keeps them in step (`mirrorsAccessibility`) and nothing here saves them. Every line, outline and
    /// pane asks here, so there is one answer to how any of them is drawn with either on (T-254).
    var increasedContrast: Bool
    var reducedTransparency: Bool

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard, increasedContrast: Bool = false, reducedTransparency: Bool = false) {
        self.defaults = defaults
        self.increasedContrast = increasedContrast
        self.reducedTransparency = reducedTransparency
        photo = defaults.string(forKey: Key.photo).flatMap(Photo.init(rawValue:)) ?? Self.startingPhoto
        appearance = defaults.string(forKey: Key.appearance).flatMap(Appearance.init(rawValue:)) ?? .auto
        blur = Self.read(defaults, Key.blur, in: Self.blurRange) ?? Self.startingBlur
        grain = Self.read(defaults, Key.grain, in: Self.percentRange) ?? Self.startingGrain
        transparency = Self.read(defaults, Key.transparency, in: Self.percentRange) ?? Self.startingTransparency
    }

    // MARK: What the rest of the window reads

    /// Which way the glass is tinted, white or near black: as picked, or as the photo is.
    var glassIsDark: Bool {
        switch appearance {
        case .auto: !photo.isLight
        case .light: false
        case .dark: true
        }
    }

    /// The panes drawn all but solid, which either of the two settings asks for.
    var isSolid: Bool { increasedContrast || reducedTransparency }

    /// How light a pane's glass is at full tint, and how much of a solid pane is its own colour.
    static let darkGlassLightness = 0.06
    static let lightGlassLightness = 1.0
    static let solidBody = 0.96

    /// How light what ends up behind the text is, 0 to 1: the photo, with as much of the glass's own colour over
    /// it as the glass carries. Light glass that is nearly clear leaves a dark photo dark.
    var backdropLightness: Double {
        let glass = glassIsDark ? Self.darkGlassLightness : Self.lightGlassLightness
        return photo.lightness + (glass - photo.lightness) * (isSolid ? Self.solidBody : tintOpacity)
    }

    /// White text, and the lines and fills that go with it. Decided by what is behind the text, not by the
    /// appearance alone: Light picked over a dark photo is still dark behind the text while the glass is nearly
    /// clear, and black text there cannot be read (T-254). With enough tint, or solid panes, it follows the glass.
    /// Where it turns depends on the photo: Light over Coast, a middling photo, has black text from 93 % down;
    /// over Eclipse, a dark one, from 82 %. Following the photo, the text always goes with the glass.
    var isDark: Bool { backdropLightness < 0.5 }

    var colorScheme: ColorScheme { isDark ? .dark : .light }

    /// The glass tinted the other way from the photo: dark glass on a light photo, or light glass on a dark one.
    var isAgainstThePhoto: Bool { glassIsDark == photo.isLight }

    /// How much colour the glass carries over the photo (the canvas's `--glo`). Against the photo it needs more
    /// body to keep text apart from it.
    var tintOpacity: Double {
        let base = (100 - transparency) / 100
        return min(1, base * (isAgainstThePhoto ? (glassIsDark ? 1.8 : 1.7) : 1))
    }

    /// Strength of everything glass adds besides its tint (`--gk`): 1 at 82 % and below, 0 when fully clear.
    var strength: Double { min(1, max(0, (100 - transparency) / 18)) }

    var glassTint: Color {
        glassIsDark ? Color(red: 12 / 255, green: 14 / 255, blue: 20 / 255).opacity(tintOpacity) : .white.opacity(tintOpacity)
    }

    /// What a pane is when it is solid.
    var solidPane: Color {
        glassIsDark ? Color(red: 22 / 255, green: 24 / 255, blue: 30 / 255).opacity(Self.solidBody)
            : Color(red: 246 / 255, green: 246 / 255, blue: 248 / 255).opacity(Self.solidBody)
    }

    /// The sidebar sits one shade deeper than the main pane.
    var sidebarShade: Color {
        glassIsDark ? .black.opacity(0.22 * strength) : Color(red: 20 / 255, green: 24 / 255, blue: 34 / 255).opacity(0.08 * strength)
    }

    /// Cards, the inspector, a control: anything outlined that sits on the glass. All of it goes with the glass,
    /// as the canvas's `--surf` does, so that fully clear glass leaves the outline and nothing else. On a solid
    /// pane it is a plain step up from the pane.
    var surface: Color { .white.opacity(surfaceOpacity) }
    var surfaceOpacity: Double {
        if isSolid { return isDark ? 0.07 : 1 }
        return (isDark ? 0.1 : 0.26) * strength
    }

    /// A small label that is only a fill, with no outline ("This Mac"): it keeps a little when the glass is clear,
    /// having nothing else to be seen by (the canvas's `--chip`).
    var chip: Color { isSolid ? ink(isDark ? 0.07 : 0.05) : .white.opacity(chipOpacity) }
    var chipOpacity: Double { isDark ? 0.03 + 0.07 * strength : 0.04 + 0.2 * strength }

    /// The selected row and the switcher's pill.
    var selection: Color { .white.opacity(isDark ? 0.12 + 0.06 * strength : 0.16 + 0.18 * strength) }

    /// The user's own letters.
    var mine: Color { .white.opacity(isDark ? 0.07 + 0.09 * strength : 0.1 + 0.26 * strength) }

    var hover: Color { .white.opacity(isDark ? 0.06 + 0.02 * strength : 0.1 + 0.1 * strength) }

    /// Thin lines between things, around a card, and around a solid pane.
    var line: Color { ink(lineOpacity) }
    var lineOpacity: Double { Self.outline(isDark ? 0.22 : 0.17, increasedContrast: increasedContrast) }

    /// The outline of a control: a capsule, a field, a switcher and its pill, and whatever is picked. Glass that
    /// is fully clear still has its edge, so unlike the fills this does not fade with `strength`.
    var edge: Color { ink(edgeOpacity) }
    var edgeOpacity: Double { Self.outline(isDark ? 0.34 : 0.26, increasedContrast: increasedContrast) }

    /// Around the field being typed in, and around the card or letter that is open in the panel.
    var focusEdge: Color { ink(focusEdgeOpacity) }
    var focusEdgeOpacity: Double { Self.outline(isDark ? 0.62 : 0.5, increasedContrast: increasedContrast) }

    /// How much stronger every line and outline is drawn with Increase Contrast on. One number for all of them:
    /// it takes the lines to about what the canvas has under `prefers-contrast` (0.6 on dark, 0.5 on light), and
    /// keeps an outline plainer than a line, as it is without.
    static let contrastGain = 2.8

    /// A line's or an outline's opacity as it is drawn. The system's own get stronger with Increase Contrast by
    /// themselves; ours are plain colours and have to be told.
    static func outline(_ opacity: Double, increasedContrast: Bool) -> Double {
        increasedContrast ? min(1, opacity * contrastGain) : opacity
    }

    /// White on dark glass, black on light.
    private func ink(_ opacity: Double) -> Color { isDark ? .white.opacity(opacity) : .black.opacity(opacity) }

    /// What a slider has filled so far: the colour of the text, as in the canvas. Not the accent colour, which
    /// here is only for a button to press.
    var mark: Color { ink(0.85) }

    /// The bright inner edge along the top of a surface.
    var rim: Color { .white.opacity(isDark ? 0.26 : 0.5) }

    /// What keeps text apart from the photo: a 1 pt white edge on light glass, a 2 pt drop shadow on dark. No glow.
    var textShadow: (color: Color, radius: CGFloat, y: CGFloat) {
        isDark ? (.black.opacity(0.75 - 0.35 * strength), 1, 1) : (.white.opacity(0.6 - 0.4 * strength), 0.5, 0)
    }

    var grainOpacity: Double { grain * 0.6 / 100 }

    // MARK: The two colours that mean something

    /// Red means one thing in this window: waiting for the user. On dark glass it is a step brighter than the
    /// system's red: the shadow that keeps text apart from the photo eats into small red words (T-254).
    static func waiting(isDark: Bool) -> Color {
        isDark ? Color(red: 1, green: 106 / 255, blue: 96 / 255) : Color(red: 209 / 255, green: 44 / 255, blue: 44 / 255)
    }

    /// Orange: something went wrong, was held back, or is shut (a quota gate). Never red, however bad. Redder
    /// than the system's orange on either glass: that one sits too near the amber of an AI at work.
    static func trouble(isDark: Bool) -> Color {
        isDark ? Color(red: 1, green: 140 / 255, blue: 40 / 255) : Color(red: 204 / 255, green: 85 / 255, blue: 0)
    }

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

/// `.waiting`, wherever a colour goes: `.foregroundStyle(.waiting)`, `Circle().fill(.waiting)`. It is the dark or
/// the light one by the view it is in, which the window sets from `Look.colorScheme`.
struct WaitingRed: ShapeStyle {
    func resolve(in environment: EnvironmentValues) -> Color { Look.waiting(isDark: environment.colorScheme == .dark) }
}

/// `.trouble`, the same way.
struct TroubleOrange: ShapeStyle {
    func resolve(in environment: EnvironmentValues) -> Color { Look.trouble(isDark: environment.colorScheme == .dark) }
}

extension ShapeStyle where Self == WaitingRed {
    static var waiting: WaitingRed { WaitingRed() }
}

extension ShapeStyle where Self == TroubleOrange {
    static var trouble: TroubleOrange { TroubleOrange() }
}
