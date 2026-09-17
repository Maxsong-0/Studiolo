import AppKit
import Foundation
import UserNotifications

/// A release version as GitLab tags it: `v1.2.3` or `1.2`. Anything with a prerelease suffix
/// (`-nightly.…`, `-beta.1`) is not a version the app offers, so it does not parse.
struct AppVersion: Comparable, Hashable, CustomStringConvertible, Sendable {
    let components: [Int]

    init?(_ raw: String) {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("v") || text.hasPrefix("V") { text.removeFirst() }
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        guard (1...3).contains(parts.count) else { return nil }
        var numbers: [Int] = []
        for part in parts {
            guard !part.isEmpty, part.allSatisfy(\.isNumber), let number = Int(part) else { return nil }
            numbers.append(number)
        }
        while numbers.count < 3 { numbers.append(0) }
        components = numbers
    }

    var description: String {
        components.map(String.init).joined(separator: ".")
    }

    /// The tag without its leading v: the version as the release calls itself.
    static func text(ofTag tag: String) -> String {
        var text = tag.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("v") || text.hasPrefix("V") { text.removeFirst() }
        return text
    }

    static func < (lhs: AppVersion, rhs: AppVersion) -> Bool {
        lhs.components.lexicographicallyPrecedes(rhs.components)
    }
}

/// A newer release on GitLab: what the About page describes and the installer downloads.
struct AvailableUpdate: Codable, Equatable, Sendable {
    var version: String
    var tag: String
    var downloadURL: URL
    var size: Int64?
    /// The release description, Markdown as written on GitLab.
    var notes: String
    var releasedAt: Date?
    /// The release page, for reading the notes in the browser.
    var pageURL: URL
}

/// Checks the app's GitLab releases for a newer version and remembers what it found.
///
/// A check reads the public releases API, keeps the newest stable version that ships a disk
/// image, and compares it with the running build. The result is persisted, so the About page
/// shows a known update the moment it opens, and `updateAvailable` is recomputed from the
/// running version, so an update that was installed stops being one. Checks run on launch,
/// every six hours while the app runs, when it comes back to the front after a long
/// while, and whenever the About page opens or asks.
@MainActor
@Observable
final class UpdateChecker {
    static let shared = UpdateChecker()

    /// The project on GitLab. Public, so the API needs no token.
    static let projectID = "86415974"
    static let projectURL = URL(string: "https://gitlab.com/droppyformac1/droppy-code")!
    static let releasesURL = projectURL.appending(path: "-/releases")
    private static let apiURL = URL(string: "https://gitlab.com/api/v4/projects/\(projectID)/releases?per_page=20")!

    /// How often the background check runs, and how stale a check may be before the app
    /// coming to the front runs another.
    private static let checkInterval: TimeInterval = 6 * 60 * 60
    private static let launchDelay: TimeInterval = 8

    private enum Keys {
        static let update = "updateChecker_release"
        static let lastChecked = "updateChecker_lastChecked"
        static let notifiedVersion = "updateChecker_notifiedVersion"
    }

    let currentVersion = AppInfo.version

    private(set) var update: AvailableUpdate?
    private(set) var isChecking = false
    private(set) var lastCheckedAt: Date?
    /// Why the last check found nothing, for the About page after a manual check.
    private(set) var lastError: String?

    /// Only a version newer than the running one is an update; anything else the last check
    /// found is kept for the notes but reads as up to date.
    var updateAvailable: Bool {
        guard let update, let latest = AppVersion(update.version), let current = AppVersion(currentVersion) else { return false }
        return latest > current
    }

    var latestVersion: String? { updateAvailable ? update?.version : nil }

    @ObservationIgnored private var backgroundTask: Task<Void, Never>?
    @ObservationIgnored private var activationObserver: NSObjectProtocol?
    @ObservationIgnored private var previewVersion: String?
    @ObservationIgnored private let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 30
        configuration.waitsForConnectivity = false
        configuration.httpAdditionalHeaders = ["User-Agent": "Droppy Code/\(AppInfo.version)", "Accept": "application/json"]
        return URLSession(configuration: configuration)
    }()

    private init() {
        let defaults = UserDefaults.standard
        if let data = defaults.data(forKey: Keys.update),
           let saved = try? JSONDecoder().decode(AvailableUpdate.self, from: data) {
            update = saved
        }
        lastCheckedAt = defaults.object(forKey: Keys.lastChecked) as? Date
        // `--preview-update 9.9.9` shows the update story without a newer release: the latest
        // real release is offered under that version. Pressing Update & restart then installs
        // that release for real, which only passes verification when its version is the one shown.
        let arguments = CommandLine.arguments
        if let index = arguments.firstIndex(of: "--preview-update"), index + 1 < arguments.count {
            previewVersion = arguments[index + 1]
        }
    }

    // MARK: - Schedule

    /// The launch check, the six-hourly one after it, and a catch-up when the app comes to
    /// the front after a long sleep.
    func startBackgroundChecks() {
        guard backgroundTask == nil, !WebsiteCaptures.isEnabled, !AppInfo.isDevelopment else { return }
        backgroundTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(Self.launchDelay))
            while !Task.isCancelled {
                await self?.check(background: true)
                try? await Task.sleep(for: .seconds(Self.checkInterval))
            }
        }
        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                let stale = lastCheckedAt.map { Date.now.timeIntervalSince($0) > Self.checkInterval } ?? true
                if stale { await check(background: true) }
            }
        }
    }

    // MARK: - Checking

    /// Reads the releases and updates the state. Returns whether an update is available
    /// afterwards. A background check also posts the one notification a new version gets.
    @discardableResult
    func check(background: Bool = false) async -> Bool {
        guard !AppInfo.isDevelopment else { return false }
        guard !isChecking else { return updateAvailable }
        isChecking = true
        defer { isChecking = false }
        do {
            let releases = try await fetchReleases()
            guard let newest = Self.newestRelease(in: releases) else {
                throw UpdateError.noRelease
            }
            var found = newest
            if let previewVersion {
                found.version = previewVersion
            }
            if found.size == nil {
                found.size = await downloadSize(of: found.downloadURL)
            }
            update = found
            lastError = nil
            lastCheckedAt = .now
            persist()
            if background, updateAvailable {
                notifyOnce(about: found)
            }
        } catch {
            lastError = (error as? UpdateError)?.errorDescription ?? "Couldn't reach GitLab."
            lastCheckedAt = .now
            UserDefaults.standard.set(lastCheckedAt, forKey: Keys.lastChecked)
        }
        return updateAvailable
    }

    private func fetchReleases() async throws -> [GitLabRelease] {
        let (data, response) = try await session.data(from: Self.apiURL)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw UpdateError.badResponse((response as? HTTPURLResponse)?.statusCode ?? -1)
        }
        return try JSONDecoder().decode([GitLabRelease].self, from: data)
    }

    /// The newest stable release that ships a disk image. Prereleases and upcoming releases
    /// never count, whatever their date.
    static func newestRelease(in releases: [GitLabRelease]) -> AvailableUpdate? {
        var best: (AppVersion, AvailableUpdate)?
        for release in releases {
            guard !release.upcomingRelease, let version = AppVersion(release.tagName) else { continue }
            if let releasedAt = release.releasedAt, releasedAt > .now { continue }
            guard let link = release.assets.links.first(where: { $0.isDiskImage }) else { continue }
            guard let downloadURL = URL(string: link.directAssetURL ?? link.url) else { continue }
            let candidate = AvailableUpdate(
                version: AppVersion.text(ofTag: release.tagName),
                tag: release.tagName,
                downloadURL: downloadURL,
                size: nil,
                notes: release.description ?? "",
                releasedAt: release.releasedAt,
                pageURL: releasesURL.appending(path: release.tagName)
            )
            if let current = best, current.0 >= version { continue }
            best = (version, candidate)
        }
        return best?.1
    }

    /// The image's size from a HEAD request, for the About page. Nothing depends on it.
    private func downloadSize(of url: URL) async -> Int64? {
        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"
        guard let (_, response) = try? await session.data(for: request),
              let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode),
              http.expectedContentLength > 0 else { return nil }
        return http.expectedContentLength
    }

    private func persist() {
        let defaults = UserDefaults.standard
        if let update, let data = try? JSONEncoder().encode(update) {
            defaults.set(data, forKey: Keys.update)
        } else {
            defaults.removeObject(forKey: Keys.update)
        }
        defaults.set(lastCheckedAt, forKey: Keys.lastChecked)
    }

    // MARK: - Notification

    /// One banner per version, the first time a background check finds it. Tapping it opens
    /// Settings on About, where the update installs.
    private func notifyOnce(about update: AvailableUpdate) {
        let defaults = UserDefaults.standard
        guard defaults.string(forKey: Keys.notifiedVersion) != update.version else { return }
        defaults.set(update.version, forKey: Keys.notifiedVersion)
        let content = UNMutableNotificationContent()
        content.title = "Droppy Code \(update.version) is available"
        content.body = "Open Settings › About to update and restart."
        content.userInfo = ["update": update.version]
        let request = UNNotificationRequest(identifier: "update-\(update.version)", content: content, trigger: nil)
        Task {
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            if settings.authorizationStatus == .notDetermined {
                _ = try? await center.requestAuthorization(options: [.alert, .sound, .badge])
            }
            try? await center.add(request)
        }
    }
}

enum UpdateError: LocalizedError {
    case noRelease
    case badResponse(Int)

    var errorDescription: String? {
        switch self {
        case .noRelease: "No release with a disk image was found."
        case .badResponse(let status): "GitLab answered with HTTP \(status)."
        }
    }
}

/// The parts of a GitLab release the checker reads.
struct GitLabRelease: Decodable, Sendable {
    struct Assets: Decodable, Sendable {
        var links: [Link]
    }

    struct Link: Decodable, Sendable {
        var name: String
        var url: String
        var directAssetURL: String?

        var isDiskImage: Bool {
            let path = (directAssetURL ?? url).lowercased()
            return path.hasSuffix(".dmg") || name.lowercased().hasSuffix(".dmg")
        }

        private enum CodingKeys: String, CodingKey {
            case name, url
            case directAssetURL = "direct_asset_url"
        }
    }

    var tagName: String
    var name: String?
    var description: String?
    var releasedAt: Date?
    var upcomingRelease: Bool
    var assets: Assets

    private enum CodingKeys: String, CodingKey {
        case tagName = "tag_name"
        case name, description, assets
        case releasedAt = "released_at"
        case upcomingRelease = "upcoming_release"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        tagName = try container.decode(String.self, forKey: .tagName)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        description = try container.decodeIfPresent(String.self, forKey: .description)
        releasedAt = try container.decodeIfPresent(String.self, forKey: .releasedAt).flatMap(Self.parseDate)
        upcomingRelease = try container.decodeIfPresent(Bool.self, forKey: .upcomingRelease) ?? false
        assets = try container.decodeIfPresent(Assets.self, forKey: .assets) ?? Assets(links: [])
    }

    /// GitLab writes `2026-09-13T18:28:42.057Z`; the fraction is optional.
    nonisolated static func parseDate(_ text: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: text) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: text)
    }
}
