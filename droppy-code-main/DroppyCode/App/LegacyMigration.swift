import Foundation

/// Carries settings, the library and thread histories over from the app's earlier name, once.
enum LegacyMigration {
    private static let marker = "migratedFromEarlierName"
    private static let formerBundleIdentifier = "iordv.cody"
    private static let formerSupportFolder = "Cody"

    static func run() {
        guard !AppInfo.isDevelopment else { return }
        let defaults = UserDefaults.standard
        guard !defaults.bool(forKey: marker) else { return }
        if let former = defaults.persistentDomain(forName: formerBundleIdentifier) {
            for (key, value) in former where defaults.object(forKey: key) == nil {
                defaults.set(value, forKey: key)
            }
        }
        let fileManager = FileManager.default
        let support = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let formerLibrary = support.appendingPathComponent(formerSupportFolder, isDirectory: true)
        let library = support.appendingPathComponent("Droppy Code", isDirectory: true)
        if fileManager.fileExists(atPath: formerLibrary.path), !fileManager.fileExists(atPath: library.path) {
            try? fileManager.moveItem(at: formerLibrary, to: library)
        }
        defaults.set(true, forKey: marker)
    }
}
