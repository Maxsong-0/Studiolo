// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
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

    @Test func startsAsTheDesignDoes() {
        let look = Look(defaults: freshDefaults())
        #expect(look.photo == .coast)
        #expect(look.appearance == .auto)
        #expect(look.blur == 4)
        #expect(look.grain == 24)
        #expect(look.transparency == 92)
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
        for photo in Look.Photo.allCases {
            look.photo = photo
            #expect(look.isDark == (photo != .forest), "\(photo)")
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
}
