// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The video behind a bare video link in chat: `https://www.youtube.com/watch?v=…` reads as
/// the video's own title instead of "youtube.com/watch".
///
/// Titles come from YouTube's public oEmbed endpoint, which needs no key and answers with
/// the title as JSON. The store is locked rather than `@MainActor` because `RichLink.build`
/// reads it off the main actor when a paragraph is warmed for the cache.
enum LinkTitles {
    /// The video id a link carries, or nil for anything that is not a video link:
    /// `youtube.com/watch?v=…`, `youtu.be/…`, `youtube.com/shorts/…`, `/embed/…`, `/live/…`.
    static func videoID(for url: URL) -> String? {
        guard let host = url.host?.lowercased() else { return nil }
        let bare = host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
        let segments = url.path.split(separator: "/").map(String.init)
        switch bare {
        case "youtu.be":
            guard let id = segments.first else { return nil }
            return checked(id)
        case "youtube.com", "m.youtube.com", "music.youtube.com", "youtube-nocookie.com":
            if segments.count >= 2, ["shorts", "embed", "live", "v"].contains(segments[0]) {
                return checked(segments[1])
            }
            let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems
            guard let value = query?.first(where: { $0.name == "v" })?.value else { return nil }
            return checked(value)
        default:
            return nil
        }
    }

    /// The title for a URL, if one has been fetched. Read from `RichLink.prettyTitle`, which
    /// runs on whichever thread builds a paragraph.
    static func cached(for url: URL) -> String? {
        guard let id = videoID(for: url) else { return nil }
        return Store.shared.title(id)
    }

    /// The title for a video, fetched at most once: nil while the fetch is in flight, and nil
    /// for good when there is none to have (private, deleted, offline), so the host-and-path
    /// form stands and the row never asks again.
    static func title(forVideo id: String) async -> String? {
        if let known = Store.shared.title(id) { return known }
        guard Store.shared.claim(id) else { return nil }
        guard let url = oembedURL(for: id),
              let (data, response) = try? await URLSession.shared.data(from: url),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let payload = try? JSONDecoder().decode(OEmbed.self, from: data) else {
            Store.shared.finish(id, title: nil)
            return nil
        }
        let title = payload.title.trimmingCharacters(in: .whitespacesAndNewlines)
        Store.shared.finish(id, title: title.isEmpty ? nil : title)
        return title.isEmpty ? nil : title
    }

    private struct OEmbed: Decodable {
        let title: String
    }

    private static func oembedURL(for id: String) -> URL? {
        var components = URLComponents(string: "https://www.youtube.com/oembed")
        components?.queryItems = [
            URLQueryItem(name: "url", value: "https://www.youtube.com/watch?v=\(id)"),
            URLQueryItem(name: "format", value: "json"),
        ]
        return components?.url
    }

    /// A video id is long enough to be one and made of the characters YouTube uses; anything
    /// else (a bare `/watch`, a playlist link) is no video to ask a title for.
    private static func checked(_ value: String) -> String? {
        guard value.count >= 8,
              value.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }) else { return nil }
        return value
    }

    /// Titles by video id, read from the main actor and from a warm-up build alike, so the
    /// store is a locked one and not `@MainActor` state. A claimed id counts as answered:
    /// a streaming row restarts its task on every token, and the same video is asked once.
    private final class Store: @unchecked Sendable {
        static let shared = Store()

        private let lock = NSLock()
        private var titles: [String: String] = [:]
        private var claimed = Set<String>()

        func title(_ id: String) -> String? {
            lock.lock()
            defer { lock.unlock() }
            return titles[id]
        }

        /// Whether this caller is the one that should do the fetch.
        func claim(_ id: String) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            return claimed.insert(id).inserted
        }

        /// The answer, title or none.
        func finish(_ id: String, title: String?) {
            lock.lock()
            defer { lock.unlock() }
            if let title {
                if titles.count >= 400 { titles.removeAll(keepingCapacity: true) }
                titles[id] = title
            }
            claimed.insert(id)
        }
    }
}
