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

    @Test func darkFollowsThePhotoUnlessFixed() {
        let look = Look(defaults: freshDefaults())
        let light: Set<Look.Photo> = [.forest, .hills, .dunes]
        for photo in Look.Photo.allCases {
            look.photo = photo
            #expect(look.isDark == !light.contains(photo), "\(photo)")
        }
        look.appearance = .light
        look.photo = .rings
        #expect(!look.isDark)
        look.appearance = .dark
        look.photo = .forest
        #expect(look.isDark)
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
        for appearance in [Look.Appearance.dark, .light] {
            look.appearance = appearance
            look.transparency = 0
            let (edge, focus, line) = (look.edgeOpacity, look.focusEdgeOpacity, look.lineOpacity)
            look.transparency = 100
            #expect(look.strength == 0)
            #expect(look.edgeOpacity == edge, "\(appearance)")
            #expect(look.focusEdgeOpacity == focus, "\(appearance)")
            #expect(look.lineOpacity == line, "\(appearance)")
            // A control's outline is plainer than a line between things, and the field being typed in plainer still.
            #expect(look.edgeOpacity > look.lineOpacity, "\(appearance)")
            #expect(look.focusEdgeOpacity > look.edgeOpacity, "\(appearance)")
            #expect(look.lineOpacity >= 0.17, "\(appearance)")
        }
    }

    /// With Increase Contrast on, the outlines are stronger, as the system's own get by themselves; the field
    /// being typed in still stands out from the rest.
    @Test func outlinesAreStrongerWithIncreasedContrast() {
        let look = Look(defaults: freshDefaults())
        for appearance in [Look.Appearance.dark, .light] {
            look.appearance = appearance
            let edge = Look.outline(look.edgeOpacity, increasedContrast: true)
            let focus = Look.outline(look.focusEdgeOpacity, increasedContrast: true)
            #expect(edge >= look.edgeOpacity * 1.5, "\(appearance)")
            #expect(focus > edge, "\(appearance)")
            #expect(focus <= 1, "\(appearance)")
            // Off, they are as they were.
            #expect(Look.outline(look.edgeOpacity, increasedContrast: false) == look.edgeOpacity, "\(appearance)")
            #expect(Look.outline(look.focusEdgeOpacity, increasedContrast: false) == look.focusEdgeOpacity, "\(appearance)")
            // And the colours the controls are drawn with are the ones that change.
            #expect(look.edge(increasedContrast: true) != look.edge(increasedContrast: false), "\(appearance)")
            #expect(look.focusEdge(increasedContrast: true) != look.focusEdge(increasedContrast: false), "\(appearance)")
        }
        // Never past opaque, however strong it was.
        #expect(Look.outline(0.9, increasedContrast: true) == 1)
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
