// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The installed families, and the faces made from them for the conversation's prose, its
/// code and the terminal (see `ChatTypefaces`). A family the font manager does not list (a
/// name stored on another Mac, a font removed since) reads as the system's face: `Font.custom`
/// falls back to it silently and `NSFontManager` hands back nil, so every family is checked
/// against the installed list before a face is made from it. With no family picked, every
/// face here is exactly the system face the views drew before there was a choice.
@MainActor
enum Typefaces {
    /// The installed families, read on first use and again whenever Settings lists them.
    private static var installedFamilies: Set<String>?
    /// The installed families with a fixed-pitch face, for the code font's list.
    private static var fixedPitch: [String]?
    /// Faces made so far, misses included, so a row drawn again never asks the font
    /// manager a second time.
    private static var faces: [FaceKey: NSFont?] = [:]

    private struct FaceKey: Hashable {
        let family: String
        let size: CGFloat
        let weight: CGFloat
        let italic: Bool
    }

    // MARK: - Lists

    /// Every installed family, in the font manager's order, read fresh: Settings lists
    /// them as its picker opens, which is also when a font installed since is noticed.
    static func allFamilies() -> [String] {
        let families = visibleFamilies()
        let installed = Set(families)
        // The fixed-pitch list costs a font made per face, so it is worked out again only
        // when the families have changed, not on every opening of a picker.
        if installed != installedFamilies { fixedPitch = nil }
        installedFamilies = installed
        return families
    }

    /// The families the font manager lists, less the system's private ones (a leading
    /// dot), which no one picks by name.
    private static func visibleFamilies() -> [String] {
        NSFontManager.shared.availableFontFamilies.filter { !$0.hasPrefix(".") }
    }

    /// What Settings lists for a face, read fresh as its picker opens: every installed family
    /// for the prose, the fixed-pitch ones for the code and the terminal.
    static func families(fixedPitch: Bool) -> [String] {
        let all = allFamilies()
        return fixedPitch ? fixedPitchFamilies() : all
    }

    /// The installed families with a fixed-pitch face: what the code font can be.
    static func fixedPitchFamilies() -> [String] {
        if let fixedPitch { return fixedPitch }
        var seen = Set<String>()
        var families: [String] = []
        for name in NSFontManager.shared.availableFontNames(with: .fixedPitchFontMask) ?? [] {
            guard let family = NSFont(name: name, size: 12)?.familyName, !family.hasPrefix("."),
                  seen.insert(family).inserted else { continue }
            families.append(family)
        }
        families.sort { $0.localizedStandardCompare($1) == .orderedAscending }
        fixedPitch = families
        return families
    }

    /// The family when it is installed, else nil: what a stored choice reads as.
    static func installed(_ family: String?) -> String? {
        guard let family, !family.isEmpty else { return nil }
        let families: Set<String>
        if let known = installedFamilies {
            families = known
        } else {
            families = Set(visibleFamilies())
            installedFamilies = families
        }
        return families.contains(family) ? family : nil
    }

    // MARK: - Faces

    /// The family's face at `size`, in the weight asked for or the nearest the family has;
    /// an italic asked of a family without one is its upright face. Nil when the family
    /// has no face at all.
    static func face(family: String, size: CGFloat, weight: NSFont.Weight = .regular, italic: Bool = false) -> NSFont? {
        let key = FaceKey(family: family, size: size, weight: weight.rawValue, italic: italic)
        if let cached = faces[key] { return cached }
        let manager = NSFontManager.shared
        let managerWeight = Self.managerWeight(weight)
        var face = manager.font(withFamily: family, traits: italic ? .italicFontMask : [], weight: managerWeight, size: size)
        if face == nil, italic {
            face = manager.font(withFamily: family, traits: [], weight: managerWeight, size: size)
        }
        // A long session at many sizes and weights never grows this without bound.
        if faces.count > 512 { faces.removeAll(keepingCapacity: true) }
        faces.updateValue(face, forKey: key)
        return face
    }

    /// A SwiftUI font in `family` at `size`: the family's own face when it is installed,
    /// else the system face in `design`, exactly as `Font.system(size:weight:design:)`.
    static func font(family: String?, size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default) -> Font {
        if let family = installed(family), let face = Self.face(family: family, size: size, weight: appKitWeight(weight)) {
            return .custom(face.fontName, fixedSize: size)
        }
        return .system(size: size, weight: weight, design: design)
    }

    /// The prose face for AppKit text: the family's when installed, else the system font
    /// the prose was always set in.
    static func proseFont(family: String?, size: CGFloat) -> NSFont {
        if let family = installed(family), let face = Self.face(family: family, size: size) { return face }
        return .systemFont(ofSize: size)
    }

    /// The prose face at a heavier weight (a heading, a bold run).
    static func proseFont(family: String?, size: CGFloat, weight: NSFont.Weight) -> NSFont {
        if let family = installed(family), let face = Self.face(family: family, size: size, weight: weight) { return face }
        return weight == .bold ? .boldSystemFont(ofSize: size) : .systemFont(ofSize: size, weight: weight)
    }

    /// The prose face's italic: the family's, or its upright face when it has none; the
    /// system font's italic without a family.
    static func proseItalicFont(family: String?, size: CGFloat) -> NSFont {
        if let family = installed(family), let face = Self.face(family: family, size: size, italic: true) { return face }
        return NSFontManager.shared.convert(.systemFont(ofSize: size), toHaveTrait: .italicFontMask)
    }

    /// The code face for AppKit text and the terminal: the family's when installed, else
    /// the system's monospaced face.
    static func codeFont(family: String?, size: CGFloat) -> NSFont {
        if let family = installed(family), let face = Self.face(family: family, size: size) { return face }
        return .monospacedSystemFont(ofSize: size, weight: .regular)
    }

    /// The font manager's 0–15 weight nearest an AppKit weight: 5 is regular, 9 bold.
    private static func managerWeight(_ weight: NSFont.Weight) -> Int {
        switch weight.rawValue {
        case ..<(-0.7): 2
        case ..<(-0.5): 3
        case ..<(-0.2): 4
        case ..<0.1: 5
        case ..<0.26: 6
        case ..<0.35: 8
        case ..<0.5: 9
        case ..<0.6: 10
        default: 11
        }
    }

    /// The AppKit weight a SwiftUI weight names.
    static func appKitWeight(_ weight: Font.Weight) -> NSFont.Weight {
        switch weight {
        case .ultraLight: .ultraLight
        case .thin: .thin
        case .light: .light
        case .medium: .medium
        case .semibold: .semibold
        case .bold: .bold
        case .heavy: .heavy
        case .black: .black
        default: .regular
        }
    }
}
