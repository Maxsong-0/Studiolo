// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import CoreGraphics

/// How strongly each glass surface wears the theme: the tint washed over the glass, the scrim
/// that settles whatever shows through it, a floating panel's shadow. One name per strength,
/// so a theme sets the app's texture as well as its colours (`ThemeSpec.glass`, read through
/// `Chrome.glass`). Every default is the look the app shipped with, and the built-in themes
/// keep them all; a theme file names only the strengths it changes (see `init(from:)`).
struct GlassTokens: Equatable, Sendable {
    /// A strength that differs with the appearance: one value on a dark window, one on a light.
    struct SchemeValue: Equatable, Sendable {
        var dark: Double
        var light: Double

        func value(isDark: Bool) -> Double { isDark ? dark : light }
    }

    // MARK: Chrome controls

    /// The theme's tint on a chrome capsule's or circle's glass.
    var capsuleTintOpacity: Double = 0.3

    // MARK: Window

    /// The theme's tint over the whole window, glass or solid.
    var windowTintOpacity: Double = 0.12
    /// The scrim over the window's glass with the transparency slider at its midpoint; the
    /// slider runs it down to none and up to a solid base (see `WindowBackdrop.scrim`).
    var windowScrimOpacity = SchemeValue(dark: 0.26, light: 0.18)

    // MARK: Detail sheet

    /// The scheme's base under the detail sheet's content: black on a dark window, white on
    /// a light one.
    var sheetScrimOpacity = SchemeValue(dark: 0.22, light: 0.3)
    /// The theme's tint over that base.
    var sheetTintOpacity = SchemeValue(dark: 0.22, light: 0.16)

    // MARK: Floating panels

    /// The theme's tint on a floating panel's glass (see `floatingPanelSurface(in:)`).
    var panelTintOpacity = SchemeValue(dark: 0.22, light: 0.16)
    /// The scrim under the panel's glass, so its text reads over whatever it floats above.
    var panelScrimOpacity = SchemeValue(dark: 0.3, light: 0.34)
    /// The panel's shadow: how dark it is, how far it spreads and how far down it falls.
    var panelShadowOpacity = SchemeValue(dark: 1, light: 0.65)
    var panelShadowRadius: CGFloat = 28
    var panelShadowOffset: CGFloat = 10

    // MARK: Controls on a glass panel

    /// A control drawn flat on a glass panel (see `Chrome.panelControlFill`): how opaque its
    /// fill is, and how far the theme's tint lifts toward white on a dark panel or black on
    /// a light one, at rest and under the pointer.
    var controlFillOpacity = SchemeValue(dark: 0.55, light: 0.42)
    var controlFillLift = SchemeValue(dark: 0.12, light: 0.04)
    var controlFillHoverLift = SchemeValue(dark: 0.24, light: 0.12)

    // MARK: Pane top veil

    /// The veil over content scrolled under the chrome (see `PaneTopVeil`): its scrim as the
    /// first line slips under, what a full scroll adds to that, and the theme's tint over it.
    var veilScrimOpacity = SchemeValue(dark: 0.14, light: 0.30)
    var veilScrimRise = SchemeValue(dark: 0.18, light: 0.26)
    var veilTintOpacity = SchemeValue(dark: 0.30, light: 0.18)

    /// The strengths the app ships with.
    static let standard = GlassTokens()
}

extension GlassTokens: Decodable {
    private enum CodingKeys: String, CodingKey, CaseIterable {
        case capsuleTintOpacity
        case windowTintOpacity, windowScrimOpacity
        case sheetScrimOpacity, sheetTintOpacity
        case panelTintOpacity, panelScrimOpacity, panelShadowOpacity, panelShadowRadius, panelShadowOffset
        case controlFillOpacity, controlFillLift, controlFillHoverLift
        case veilScrimOpacity, veilScrimRise, veilTintOpacity
    }

    /// A theme file names only the strengths it changes; the rest keep their stock values.
    /// An opacity is a number from 0 to 1 (anything outside is pulled back into range), and a
    /// strength that differs with the appearance takes one number for both, or
    /// `{"dark": 0.3, "light": 0.2}` with either side left out.
    init(from decoder: any Decoder) throws {
        self.init()
        let container = try decoder.container(keyedBy: CodingKeys.self)
        func opacity(_ value: inout Double, _ key: CodingKeys) throws {
            if let read = try container.decodeIfPresent(Double.self, forKey: key) { value = Self.unit(read) }
        }
        func pair(_ value: inout SchemeValue, _ key: CodingKeys) throws {
            guard let read = try container.decodeIfPresent(SchemeValueFile.self, forKey: key) else { return }
            if let dark = read.dark { value.dark = Self.unit(dark) }
            if let light = read.light { value.light = Self.unit(light) }
        }
        func length(_ value: inout CGFloat, _ key: CodingKeys) throws {
            if let read = try container.decodeIfPresent(Double.self, forKey: key) { value = CGFloat(read) }
        }
        try opacity(&capsuleTintOpacity, .capsuleTintOpacity)
        try opacity(&windowTintOpacity, .windowTintOpacity)
        try pair(&windowScrimOpacity, .windowScrimOpacity)
        try pair(&sheetScrimOpacity, .sheetScrimOpacity)
        try pair(&sheetTintOpacity, .sheetTintOpacity)
        try pair(&panelTintOpacity, .panelTintOpacity)
        try pair(&panelScrimOpacity, .panelScrimOpacity)
        try pair(&panelShadowOpacity, .panelShadowOpacity)
        try length(&panelShadowRadius, .panelShadowRadius)
        // A blur has no negative radius; an offset may lift the shadow above the panel.
        panelShadowRadius = max(panelShadowRadius, 0)
        try length(&panelShadowOffset, .panelShadowOffset)
        try pair(&controlFillOpacity, .controlFillOpacity)
        try pair(&controlFillLift, .controlFillLift)
        try pair(&controlFillHoverLift, .controlFillHoverLift)
        try pair(&veilScrimOpacity, .veilScrimOpacity)
        try pair(&veilScrimRise, .veilScrimRise)
        try pair(&veilTintOpacity, .veilTintOpacity)
    }

    private static func unit(_ value: Double) -> Double { min(max(value, 0), 1) }

    /// The strengths as theme.json names them, for its check of the keys it takes.
    static let fileKeys = CodingKeys.allCases.map(\.stringValue)

    /// Every strength as theme.json writes it, at its stock value, in the order the file
    /// reads them: the entries of the template's glass block (see `CustomTheme.template`).
    static var templateEntries: [String] {
        CodingKeys.allCases.map { "\"\($0.stringValue)\": \(stockValue($0))" }
    }

    /// One strength's stock value as JSON: a number, or a dark and a light one. A switch,
    /// so a strength added to the keys cannot be left out of the template.
    private static func stockValue(_ key: CodingKeys) -> String {
        let stock = GlassTokens.standard
        func pair(_ value: SchemeValue) -> String { "{ \"dark\": \(value.dark), \"light\": \(value.light) }" }
        switch key {
        case .capsuleTintOpacity: return "\(stock.capsuleTintOpacity)"
        case .windowTintOpacity: return "\(stock.windowTintOpacity)"
        case .windowScrimOpacity: return pair(stock.windowScrimOpacity)
        case .sheetScrimOpacity: return pair(stock.sheetScrimOpacity)
        case .sheetTintOpacity: return pair(stock.sheetTintOpacity)
        case .panelTintOpacity: return pair(stock.panelTintOpacity)
        case .panelScrimOpacity: return pair(stock.panelScrimOpacity)
        case .panelShadowOpacity: return pair(stock.panelShadowOpacity)
        case .panelShadowRadius: return "\(Double(stock.panelShadowRadius))"
        case .panelShadowOffset: return "\(Double(stock.panelShadowOffset))"
        case .controlFillOpacity: return pair(stock.controlFillOpacity)
        case .controlFillLift: return pair(stock.controlFillLift)
        case .controlFillHoverLift: return pair(stock.controlFillHoverLift)
        case .veilScrimOpacity: return pair(stock.veilScrimOpacity)
        case .veilScrimRise: return pair(stock.veilScrimRise)
        case .veilTintOpacity: return pair(stock.veilTintOpacity)
        }
    }

    /// A `SchemeValue` as a theme file writes it: one number for both appearances, or an
    /// object with a `dark` side, a `light` side or both.
    private struct SchemeValueFile: Decodable {
        var dark: Double?
        var light: Double?

        private enum CodingKeys: String, CodingKey {
            case dark, light
        }

        init(from decoder: any Decoder) throws {
            if let single = try? decoder.singleValueContainer(), let both = try? single.decode(Double.self) {
                dark = both
                light = both
                return
            }
            let container = try decoder.container(keyedBy: CodingKeys.self)
            dark = try container.decodeIfPresent(Double.self, forKey: .dark)
            light = try container.decodeIfPresent(Double.self, forKey: .light)
        }
    }
}
