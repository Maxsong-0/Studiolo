// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing

/// Keys that sign without a finger (`SoftwareKeys` in `BottegaTestKeys`, for the package's tests and
/// `bottega-probe`) stay out of the app: with them inside, whoever got the app to use them could approve
/// without Touch ID.
struct TestKeysStayOutTests {
    /// The module and the type, as they are spelled in compiled code and wherever a dependency is named.
    static let names = ["BottegaTestKeys", "SoftwareKeys"]

    static func names(in bytes: Data) -> [String] {
        names.filter { bytes.range(of: Data($0.utf8)) != nil }
    }

    /// `bottega/`, from where this file is.
    static let folder = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()

    private static func files(under folder: URL) throws -> [URL] {
        let all = try #require(FileManager.default.enumerator(at: folder, includingPropertiesForKeys: [.isRegularFileKey]))
        return all.compactMap { $0 as? URL }.filter { (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true }
    }

    /// What the app is built from does not name them: not as a dependency, not in an import.
    @Test func nothingTheAppIsBuiltFromNamesThem() throws {
        let project = try Data(contentsOf: Self.folder.appendingPathComponent("project.yml"))
        #expect(Self.names(in: project).isEmpty, "project.yml names the test keys")
        var looked = 0
        for part in ["Bottega", "Shared", "BottegaKit/Sources/BottegaKit", "BottegaKit/Sources/BottegaLink"] {
            for file in try Self.files(under: Self.folder.appendingPathComponent(part)) where file.pathExtension == "swift" {
                looked += 1
                #expect(Self.names(in: try Data(contentsOf: file)).isEmpty, "\(part)/…/\(file.lastPathComponent) names the test keys")
            }
        }
        // Not an empty folder that passes for clean.
        #expect(looked > 100)
    }

    /// Nothing the built app is made of carries them.
    @Test func theBuiltAppDoesNotCarryThem() throws {
        let contents = try IdentityTests.app().bundleURL.appendingPathComponent("Contents")
        let files = try Self.files(under: contents)
        #expect(files.contains { $0.lastPathComponent == "Bottega Dev.debug.dylib" }, "the app's own code was not among what was looked at")
        for file in files {
            let found = Self.names(in: try Data(contentsOf: file, options: .mappedIfSafe))
            #expect(found.isEmpty, "\(file.lastPathComponent) carries \(found)")
        }
    }

    /// The other side of both checks: where they are named and where they are compiled in, they are seen.
    @Test func whereTheyAreTheyAreSeen() throws {
        let package = try Data(contentsOf: Self.folder.appendingPathComponent("BottegaKit/Package.swift"))
        #expect(Self.names(in: package) == ["BottegaTestKeys"])
        let source = try Data(contentsOf: Self.folder.appendingPathComponent("BottegaKit/Sources/BottegaTestKeys/SoftwareKeys.swift"))
        #expect(Self.names(in: source) == ["SoftwareKeys"])
        // The package's own tests link them, and are built by the same scheme into the same products folder. Both
        // names are in that code, so either one alone gives the app away.
        let products = try IdentityTests.app().bundleURL.deletingLastPathComponent()
        let tests = try Self.files(under: products.appendingPathComponent("BottegaLinkTests.xctest"))
        let carrying = try tests.filter { Self.names(in: try Data(contentsOf: $0, options: .mappedIfSafe)) == Self.names }
        #expect(!carrying.isEmpty, "compiled code that links the test keys shows none of their names: the check above sees nothing")
    }
}
