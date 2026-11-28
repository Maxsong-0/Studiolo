// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import SwiftUI
import Testing

/// The backdrop and the glass over it (Vetro, ADR-047): what is kept, and how the sliders turn into glass.
@MainActor
struct LookTests {
    private func freshDefaults() -> UserDefaults {
        let name = "LookTests-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    /// What the user set the backdrop panel to, 2026-11-27 (T-245).
    @Test func startsAsTheUserSetIt() {
        let look = Look(defaults: freshDefaults())
        #expect(look.photo == .coast)
        #expect(look.appearance == .auto)
        #expect(look.blur == 3)
        #expect(look.grain == 31)
        #expect(look.transparency == 97)
    }

    @Test func keepsWhatIsChosen() {
        let defaults = freshDefaults()
        let look = Look(defaults: defaults)
        look.photo = .forest
        look.appearance = .dark
        look.blur = 12
        look.grain = 0
        look.transparency = 100
        let again = Look(defaults: defaults)
        #expect(again.photo == .forest)
        #expect(again.appearance == .dark)
        #expect(again.blur == 12)
        #expect(again.grain == 0)
        #expect(again.transparency == 100)
    }

    @Test func readsBackOnlyWhatIsInRange() {
        let defaults = freshDefaults()
        defaults.set(99, forKey: "vetro.blur")
        defaults.set(-5, forKey: "vetro.grain")
        defaults.set(140, forKey: "vetro.transparency")
        defaults.set("mars", forKey: "vetro.photo")
        defaults.set("sepia", forKey: "vetro.appearance")
        let look = Look(defaults: defaults)
        #expect(look.blur == 48)
        #expect(look.grain == 0)
        #expect(look.transparency == 100)
        #expect(look.photo == .coast)
        #expect(look.appearance == .auto)
    }

    /// The glass is tinted as the photo is, or as picked. Left to follow the photo, the text goes the same way,
    /// however clear the glass.
    @Test func theGlassFollowsThePhotoUnlessFixed() {
        let look = Look(defaults: freshDefaults())
        let light: Set<Look.Photo> = [.forest, .hills, .dunes]
        for photo in Look.Photo.allCases {
            look.photo = photo
            for transparency in [0.0, 50, 82, 97, 100] {
                look.transparency = transparency
                #expect(look.glassIsDark == !light.contains(photo), "\(photo)")
                #expect(look.isDark == look.glassIsDark, "\(photo) at \(transparency)")
            }
        }
        look.appearance = .light
        look.photo = .rings
        #expect(!look.glassIsDark)
        look.appearance = .dark
        look.photo = .forest
        #expect(look.glassIsDark)
    }

    /// Light picked over a dark photo: the text is black only once the glass carries enough white to read it on.
    /// Nearly clear glass leaves the photo dark behind the text, and the text white (T-254). Dark over a light
    /// photo, the other way round.
    @Test func textFollowsWhatIsBehindIt() {
        let look = Look(defaults: freshDefaults())
        // Coast is a middling photo: white text over it only while the light glass is nearly clear.
        look.photo = .coast
        look.appearance = .light
        look.transparency = 97
        #expect(!look.glassIsDark)
        #expect(look.isAgainstThePhoto)
        #expect(look.isDark)
        #expect(look.colorScheme == .dark)
        #expect(abs(look.backdropLightness - (0.44 + 0.56 * 0.03 * 1.7)) < 1e-9)
        // The glass is still tinted white, however the text is drawn, and the sidebar is the light glass's shade.
        #expect(look.glassTint == .white.opacity(look.tintOpacity))
        #expect(look.sidebarShade == Color(red: 20 / 255, green: 24 / 255, blue: 34 / 255).opacity(0.08 * look.strength))
        look.transparency = 94
        #expect(look.isDark)
        look.transparency = 93
        #expect(!look.isDark)
        #expect(look.colorScheme == .light)
        // A darker photo keeps its white text further down the slider.
        look.photo = .rings
        #expect(look.isDark)
        look.transparency = 85
        #expect(look.isDark)
        look.transparency = 84
        #expect(!look.isDark)
        look.transparency = 0
        #expect(!look.isDark)
        #expect(abs(look.backdropLightness - 1) < 1e-9)

        look.photo = .forest
        look.appearance = .dark
        look.transparency = 97
        #expect(look.glassIsDark)
        #expect(look.isAgainstThePhoto)
        #expect(!look.isDark)
        look.transparency = 86
        #expect(!look.isDark)
        look.transparency = 85
        #expect(look.isDark)
        look.transparency = 0
        #expect(look.isDark)
        #expect(look.backdropLightness < 0.1)
        // Glass that goes with the photo is not against it, picked or followed.
        look.appearance = .light
        #expect(!look.isAgainstThePhoto)
        look.appearance = .auto
        #expect(!look.isAgainstThePhoto)
    }

    /// How light each photo is was measured; which glass goes with it was picked. The two have to agree, or text
    /// that follows the photo would not be the colour its own glass takes.
    @Test func everyPhotoIsAsLightAsItsGlass() {
        for photo in Look.Photo.allCases {
            #expect(photo.lightness > 0 && photo.lightness < 1, "\(photo)")
            #expect(photo.isLight == (photo.lightness > 0.5), "\(photo)")
        }
        // Coast, the photo the app starts with, is nearer the middle than Eclipse; Dunes is the lightest.
        #expect(Look.Photo.eclipse.lightness < Look.Photo.coast.lightness)
        #expect(Look.Photo.allCases.max { $0.lightness < $1.lightness } == .dunes)
    }

    /// A solid pane is its own colour behind the text, whatever the photo and however clear the slider says.
    @Test func onSolidPanesTextFollowsTheGlass() {
        let look = Look(defaults: freshDefaults())
        #expect(!look.isSolid)
        look.photo = .rings
        look.appearance = .light
        look.transparency = 100
        #expect(look.isDark)
        look.reducedTransparency = true
        #expect(look.isSolid)
        #expect(!look.isDark)
        look.reducedTransparency = false
        look.increasedContrast = true
        #expect(look.isSolid)
        #expect(!look.isDark)
        let lightPane = look.solidPane
        look.photo = .forest
        look.appearance = .dark
        #expect(look.isDark)
        // The pane itself is the colour of the glass that was picked.
        #expect(look.solidPane != lightPane)
        // Every photo, either way: the darkest of them under a light pane, the lightest under a dark one.
        for photo in Look.Photo.allCases {
            look.photo = photo
            look.appearance = .light
            #expect(!look.isDark, "\(photo)")
            look.appearance = .dark
            #expect(look.isDark, "\(photo)")
        }
        // All but solid: what is left of the photo is too little to tell.
        #expect(Look.solidBody >= 0.9 && Look.solidBody < 1)
        // The app opens with them as the Mac has them.
        let opened = Look(defaults: freshDefaults(), increasedContrast: true, reducedTransparency: false)
        #expect(opened.increasedContrast && !opened.reducedTransparency && opened.isSolid)
        #expect(!Look(defaults: freshDefaults()).isSolid)
    }

    @Test func transparencyTurnsIntoTintAndStrength() {
        let look = Look(defaults: freshDefaults())
        look.photo = .rings
        look.transparency = 92
        #expect(abs(look.tintOpacity - 0.08) < 1e-9)
        #expect(abs(look.strength - 8.0 / 18) < 1e-9)
        look.transparency = 82
        #expect(look.strength == 1)
        look.transparency = 100
        #expect(look.tintOpacity == 0)
        #expect(look.strength == 0)
        look.transparency = 0
        #expect(look.tintOpacity == 1)
    }

    /// Glass that does not match the photo carries more tint, so text stays apart from it.
    @Test func mismatchedGlassCarriesMore() {
        let look = Look(defaults: freshDefaults())
        look.transparency = 90
        look.photo = .forest
        look.appearance = .dark
        #expect(abs(look.tintOpacity - 0.1 * 1.8) < 1e-9)
        look.photo = .rings
        look.appearance = .light
        #expect(abs(look.tintOpacity - 0.1 * 1.7) < 1e-9)
        look.appearance = .auto
        #expect(abs(look.tintOpacity - 0.1) < 1e-9)
        look.transparency = 20
        look.appearance = .light
        #expect(look.tintOpacity == 1)
    }

    /// Twelve photos, each with a name of its own and its picture in the app's assets.
    @Test func everyPhotoHasItsPicture() throws {
        #expect(Look.Photo.allCases.count == 12)
        #expect(Set(Look.Photo.allCases.map(\.title)).count == 12)
        let assets = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Bottega/Resources/Assets.xcassets")
        for photo in Look.Photo.allCases {
            let set = assets.appendingPathComponent(photo.asset + ".imageset")
            let contents = try JSONSerialization.jsonObject(with: Data(contentsOf: set.appendingPathComponent("Contents.json")))
            let images = (contents as? [String: Any])?["images"] as? [[String: Any]] ?? []
            let names = images.compactMap { $0["filename"] as? String }
            #expect(!names.isEmpty, "\(photo)")
            for name in names {
                let size = try FileManager.default.attributesOfItem(atPath: set.appendingPathComponent(name).path)[.size] as? Int
                #expect((size ?? 0) > 10_000, "\(photo) \(name)")
            }
        }
    }

    /// The outline of a control and the lines between things do not fade with the glass: fully clear glass still
    /// shows where its controls are, by their edges alone (T-245).
    @Test func edgesStayWhenTheGlassIsClear() {
        let look = Look(defaults: freshDefaults())
        // A dark photo and a light one, each with the glass that goes with it.
        for photo in [Look.Photo.coast, .forest] {
            look.photo = photo
            look.transparency = 0
            let (edge, focus, line) = (look.edgeOpacity, look.focusEdgeOpacity, look.lineOpacity)
            look.transparency = 100
            #expect(look.strength == 0)
            #expect(look.edgeOpacity == edge, "\(photo)")
            #expect(look.focusEdgeOpacity == focus, "\(photo)")
            #expect(look.lineOpacity == line, "\(photo)")
            // A control's outline is plainer than a line between things, and the field being typed in plainer still.
            #expect(look.edgeOpacity > look.lineOpacity, "\(photo)")
            #expect(look.focusEdgeOpacity > look.edgeOpacity, "\(photo)")
            #expect(look.lineOpacity >= 0.17, "\(photo)")
        }
    }

    /// A card, and a control, is filled as the canvas's `--surf` is: all of it goes when the glass is clear, and
    /// the outline is what is left. A label that is only a fill keeps a little (T-254).
    @Test func aCardGoesWithTheGlassAndALabelKeepsALittle() {
        let look = Look(defaults: freshDefaults())
        for (photo, full, label) in [(Look.Photo.coast, 0.1, 0.1), (.forest, 0.26, 0.24)] {
            look.photo = photo
            look.transparency = 82
            #expect(abs(look.surfaceOpacity - full) < 1e-9, "\(photo)")
            #expect(abs(look.chipOpacity - label) < 1e-9, "\(photo)")
            look.transparency = 91
            #expect(abs(look.surfaceOpacity - full / 2) < 1e-9, "\(photo)")
            look.transparency = 100
            #expect(look.surfaceOpacity == 0, "\(photo)")
            #expect(look.surface == .white.opacity(0), "\(photo)")
            #expect(look.chipOpacity >= 0.03, "\(photo)")
            #expect(look.chip == .white.opacity(look.chipOpacity), "\(photo)")
        }
        // On a solid pane a card is a plain step up from it, whatever the slider says.
        look.reducedTransparency = true
        look.photo = .forest
        #expect(look.surfaceOpacity == 1)
        #expect(look.chip == .black.opacity(0.05))
        look.photo = .coast
        #expect(look.surfaceOpacity == 0.07)
        #expect(look.chip == .white.opacity(0.07))
    }

    /// With Increase Contrast on, the outlines are stronger, as the system's own get by themselves; the field
    /// being typed in still stands out from the rest.
    @Test func outlinesAreStrongerWithIncreasedContrast() {
        let look = Look(defaults: freshDefaults())
        // Solid panes either way, so that the text is as the appearance says: dark, then light.
        look.reducedTransparency = true
        for (appearance, canvasLine) in [(Look.Appearance.dark, 0.6), (.light, 0.5)] {
            look.appearance = appearance
            look.increasedContrast = false
            let (edge, focus, line) = (look.edgeOpacity, look.focusEdgeOpacity, look.lineOpacity)
            let (edgeColour, focusColour, lineColour) = (look.edge, look.focusEdge, look.line)
            look.increasedContrast = true
            #expect(look.edgeOpacity >= edge * 1.5, "\(appearance)")
            #expect(look.lineOpacity >= line * 1.5, "\(appearance)")
            // One rule for all three (T-254): an outline is still plainer than a line, and the field being typed
            // in plainer than either.
            #expect(look.edgeOpacity > look.lineOpacity, "\(appearance)")
            #expect(look.focusEdgeOpacity > look.edgeOpacity, "\(appearance)")
            #expect(look.focusEdgeOpacity <= 1, "\(appearance)")
            // The lines come out about where the canvas has them under `prefers-contrast`.
            #expect(abs(look.lineOpacity - canvasLine) < 0.05, "\(appearance)")
            // And the colours things are drawn with are the ones that change.
            #expect(look.edge != edgeColour, "\(appearance)")
            #expect(look.focusEdge != focusColour, "\(appearance)")
            #expect(look.line != lineColour, "\(appearance)")
            // Off, they are as they were.
            look.increasedContrast = false
            #expect(look.edgeOpacity == edge && look.focusEdgeOpacity == focus && look.lineOpacity == line, "\(appearance)")
            #expect(Look.outline(edge, increasedContrast: false) == edge, "\(appearance)")
        }
        // Never past opaque, however strong it was.
        #expect(Look.outline(0.9, increasedContrast: true) == 1)
    }

    /// Red for what waits on the user and orange for trouble: two colours, each with a dark-glass and a
    /// light-glass one, taken by the view from the scheme the window sets. A slider is filled in the text's colour.
    @Test func redAndOrangeAreGivenByTheGlass() {
        for isDark in [true, false] {
            #expect(Look.waiting(isDark: isDark) != Look.trouble(isDark: isDark))
            var environment = EnvironmentValues()
            environment.colorScheme = isDark ? .dark : .light
            #expect(WaitingRed().resolve(in: environment) == Look.waiting(isDark: isDark))
            #expect(TroubleOrange().resolve(in: environment) == Look.trouble(isDark: isDark))
        }
        #expect(Look.waiting(isDark: true) != Look.waiting(isDark: false))
        #expect(Look.trouble(isDark: true) != Look.trouble(isDark: false))
        // Neither is the system's own: those change with the Mac's settings, and red on dark glass is brighter here.
        #expect(Look.waiting(isDark: true) != Color.red)
        let look = Look(defaults: freshDefaults())
        look.photo = .coast
        #expect(look.mark == .white.opacity(0.85))
        look.photo = .forest
        #expect(look.mark == .black.opacity(0.85))
    }

    /// A photo that is not the window's shape is cut where its focus says, and never leaves the window bare.
    @Test func aPhotoKeepsItsFocusInView() {
        // In the middle: as much cut off above as below.
        #expect(Look.Photo.guide(content: 2560, frame: 800, focus: 0.5) == 1280)
        // 0 keeps the top: the photo's top is at the frame's top (the frame's middle, 400, less the guide).
        #expect(400 - Look.Photo.guide(content: 2560, frame: 800, focus: 0) == 0)
        // 1 keeps the bottom.
        #expect(400 - Look.Photo.guide(content: 2560, frame: 800, focus: 1) == 800 - 2560)
        // 0.3: three tenths of what does not fit is cut off above.
        #expect(abs(400 - Look.Photo.guide(content: 2560, frame: 800, focus: 0.3) - -0.3 * 1760) < 1e-9)
        for photo in Look.Photo.allCases {
            for (content, frame) in [(2560.0, 800.0), (800.0, 800.0), (1300.0, 1280.0)] {
                for focus in [photo.focus.x, photo.focus.y] {
                    let start = frame / 2 - Look.Photo.guide(content: content, frame: frame, focus: focus)
                    #expect(start <= 0 && start + content >= frame, "\(photo) \(content) in \(frame)")
                }
            }
        }
        // Hills is tall, with its moon and cloud above the middle.
        #expect(Look.Photo.hills.focus.y < 0.5)
        #expect(Look.Photo.coast.focus == .center)
    }
}
