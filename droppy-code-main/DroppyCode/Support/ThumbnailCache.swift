import CoreGraphics
import Foundation
import ImageIO

/// Downsampled image attachments, decoded off the main actor and kept for reuse, so scrolling past
/// a message with images never reads or decodes a file while drawing. The decodes run outside
/// the actor, so fifteen thumbnails for the downloads popover decode side by side instead of
/// one behind the other, and callers asking for the same thumbnail share one decode.
actor ThumbnailCache {
    static let shared = ThumbnailCache()

    struct Thumbnail: @unchecked Sendable {
        let image: CGImage
    }

    private var thumbnails = RecentCache<String, Thumbnail>(limit: 120, costLimit: 32 * 1_024 * 1_024)
    /// Decodes under way, by key, for later callers to join.
    private var inFlight: [String: Task<Thumbnail?, Never>] = [:]
    /// How many decodes have started, for tests of the sharing.
    private(set) var decodes = 0

    func thumbnail(for path: String, pointSize: CGFloat, fillingSquare: Bool = false) async -> Thumbnail? {
        guard !Task.isCancelled else { return nil }
        let pixels = max(1, Int((pointSize * 2).rounded(.up)))
        let key = "\(path)#\(pixels)#\(fillingSquare)"
        if let cached = thumbnails.value(for: key) { return cached }
        let task: Task<Thumbnail?, Never>
        if let pending = inFlight[key] {
            task = pending
        } else {
            decodes += 1
            task = Task.detached(priority: .userInitiated) { Self.decode(path, pixels: pixels, fillingSquare: fillingSquare) }
            inFlight[key] = task
        }
        // A caller giving up waits the decode out rather than cancelling it for the others.
        let thumbnail = await task.value
        if inFlight[key] == task {
            inFlight[key] = nil
            if let thumbnail { thumbnails.insert(thumbnail, for: key, cost: thumbnail.image.bytesPerRow * thumbnail.image.height) }
        }
        return Task.isCancelled ? nil : thumbnail
    }

    private nonisolated static func decode(_ path: String, pixels: Int, fillingSquare: Bool) -> Thumbnail? {
        guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else { return nil }
        var pixels = pixels
        if fillingSquare,
           let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
           let width = properties[kCGImagePropertyPixelWidth] as? Double,
           let height = properties[kCGImagePropertyPixelHeight] as? Double,
           min(width, height) > 0 {
            pixels = Int(min(2048, ceil(Double(pixels) * max(width, height) / min(width, height))))
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: pixels,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return Thumbnail(image: image)
    }
}

/// Thumbnails already decoded, readable on the main actor without a hop. A row scrolling
/// back into view shows its photos in the pass that builds it, instead of a frame later
/// with a fade, and never asks the cache actor for something it has shown before.
@MainActor
enum ThumbnailMemory {
    private static var images = RecentCache<String, CGImage>(limit: 120, costLimit: 16 * 1_024 * 1_024)

    private static func key(_ path: String, pointSize: CGFloat) -> String {
        "\(path)#\(Int((pointSize * 2).rounded(.up)))"
    }

    static func image(for path: String, pointSize: CGFloat) -> CGImage? {
        images.value(for: key(path, pointSize: pointSize))
    }

    static func store(_ image: CGImage, for path: String, pointSize: CGFloat) {
        images.insert(image, for: key(path, pointSize: pointSize), cost: image.bytesPerRow * image.height)
    }
}
