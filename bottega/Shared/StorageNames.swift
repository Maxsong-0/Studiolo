// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// What Bottega calls itself on disk, for a Debug or a Release build. Pure and dependency-free:
/// `AppInfo` reads it with the running build's flavor, and BottegaTests compiles this folder in
/// directly (bottega/Shared, see project.yml), so the names are checked without launching the
/// app (ADR-043: tests touch no real run directories or keys).
enum StorageNames {
    /// The Debug build is told apart by its bundle id's suffix rather than one hard-coded id, so
    /// renaming the bundle does not silently make the dev build look like the release one.
    static func isDevelopment(bundleIdentifier: String?) -> Bool {
        bundleIdentifier?.hasSuffix(".dev") == true
    }

    /// The Application Support folder, the Keychain service and the log folder.
    static func storage(isDevelopment: Bool) -> String {
        isDevelopment ? "Bottega Dev" : "Bottega"
    }

    /// The state directory in the home folder.
    static func stateDirectory(isDevelopment: Bool) -> String {
        isDevelopment ? ".bottega-dev" : ".bottega"
    }
}
