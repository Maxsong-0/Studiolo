// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import Bottega

/// Smoke tests for the move out of Tillerwright (Studiolo T-225): the app this target is
/// hosted in is Bottega, keeps its own storage, and still carries Droppy Code's attribution.
/// They run in the Debug app (TEST_HOST), so Bundle.main is "Bottega Dev".
struct IdentityTests {
    /// The attribution LICENSE section 7(a) asks every modified copy to keep, word for word.
    static let attribution = "Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app"

    @Test func hostIsBottegaDev() {
        #expect(Bundle.main.bundleIdentifier == "me.studiolo.bottega.dev")
        #expect(AppInfo.name == "Bottega Dev")
        #expect(AppInfo.isDevelopment)
    }

    /// Storage, Keychain and state directory are keyed on Bottega, never on Tillerwright, so the
    /// still-installed Tillerwright Dev.app and this build never read each other's data.
    @Test func storageIsBottegasOwn() {
        #expect(AppInfo.storageName == "Bottega Dev")
        #expect(AppInfo.keychainService == "Bottega Dev")
        #expect(AppInfo.stateDirectoryName == ".bottega-dev")
        #expect(!Storage.root.path.localizedCaseInsensitiveContains("tillerwright"))
    }

    @Test func copyrightCarriesDroppyAttribution() throws {
        let copyright = try #require(Bundle.main.object(forInfoDictionaryKey: "NSHumanReadableCopyright") as? String)
        #expect(copyright.contains(Self.attribution))
        #expect(copyright.contains("modified"))
    }

    /// About › Licenses reads these three from the bundle; LICENSE section 7(a) requires the notices to travel too.
    @Test(arguments: [("LICENSE", nil as String?), ("THIRD_PARTY_NOTICES", "md"), ("TRADEMARK", "md")])
    func licenseFilesShipInTheBundle(name: String, ext: String?) throws {
        let url = try #require(Bundle.main.url(forResource: name, withExtension: ext))
        let text = try String(contentsOf: url, encoding: .utf8)
        #expect(!text.isEmpty)
        if name == "LICENSE" {
            #expect(text.contains(Self.attribution))
        }
    }
}
