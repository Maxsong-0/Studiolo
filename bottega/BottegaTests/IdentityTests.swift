// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing

/// Smoke tests for the move out of Tillerwright (Studiolo T-225). They read the built Debug app
/// next to this test bundle, plus the pure StorageNames.swift compiled in, without launching it:
/// a launched app reaches for the real Studiolo vault, the Keychain and the agent CLIs, and an ad
/// hoc signed build re-asks macOS for each of those, which hangs an unattended run (ADR-043:
/// tests touch no real run directories or keys).
struct IdentityTests {
    /// The attribution LICENSE section 7(a) asks every modified copy to keep, word for word.
    static let attribution = "Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app"

    /// The one place the old name stays on purpose: the title of an archived Studiolo vault note.
    static let allowedOldName = "2026-09-20-T-095-Tillerwright纳入供给白名单"

    /// `Bottega Dev.app`, built by the same scheme into the same products folder as this bundle.
    static func app() throws -> Bundle {
        let products = Bundle(for: Marker.self).bundleURL.deletingLastPathComponent()
        let url = products.appendingPathComponent("Bottega Dev.app", isDirectory: true)
        return try #require(Bundle(url: url), "no app at \(url.path)")
    }

    @Test func appIsBottegaDev() throws {
        let app = try Self.app()
        #expect(app.bundleIdentifier == "me.studiolo.bottega.dev")
        #expect(app.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String == "Bottega Dev")
        #expect(app.object(forInfoDictionaryKey: "CFBundleName") as? String == "Bottega Dev")
    }

    /// Storage, Keychain and state directory, as `AppInfo` computes them (bottega/Shared/StorageNames.swift
    /// is compiled into these tests): Bottega's own, Debug and Release apart, and the built app is
    /// the Debug flavor. The still-installed Tillerwright Dev.app and this build must never read
    /// each other's data, and a Debug and a Release copy must never share a library.
    @Test func storageNamesAreBottegasOwn() throws {
        #expect(StorageNames.storage(isDevelopment: true) == "Bottega Dev")
        #expect(StorageNames.storage(isDevelopment: false) == "Bottega")
        #expect(StorageNames.stateDirectory(isDevelopment: true) == ".bottega-dev")
        #expect(StorageNames.stateDirectory(isDevelopment: false) == ".bottega")
        #expect(StorageNames.isDevelopment(bundleIdentifier: try Self.app().bundleIdentifier))
        #expect(!StorageNames.isDevelopment(bundleIdentifier: "me.studiolo.bottega"))
        for name in [true, false].flatMap({ [StorageNames.storage(isDevelopment: $0), StorageNames.stateDirectory(isDevelopment: $0)] }) {
            #expect(!name.localizedCaseInsensitiveContains("tillerwright"))
        }
    }

    /// A net for the rename, not proof of it: the string constants in the Debug build's code
    /// (Bottega Dev.debug.dylib, where a Debug build keeps all of its Swift) carry no Tillerwright
    /// spelling except the one allowed above. Only literals stored as strings are seen; a short one
    /// the compiler folds into instructions would slip past, which is why the storage names have
    /// their own test.
    @Test func debugDylibCarriesNoOldName() throws {
        let app = try Self.app()
        let code = app.bundleURL.appendingPathComponent("Contents/MacOS/Bottega Dev.debug.dylib")
        let bytes = try Data(contentsOf: code)
        let allowed = Data(Self.allowedOldName.utf8)
        var stripped = bytes
        while let found = stripped.range(of: allowed) { stripped.removeSubrange(found) }
        for spelling in ["Tillerwright", "tillerwright", "TILLERWRIGHT"] {
            #expect(stripped.range(of: Data(spelling.utf8)) == nil, "\(spelling) is still in the compiled code")
        }
    }

    @Test func copyrightCarriesDroppyAttribution() throws {
        let copyright = try #require(try Self.app().object(forInfoDictionaryKey: "NSHumanReadableCopyright") as? String)
        #expect(copyright.contains(Self.attribution))
        #expect(copyright.contains("modified"))
    }

    /// About › Licenses reads these three from the bundle; LICENSE section 7(a) requires the notices to travel too.
    @Test(arguments: [("LICENSE", nil as String?), ("THIRD_PARTY_NOTICES", "md"), ("TRADEMARK", "md")])
    func licenseFilesShipInTheBundle(name: String, ext: String?) throws {
        let url = try #require(try Self.app().url(forResource: name, withExtension: ext))
        let text = try String(contentsOf: url, encoding: .utf8)
        #expect(!text.isEmpty)
        if name == "LICENSE" {
            #expect(text.contains(Self.attribution))
        }
    }
}

/// Only here so `Bundle(for:)` can find this test bundle.
private final class Marker {}
