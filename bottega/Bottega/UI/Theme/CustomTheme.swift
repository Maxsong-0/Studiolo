// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Observation
import Synchronization
import SwiftUI

// MARK: - The file

/// theme.json as written. Every key is optional, and a key set to null counts as left out;
/// `CustomTheme.template` shows the whole shape. A key the file does not take is an error
/// rather than something to skip: a misspelt key would otherwise change nothing without a word.
private struct CustomThemeFile: Decodable {
    /// The terminal drawer's own colours.
    struct Terminal: Decodable {
        var background: String?
        var foreground: String?
        var ansi: [String]?

        private enum CodingKeys: String, CodingKey, CaseIterable {
            case background, foreground, ansi
        }

        init(from decoder: any Decoder) throws {
            try ThemeFileKeys.check(decoder, takes: CodingKeys.allCases.map(\.stringValue), place: "A terminal block")
            let container = try decoder.container(keyedBy: CodingKeys.self)
            background = try container.decodeIfPresent(String.self, forKey: .background)
            foreground = try container.decodeIfPresent(String.self, forKey: .foreground)
            ansi = try container.decodeIfPresent([String].self, forKey: .ansi)
        }
    }

    /// The colours a theme sets: at the top of the file for both sides of the Mac's
    /// appearance, or in a `light` or a `dark` block for that side alone.
    struct Colors: Decodable {
        var accent: String?
        var surface: String?
        var success: String?
        var warning: String?
        var danger: String?
        var terminal: Terminal?

        static let keys = CodingKeys.allCases.map(\.stringValue)

        private enum CodingKeys: String, CodingKey, CaseIterable {
            case accent, surface, success, warning, danger, terminal
        }

        init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            accent = try container.decodeIfPresent(String.self, forKey: .accent)
            surface = try container.decodeIfPresent(String.self, forKey: .surface)
            success = try container.decodeIfPresent(String.self, forKey: .success)
            warning = try container.decodeIfPresent(String.self, forKey: .warning)
            danger = try container.decodeIfPresent(String.self, forKey: .danger)
            terminal = try container.decodeIfPresent(Terminal.self, forKey: .terminal)
        }
    }

    /// A `light` or a `dark` block, which holds colours and nothing else.
    struct Side: Decodable {
        var colors: Colors

        init(from decoder: any Decoder) throws {
            try ThemeFileKeys.check(decoder, takes: Colors.keys, place: "A light or dark block")
            colors = try Colors(from: decoder)
        }
    }

    /// The `glass` block: the strengths of `GlassTokens` the theme changes, the rest kept at
    /// their stock values. Its keys, and those of a strength written as a dark and a light
    /// value, are checked as strictly as the rest of the file.
    struct Glass: Decodable {
        var tokens: GlassTokens

        init(from decoder: any Decoder) throws {
            try ThemeFileKeys.check(decoder, takes: GlassTokens.fileKeys, place: "The glass block")
            let container = try decoder.container(keyedBy: ThemeFileKeys.AnyKey.self)
            for key in container.allKeys {
                let value = try container.superDecoder(forKey: key)
                // One number for both sides needs no check; a malformed value is left to
                // `GlassTokens`, which names it.
                guard (try? value.singleValueContainer().decode(Double.self)) == nil,
                      (try? value.container(keyedBy: ThemeFileKeys.AnyKey.self)) != nil else { continue }
                try ThemeFileKeys.check(value, takes: ["dark", "light"], place: "A glass strength")
            }
            tokens = try GlassTokens(from: decoder)
        }
    }

    var name: String?
    var detail: String?
    var scheme: String?
    var base: Colors
    var light: Colors?
    var dark: Colors?
    /// A null colour keeps the roster's, so the template can name every hand.
    var crew: [String: String?]?
    /// A null colour keeps the built-in one, so the template can name every brand.
    var brands: [String: String?]?
    /// The glass strengths the theme changes (see `GlassTokens`); nil keeps them all.
    var glass: GlassTokens?

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case name, detail, scheme, light, dark, crew, brands, glass
    }

    init(from decoder: any Decoder) throws {
        // `about` is the template's note to whoever edits the file; it is read by no one.
        let keys = CodingKeys.allCases.map(\.stringValue) + Colors.keys + ["about"]
        try ThemeFileKeys.check(decoder, takes: keys, place: "The top of the file")
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        detail = try container.decodeIfPresent(String.self, forKey: .detail)
        scheme = try container.decodeIfPresent(String.self, forKey: .scheme)
        light = try container.decodeIfPresent(Side.self, forKey: .light)?.colors
        dark = try container.decodeIfPresent(Side.self, forKey: .dark)?.colors
        crew = try container.decodeIfPresent([String: String?].self, forKey: .crew)
        brands = try container.decodeIfPresent([String: String?].self, forKey: .brands)
        glass = try container.decodeIfPresent(Glass.self, forKey: .glass)?.tokens
        // The colours for both sides sit in the file's own object, beside the keys above.
        base = try Colors(from: decoder)
    }
}

/// Finds a key an object in theme.json does not take.
private enum ThemeFileKeys {
    /// Any key at all, so the one the file got wrong can be named.
    struct AnyKey: CodingKey {
        let stringValue: String
        var intValue: Int? { nil }

        init(stringValue: String) {
            self.stringValue = stringValue
        }

        init?(intValue: Int) {
            return nil
        }
    }

    static func check(_ decoder: any Decoder, takes keys: [String], place: String) throws {
        let container = try decoder.container(keyedBy: AnyKey.self)
        let unknown = container.allKeys.map(\.stringValue).filter { !keys.contains($0) }.sorted()
        guard let first = unknown.first else { return }
        var codingPath = decoder.codingPath
        codingPath.append(AnyKey(stringValue: first))
        let known = keys.filter { $0 != "about" }.joined(separator: ", ")
        throw CustomTheme.Problem(message: "\(CustomTheme.path(codingPath)) is not a key theme.json knows. \(place) takes \(known)")
    }
}

// MARK: - The theme

/// The custom theme as a read of theme.json made it: its names, its colours and the spec they
/// make, built once per read so a view that shows the theme pays nothing on a render.
struct CustomTheme: Sendable {
    var name: String
    var detail: String
    var colors: ThemeColors
    var spec: ThemeSpec

    /// Why the file could not be used, as a clause naming the key at fault.
    struct Problem: Error {
        let message: String
    }

    /// The largest theme.json read. A theme takes a few kilobytes; anything near this is not one.
    static let byteLimit = 256 * 1024

    /// Reads a theme from the file's bytes, or throws a `Problem`.
    static func decode(_ data: Data) throws -> CustomTheme {
        // The syntax first, so a slip reads the way JSON tools report it, with its line.
        let object: Any
        do {
            object = try JSONSerialization.jsonObject(with: data)
        } catch {
            let detail = ((error as NSError).userInfo[NSDebugDescriptionErrorKey] as? String)?
                .trimmingCharacters(in: CharacterSet(charactersIn: ". "))
            throw Problem(message: "it is not valid JSON" + (detail.map { " (\($0))" } ?? ""))
        }
        guard object is [String: Any] else {
            throw Problem(message: "it holds no JSON object; a theme is one pair of braces, { … }")
        }
        let file: CustomThemeFile
        do {
            file = try JSONDecoder().decode(CustomThemeFile.self, from: data)
        } catch let problem as Problem {
            throw problem
        } catch {
            throw Problem(message: describe(error))
        }
        // Left out, the theme follows the Mac, as the default theme does.
        let scheme: ColorScheme?
        switch file.scheme?.trimmingCharacters(in: .whitespaces).lowercased() ?? "system" {
        case "dark": scheme = .dark
        case "light": scheme = .light
        case "system": scheme = nil
        case let other: throw Problem(message: "scheme is “\(other)”; it takes dark, light or system")
        }
        let dark = try palette(file.base, file.dark, side: "dark", surface: ThemePalette.darkSurface)
        let light = try palette(file.base, file.light, side: "light", surface: ThemePalette.lightSurface)
        let colors = ThemeColors(
            scheme: scheme,
            // A theme that keeps to one side paints that side on both.
            dark: scheme == .light ? light : dark,
            light: scheme == .dark ? dark : light,
            crew: try crewColors(file.crew),
            brands: try brandColors(file.brands)
        )
        var spec = colors.makeSpec()
        // The strengths the file changes; the built-in themes keep the stock ones.
        spec.glass = file.glass ?? .standard
        return CustomTheme(
            name: file.name.flatMap(nonEmpty) ?? "Custom",
            detail: file.detail.flatMap(nonEmpty) ?? defaultDetail(scheme),
            colors: colors,
            spec: spec
        )
    }

    /// A colour of the file, or a `Problem` naming its key.
    private static func color(_ text: String, _ key: String) throws -> ThemeRGB {
        guard let color = ThemeRGB(hex: text) else {
            throw Problem(message: "\(key) is “\(text)”, not a colour like #C8A96A, #FB0 or #C8A96A80")
        }
        return color
    }

    /// One side's palette: that side's own block over the colours for both, over the surface
    /// the built-in themes use on that side.
    private static func palette(
        _ base: CustomThemeFile.Colors,
        _ block: CustomThemeFile.Colors?,
        side: String,
        surface: ThemeRGB
    ) throws -> ThemePalette {
        func value(_ key: KeyPath<CustomThemeFile.Colors, String?>, _ name: String) throws -> ThemeRGB? {
            if let own = block?[keyPath: key] { return try color(own, "\(side).\(name)") }
            return try base[keyPath: key].map { try color($0, name) }
        }
        func terminal(_ key: KeyPath<CustomThemeFile.Terminal, String?>, _ name: String) throws -> ThemeRGB? {
            if let own = block?.terminal?[keyPath: key] { return try color(own, "\(side).terminal.\(name)") }
            return try base.terminal?[keyPath: key].map { try color($0, "terminal.\(name)") }
        }
        let ownANSI = block?.terminal?.ansi
        let ansiKey = ownANSI != nil ? "\(side).terminal.ansi" : "terminal.ansi"
        let ansi = try (ownANSI ?? base.terminal?.ansi).map { (list: [String]) throws -> [ThemeRGB] in
            guard list.count == 16 else {
                throw Problem(message: "\(ansiKey) has \(list.count) colours; it takes 16, black to bright white")
            }
            return try list.enumerated().map { index, entry in try color(entry, "\(ansiKey)[\(index)]") }
        }
        return ThemePalette(
            accent: try value(\.accent, "accent"),
            surface: try value(\.surface, "surface") ?? surface,
            success: try value(\.success, "success"),
            warning: try value(\.warning, "warning"),
            danger: try value(\.danger, "danger"),
            terminalBackground: try terminal(\.background, "background"),
            terminalForeground: try terminal(\.foreground, "foreground"),
            ansi: ansi
        )
    }

    /// The hands' colours by lowercased roster name. A name off the roster is a mistake worth
    /// saying, not one to skip: a typo would otherwise change nothing without a word.
    private static func crewColors(_ crew: [String: String?]?) throws -> [String: ThemeRGB] {
        var colors: [String: ThemeRGB] = [:]
        var named = Set<String>()
        // In order, so a file with two mistakes names the same one every time.
        for (name, value) in (crew ?? [:]).sorted(by: { $0.key < $1.key }) {
            guard let index = CrewRoster.index(named: name) else {
                let first = CrewRoster.builtInPersonas.first?.name ?? ""
                let last = CrewRoster.builtInPersonas.last?.name ?? ""
                throw Problem(message: "crew names “\(name)”, who is not on the roster (\(first) to \(last))")
            }
            let key = CrewRoster.builtInPersonas[index].name.lowercased()
            guard named.insert(key).inserted else {
                throw Problem(message: "crew names \(CrewRoster.builtInPersonas[index].name) twice")
            }
            // Null keeps the roster's colour.
            guard let value else { continue }
            colors[key] = try color(value, "crew.\(name)")
        }
        return colors
    }

    /// The brand colours: a provider's by its name, the effort slider's own by theirs.
    private static func brandColors(_ brands: [String: String?]?) throws -> ThemeBrandColors {
        var colors = ThemeBrandColors()
        for (key, value) in (brands ?? [:]).sorted(by: { $0.key < $1.key }) {
            let name = key.lowercased()
            let provider = ProviderKind(rawValue: name)
            guard provider != nil || name == "maximum" || name == "fast" else {
                throw Problem(message: "brands names “\(key)”; it takes \(ThemeBrandColors.keys.joined(separator: ", "))")
            }
            // Null keeps the built-in colour.
            guard let value else { continue }
            let rgb = try color(value, "brands.\(key)")
            if let provider {
                colors.providers[provider] = rgb
            } else if name == "maximum" {
                colors.maximum = rgb
            } else {
                colors.fast = rgb
            }
        }
        return colors
    }

    private static func nonEmpty(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func defaultDetail(_ scheme: ColorScheme?) -> String {
        switch scheme {
        case .dark?: "Dark · theme.json"
        case .light?: "Light · theme.json"
        default: "Follows your Mac · theme.json"
        }
    }

    /// A decoding failure as a clause: where in the file, and what was wrong there.
    private static func describe(_ error: any Error) -> String {
        guard let error = error as? DecodingError else { return error.localizedDescription }
        switch error {
        case .dataCorrupted(let context):
            return "\(path(context.codingPath)): \(context.debugDescription)"
        case .typeMismatch(_, let context), .valueNotFound(_, let context):
            return "\(path(context.codingPath)): \(context.debugDescription)"
        case .keyNotFound(let key, let context):
            return "\(path(context.codingPath + [key])) is missing"
        @unknown default:
            return error.localizedDescription
        }
    }

    /// A place in the file as its keys read, `dark.terminal.ansi[3]`.
    static func path(_ codingPath: [any CodingKey]) -> String {
        let path = codingPath
            .map { key in key.intValue.map { "[\($0)]" } ?? key.stringValue }
            .joined(separator: ".")
            .replacingOccurrences(of: ".[", with: "[")
        return path.isEmpty ? "the file" : path
    }

    /// SwiftTerm's own sixteen (`SwiftTerm.Color.terminalAppColors`), the ones the terminal
    /// draws in without a theme's, for the template to spell out.
    private static let stockANSI: [ThemeRGB] = [
        0x000000, 0xC23621, 0x25BC24, 0xADAD27, 0x492EE1, 0xD338D3, 0x33BBC8, 0xCBCCCD,
        0x818383, 0xFC391F, 0x31E722, 0xEAEC23, 0x5833FF, 0xF935F8, 0x14F0F0, 0xE9EBEB,
    ]

    /// The file Create writes: the default theme spelled out in full, following the Mac with
    /// a block for each side, so it changes nothing until it is edited. Every other key is
    /// there too, null where null keeps the built-in colour, and the Crew hands with the
    /// roster's own.
    static var template: String {
        let darkTerminal = TerminalPalette(ThemePalette(surface: ThemePalette.darkSurface), isDark: true)
        let lightTerminal = TerminalPalette(ThemePalette(surface: ThemePalette.lightSurface), isDark: false)
        let ansi = stockANSI.map { "\"\($0.hexString)\"" }.joined(separator: ", ")
        let crew = CrewRoster.builtInPersonas
            .map { "    \"\($0.name)\": \"\(ThemeRGB($0.hex).hexString)\"" }
            .joined(separator: ",\n")
        let brands = ThemeBrandColors.keys
            .map { "    \"\($0)\": null" }
            .joined(separator: ",\n")
        let glass = GlassTokens.templateEntries
            .map { "    \($0)" }
            .joined(separator: ",\n")
        return """
        {
          "about": "Bottega reads this file each time it is saved; Settings › General › Appearance says what it made of it. Colours are #RRGGBB or #RGB, or #RRGGBBAA with an opacity at the end, and a colour left out or null keeps the built-in one. scheme is system (follow the Mac), dark or light. The colours at the top paint both sides of the Mac; a dark and a light block paint that side alone, over the top ones. terminal.ansi takes the terminal's 16 colours, black to bright white. crew recolours the Crew hands by name; brands recolours a provider in a pair's icon and at the hands' end of a pair's effort slider, by provider, and maximum and fast the effort slider at maximum (the accent unless set) and in fast mode (gold unless set). glass sets how strongly each glass surface wears the theme: an opacity from 0 to 1 (a panel's shadow radius and offset in points), one number for both sides of the Mac or a dark and a light one; a strength left out keeps its stock value.",
          "name": "My theme",
          "detail": "Brass on glass, follows your Mac",
          "scheme": "system",
          "accent": "\(AppTheme.brass.hexString)",
          "success": null,
          "warning": null,
          "danger": null,
          "terminal": {
            "ansi": [\(ansi)]
          },
          "dark": {
            "surface": "\(ThemePalette.darkSurface.hexString)",
            "terminal": { "background": "\(darkTerminal.background.hexString)", "foreground": "\(darkTerminal.foreground.hexString)" }
          },
          "light": {
            "surface": "\(ThemePalette.lightSurface.hexString)",
            "terminal": { "background": "\(lightTerminal.background.hexString)", "foreground": "\(lightTerminal.foreground.hexString)" }
          },
          "crew": {
        \(crew)
          },
          "brands": {
        \(brands)
          },
          "glass": {
        \(glass)
          }
        }

        """
    }
}

// MARK: - The slot

/// The custom theme as last read, for any reader on any thread. Observed the way an
/// `@Observable` model is, so a view showing the custom theme (the window's root, the theme
/// picker, the terminal) draws again when the file is saved, though the theme stays `.custom`.
final class CustomThemeSlot: Observable, Sendable {
    static let shared = CustomThemeSlot()

    private let registrar = ObservationRegistrar()
    private let stored = Mutex<CustomTheme?>(nil)

    /// The last good read of theme.json; nil before one succeeds and once the file is gone.
    var theme: CustomTheme? {
        registrar.access(self, keyPath: \.theme)
        return stored.withLock { $0 }
    }

    func update(_ theme: CustomTheme?) {
        registrar.withMutation(of: self, keyPath: \.theme) {
            stored.withLock { $0 = theme }
        }
    }
}
