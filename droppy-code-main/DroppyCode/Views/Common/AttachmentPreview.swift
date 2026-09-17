import AppKit
import AVFoundation
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

/// Shared helpers for showing large image previews of attachments and files the agent read.
enum PreviewImages {
    static let extensions: Set<String> = [
        "png", "jpg", "jpeg", "gif", "webp", "heic", "heif",
        "bmp", "tif", "tiff", "avif", "ico", "svg",
    ]

    static func isImagePath(_ path: String) -> Bool {
        extensions.contains((path as NSString).pathExtension.lowercased())
    }

    /// Answers already found, keyed by everything the answer is made of. Every tool row
    /// asks this on every render, and the answer ends in `FileManager.fileExists`: a disk
    /// hit, per row, per frame the row is rebuilt. A row's question never changes once its
    /// call has finished, so it is answered once and remembered.
    @MainActor private static var resolvedPaths = RecentCache<String, String?>(limit: 600)

    /// Finds an image file a tool looked at or made. A read's title (and detail) is the
    /// path itself, spaces included; any other tool's title and detail are searched for a
    /// path among their words, so a screenshot a command took shows too. Relative paths
    /// resolve against the thread's working directory; only a file that exists counts.
    @MainActor
    static func resolveToolImagePath(for call: ToolCall, workingDirectory: String?) -> String? {
        // Most rows name no image at all: no image extension anywhere means there is no
        // candidate to split the text for, and nothing worth a cache entry.
        guard mentionsImage(call.title) || call.detail.map(mentionsImage) == true else { return nil }
        // The status is part of the question: a command that makes a file has not made it
        // yet while it runs, so the answer it gets then must not outlive it.
        let key = "\(call.kind.rawValue)\u{1}\(call.status.rawValue)\u{1}\(call.title)\u{1}\(call.detail ?? "")\u{1}\(workingDirectory ?? "")"
        if let hit = resolvedPaths.value(for: key) { return hit }
        let resolved = search(for: call, workingDirectory: workingDirectory)
        resolvedPaths.insert(resolved, for: key)
        return resolved
    }

    private static func mentionsImage(_ text: String) -> Bool {
        let lowered = text.lowercased()
        return extensions.contains { lowered.contains("." + $0) }
    }

    /// The search itself: the string work first, and the disk only for a candidate that
    /// already looks like an image path.
    private static func search(for call: ToolCall, workingDirectory: String?) -> String? {
        var candidates: [String] = []
        if call.kind == .read {
            candidates.append(call.title)
            if let detail = call.detail, !detail.isEmpty { candidates.append(detail) }
        } else {
            for text in [call.title, call.detail ?? ""] where !text.isEmpty {
                let separators = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\"'`()<>,;"))
                candidates.append(contentsOf: text.components(separatedBy: separators))
            }
        }
        for candidate in candidates {
            let trimmed = candidate.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, isImagePath(trimmed) else { continue }
            let absolute: String
            if trimmed.hasPrefix("/") {
                absolute = trimmed
            } else if trimmed.hasPrefix("file://") {
                absolute = URL(string: trimmed)?.path ?? trimmed
            } else if trimmed.hasPrefix("~/") {
                absolute = (trimmed as NSString).expandingTildeInPath
            } else if let workingDirectory {
                absolute = (workingDirectory as NSString).appendingPathComponent(trimmed)
            } else {
                continue
            }
            if FileManager.default.fileExists(atPath: absolute) { return absolute }
        }
        return nil
    }

    /// One attachment value per file, so the preview panel's toggle recognises the same
    /// image on a second tap and closes instead of reopening.
    @MainActor private static var attachments: [String: Attachment] = [:]

    /// The file as an attachment, for the same large preview a sent photo opens in.
    @MainActor
    static func attachment(for path: String) -> Attachment {
        if let known = attachments[path] { return known }
        let name = (path as NSString).lastPathComponent
        let mimeType = UTType(filenameExtension: (path as NSString).pathExtension.lowercased())?.preferredMIMEType ?? "image/*"
        let attachment = Attachment(name: name, path: path, mimeType: mimeType.hasPrefix("image/") ? mimeType : "image/*")
        attachments[path] = attachment
        return attachment
    }

    // One formatter: allocating one per size string needlessly repeats the setup.
    @MainActor private static let sizeFormatter: ByteCountFormatter = {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter
    }()

    @MainActor
    static func fileSize(_ path: String) -> String? {
        guard let size = try? FileManager.default.attributesOfItem(atPath: path)[.size] as? NSNumber else { return nil }
        return sizeFormatter.string(fromByteCount: size.int64Value)
    }
}

/// Finder icons by extension, cached: one lookup per row per render stalled scrolling.
@MainActor
enum FileIcons {
    private static var cache: [String: NSImage] = [:]
    private static var order: [String] = []

    static func icon(for path: String) -> NSImage {
        let key = iconKey(for: path)
        if let hit = cache[key] { return hit }
        let image = NSWorkspace.shared.icon(forFile: path)
        cache[key] = image
        order.append(key)
        // Bounded: icons are small but unbounded kinds are not.
        while order.count > 64, let oldest = order.first {
            order.removeFirst()
            cache.removeValue(forKey: oldest)
        }
        return image
    }

    private static func iconKey(for path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        if !ext.isEmpty { return ext }
        var isDirectory: ObjCBool = false
        if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
            return "folder"
        }
        if let type = try? URL(fileURLWithPath: path).resourceValues(forKeys: [.contentTypeKey]).contentType {
            return "type:\(type.identifier)"
        }
        return "file"
    }
}

/// Video/movie attachments, shown with the same photo container.
enum PreviewVideos {
    static let extensions: Set<String> = [
        "mov", "mp4", "m4v", "mpg", "mpeg", "avi", "mkv", "webm",
    ]

    static func isVideoPath(_ path: String) -> Bool {
        extensions.contains((path as NSString).pathExtension.lowercased())
    }

    /// One video frame off the main actor, so opening a movie never stalls drawing.
    static func thumbnail(for path: String, pointSize: CGFloat) async -> CGImage? {
        await Task.detached(priority: .userInitiated) {
            let pixels = Int((pointSize * 2).rounded(.up))
            let asset = AVURLAsset(url: URL(fileURLWithPath: path))
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            generator.maximumSize = CGSize(width: pixels, height: pixels)
            var actualTime = CMTime.zero
            // A beat in, so a black first frame does not become the thumbnail.
            let time = CMTime(seconds: 0.5, preferredTimescale: 600)
            return try? generator.copyCGImage(at: time, actualTime: &actualTime)
        }.value
    }
}

extension Attachment {
    /// A movie by MIME type, or by extension when the MIME type is generic.
    var isVideo: Bool {
        if mimeType.hasPrefix("video/") { return true }
        return PreviewVideos.isVideoPath(path)
    }
}

/// The play badge drawn centered over a video thumbnail. It lives inside the
/// same clipped container as the photo, so the container style is preserved.
struct AttachmentPlayBadge: View {
    var body: some View {
        ZStack {
            Circle()
                .fill(.black.opacity(0.45))
                .frame(width: 30, height: 30)
            Image(systemName: "play.fill")
                .foregroundStyle(.white)
                .font(.system(size: 13, weight: .semibold))
                .offset(x: 1)
        }
        .accessibilityHidden(true)
    }
}

/// A list thumbnail for a movie that matches the photo thumbnail in
/// `AttachmentThumbnail`: same `size` box, same 12pt continuous rounding, same
/// `.quaternary` fill, same `.fill` image content, plus a centered play badge.
/// The badge sits inside the clipped stack, so the rounding is untouched.
struct AttachmentVideoThumbnail: View {
    let attachment: Attachment
    var size: CGFloat = 56

    @State private var image: CGImage?

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(.quaternary.opacity(0.6))
            if let image {
                Image(decorative: image, scale: 2)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .transition(.opacity)
            }
            AttachmentPlayBadge()
        }
        .frame(width: size, height: size)
        .clipShape(.rect(cornerRadius: 12, style: .continuous))
        .task(id: attachment.path) {
            guard image == nil else { return }
            guard let frame = await PreviewVideos.thumbnail(for: attachment.path, pointSize: size) else { return }
            withAnimation(.easeOut(duration: 0.15)) { image = frame }
        }
    }
}

/// A large preview shown in a popover when an attachment thumbnail is clicked.
///
/// The photo slot is sized up front (`imageSize`, see `imageDisplaySize`), so
/// the spinner and the loaded photo take exactly the same room: the panel is
/// sized once before it opens and never reflows under the arrow.
struct AttachmentLargePreview: View {
    let attachment: Attachment
    /// The photo's display size, precomputed by the panel. Nil for non-images.
    var imageSize: CGSize?

    @State private var image: CGImage?
    @State private var videoFrame: CGImage?
    @State private var textPreview: String?

    /// The largest a photo is shown at inside the panel.
    static let imageBounds = CGSize(width: 480, height: 360)

    /// The photo aspect-fitted into `imageBounds`, from the file's pixel size
    /// (orientation applied). Only the aspect matters, so this reads the
    /// header through ImageIO rather than decoding the image.
    static func imageDisplaySize(for attachment: Attachment) -> CGSize {
        var pixels = CGSize(width: 4, height: 3)
        if let source = CGImageSourceCreateWithURL(attachment.url as CFURL, nil),
           let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
           let width = properties[kCGImagePropertyPixelWidth] as? CGFloat,
           let height = properties[kCGImagePropertyPixelHeight] as? CGFloat,
           width > 0, height > 0 {
            let orientation = properties[kCGImagePropertyOrientation] as? UInt32 ?? 1
            pixels = orientation >= 5 ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
        }
        let scale = min(imageBounds.width / pixels.width, imageBounds.height / pixels.height)
        return CGSize(width: floor(pixels.width * scale), height: floor(pixels.height * scale))
    }

    /// The movie aspect-fitted into `imageBounds`, from the video track's
    /// natural size (preferred transform applied). Header-only, like the photo
    /// path above. Falls back to `imageBounds` when the track is unreadable.
    /// The coordinator can pass this as `imageSize` so the panel is sized up
    /// front exactly like a photo; the body uses it as the fallback too.
    static func videoDisplaySize(for attachment: Attachment) -> CGSize {
        let asset = AVURLAsset(url: attachment.url)
        if let track = asset.tracks(withMediaType: .video).first {
            var size = track.naturalSize.applying(track.preferredTransform)
            size = CGSize(width: abs(size.width), height: abs(size.height))
            if size.width > 0, size.height > 0 {
                let scale = min(imageBounds.width / size.width, imageBounds.height / size.height)
                return CGSize(width: floor(size.width * scale), height: floor(size.height * scale))
            }
        }
        return imageBounds
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if attachment.isImage {
                let slot = imageSize ?? Self.imageBounds
                ZStack {
                    if let image {
                        Image(decorative: image, scale: 2)
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                            .clipShape(.rect(cornerRadius: 12, style: .continuous))
                            .transition(.opacity)
                    } else {
                        ProgressView()
                    }
                }
                .frame(width: slot.width, height: slot.height)
                .frame(maxWidth: .infinity, alignment: .center)
            } else if attachment.isVideo {
                // Same container as the photo above: pre-sized slot, 12pt
                // continuous rounding, spinner until the frame lands. The play
                // badge sits inside the clipped image so the rounding holds.
                // The coordinator sizes the panel up front; here a fixed slot fills in.
                let slot = imageSize ?? Self.imageBounds
                ZStack {
                    if let videoFrame {
                        Image(decorative: videoFrame, scale: 2)
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                            .clipShape(.rect(cornerRadius: 12, style: .continuous))
                            .transition(.opacity)
                            .overlay { AttachmentPlayBadge() }
                    } else {
                        ProgressView()
                    }
                }
                .frame(width: slot.width, height: slot.height)
                .frame(maxWidth: .infinity, alignment: .center)
            } else if let textPreview {
                ScrollView {
                    Text(textPreview)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                }
                .frame(width: 440, height: min(320, max(120, CGFloat(textPreview.count / 4))))
                .background(.quaternary.opacity(0.45), in: .rect(cornerRadius: 10, style: .continuous))
            } else {
                HStack(spacing: 10) {
                    Image(nsImage: FileIcons.icon(for: attachment.path))
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(width: 32, height: 32)
                    Text(attachment.name)
                        .font(.callout)
                        .lineLimit(2)
                }
                .frame(width: 320, alignment: .leading)
                .padding(.vertical, 8)
            }

            HStack(spacing: 8) {
                Text(attachment.name)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let size = PreviewImages.fileSize(attachment.path) {
                    Text(size)
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
                Spacer(minLength: 12)
                Button("Show in Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([attachment.url])
                }
                .buttonStyle(.link)
                .font(.caption)
                Button("Open") {
                    NSWorkspace.shared.open(attachment.url)
                }
                .buttonStyle(.link)
                .font(.caption)
            }
        }
        .padding(12)
        .frame(maxWidth: 504)
        .task(id: attachment.path) {
            if attachment.isImage {
                let loaded = await ThumbnailCache.shared.thumbnail(for: attachment.path, pointSize: Self.imageBounds.width)?.image
                withAnimation(.easeOut(duration: 0.15)) { image = loaded }
            } else if attachment.isVideo {
                guard let frame = await PreviewVideos.thumbnail(for: attachment.path, pointSize: Self.imageBounds.width) else {
                    textPreview = Self.textPreview(for: attachment)
                    return
                }
                withAnimation(.easeOut(duration: 0.15)) { videoFrame = frame }
            } else {
                textPreview = Self.textPreview(for: attachment)
            }
        }
    }

    private static func textPreview(for attachment: Attachment) -> String? {
        let url = attachment.url
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size < 500_000 else { return nil }
        guard let text = try? String(contentsOf: url, encoding: .utf8), !text.isEmpty else { return nil }
        return String(text.prefix(8_000))
    }
}

/// Full-width image preview at the top of an expanded read tool row,
/// so it is clear which photo the agent looked at.
struct ToolImagePreview: View {
    let path: String

    @State private var image: CGImage?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(.quaternary.opacity(0.45))
                if let image {
                    Image(decorative: image, scale: 2)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .transition(.opacity)
                } else {
                    ProgressView()
                        .controlSize(.small)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: 420)
            .clipShape(.rect(cornerRadius: 10, style: .continuous))
            HStack(spacing: 8) {
                Text((path as NSString).lastPathComponent)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 8)
                Button("Open") {
                    NSWorkspace.shared.open(URL(fileURLWithPath: path))
                }
                .buttonStyle(.link)
                .font(.caption)
            }
        }
        .task(id: path) {
            image = await ThumbnailCache.shared.thumbnail(for: path, pointSize: 900)?.image
        }
    }
}
