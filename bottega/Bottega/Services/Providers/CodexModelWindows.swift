// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Synchronization

/// The largest context window Codex runs a model at, read from the catalog Codex keeps of
/// the models its account may use (`models_cache.json` in its home). The catalog names each
/// model's standard window (`context_window`, what Codex uses unless told otherwise) and the
/// largest it takes (`max_context_window`); a thread asks for the largest in its config (see
/// `CodexSession.threadParameters`). The app-server's `model/list` carries neither, so the
/// file is the one place to read them. Read once per change of the file: the window is
/// wanted at every launch and on every turn's signature.
enum CodexModelWindows {
    struct Window: Equatable, Sendable {
        /// Tokens the model takes at most.
        let maximum: Int
        /// Where Codex compacts the conversation: nine tenths of the window, below the share
        /// of it Codex actually fills (`effective_context_window_percent`, 95 for every model
        /// the catalog lists), so a turn never runs into the window before compacting.
        var compactAt: Int { maximum / 10 * 9 }
    }

    /// The window to ask for `model` (nil: the model Codex's own config names), or nil where
    /// the catalog cannot say, or offers nothing above the standard window: Codex then keeps
    /// its own default rather than being told a number nobody read.
    static func largest(for model: String?, environment: [String: String]) -> Window? {
        largest(forAll: [model], environment: environment)
    }

    /// The window a thread may ask for when every one of `models` runs in it (nil among
    /// them: Codex's own default model): the smallest of their largest windows, since the
    /// thread's window is one number for its lead and every hand Codex runs for it, and a
    /// model given more than it takes is an error. Nil where any of them is unknown to the
    /// catalog, or none offers more than its standard window.
    static func largest(forAll models: [String?], environment: [String: String]) -> Window? {
        let home = codexHome(environment)
        let catalog = catalog(home: home)
        var smallest: Int?
        for model in models {
            guard let slug = model ?? defaultModel(home: home), let entry = catalog[slug] else { return nil }
            smallest = min(smallest ?? entry.maximum, entry.maximum)
        }
        guard let smallest, models.contains(where: { model in
            (model ?? defaultModel(home: home)).flatMap { catalog[$0] }.map { $0.maximum > $0.standard } == true
        }) else { return nil }
        return Window(maximum: smallest)
    }

    private struct Entry: Sendable {
        let standard: Int
        let maximum: Int
    }

    private struct Cache: Sendable {
        var home: URL
        var modified: Date?
        var size: Int?
        var entries: [String: Entry]
    }

    private static let cache = Mutex<Cache?>(nil)

    /// The catalog by model slug, read again only when the file changed.
    private static func catalog(home: URL) -> [String: Entry] {
        let url = home.appendingPathComponent("models_cache.json")
        let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
        let modified = attributes?[.modificationDate] as? Date
        let size = attributes?[.size] as? Int
        return cache.withLock { cache in
            if let cache, cache.home == home, cache.modified == modified, cache.size == size { return cache.entries }
            var entries: [String: Entry] = [:]
            if let data = try? Data(contentsOf: url),
               let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let models = root["models"] as? [[String: Any]] {
                for model in models {
                    guard let slug = model["slug"] as? String, let maximum = model["max_context_window"] as? Int else { continue }
                    entries[slug] = Entry(standard: model["context_window"] as? Int ?? maximum, maximum: maximum)
                }
            }
            cache = Cache(home: home, modified: modified, size: size, entries: entries)
            return entries
        }
    }

    /// Codex's home: `CODEX_HOME` where the session's environment or the app's sets it, else
    /// `~/.codex`.
    private static func codexHome(_ environment: [String: String]) -> URL {
        if let home = environment["CODEX_HOME"] ?? ProcessInfo.processInfo.environment["CODEX_HOME"], !home.isEmpty {
            return URL(fileURLWithPath: (home as NSString).expandingTildeInPath)
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".codex")
    }

    /// The top-level `model = "…"` of Codex's `config.toml`: the model a thread started
    /// without one runs. Only the lines before the first table count; a `model` key inside
    /// `[profiles.…]` or any other table is not the default.
    private static func defaultModel(home: URL) -> String? {
        guard let text = try? String(contentsOf: home.appendingPathComponent("config.toml"), encoding: .utf8) else { return nil }
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("[") { return nil }
            guard let equals = trimmed.firstIndex(of: "="),
                  trimmed[..<equals].trimmingCharacters(in: .whitespaces) == "model" else { continue }
            let value = trimmed[trimmed.index(after: equals)...].trimmingCharacters(in: .whitespaces)
            if value.hasPrefix("\"") {
                let body = value.dropFirst()
                guard let close = body.firstIndex(of: "\"") else { return nil }
                return String(body[..<close]).nilIfEmpty
            }
            let bare = value.split(separator: "#", maxSplits: 1).first.map(String.init) ?? ""
            return bare.trimmingCharacters(in: .whitespaces).nilIfEmpty
        }
        return nil
    }

    /// The models a thread's window must fit: its own, and with Codex running the hands,
    /// the hands' (the pair's worker model, and each profile's own where one names it).
    static func models(thread model: String?, crew: CrewLaunch?) -> [String?] {
        var models: [String?] = [model]
        if let crew, crew.runsNatively {
            if let worker = crew.workerModel?.nilIfEmpty { models.append(worker) }
            for profile in CrewPrompts.nativeRoutableProfiles(of: crew) {
                if let own = profile.model?.nilIfEmpty { models.append(own) }
            }
        }
        return models
    }
}
