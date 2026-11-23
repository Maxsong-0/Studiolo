// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import CoreImage
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

/// An NSImage crossing from the detached decode work back to the main actor. The picture
/// is fully decoded before it crosses and only ever touched on the main actor after.
private struct DecodedPicture: @unchecked Sendable {
    let image: NSImage
}

/// How far the softness slider can blur the wallpaper: the blur at full softness. Standard
/// is the stock blur; the heavier ones are for a picture that should read as frosted glass.
enum WallpaperBlurRange: String, CaseIterable, Sendable {
    case standard
    case strong
    case frosted

    var title: String {
        switch self {
        case .standard: "Standard"
        case .strong: "Strong"
        case .frosted: "Frosted"
        }
    }

    /// The blur radius at full softness, in pixels of a picture whose long side is
    /// `WallpaperStore.referencePixel` (see `WallpaperStore.blurRadius`).
    var maxRadius: Double {
        switch self {
        case .standard: 28
        case .strong: 56
        case .frosted: 96
        }
    }
}

/// How large the wallpaper is kept: the long side it is downsampled to. Standard fills a
/// window up to 3K; high keeps the picture sharp across a 5K or 6K display, at about four
/// times the memory.
enum WallpaperResolution: String, CaseIterable, Sendable {
    case standard
    case high

    var title: String {
        switch self {
        case .standard: "Standard"
        case .high: "High"
        }
    }

    /// The long side the full-size picture is downsampled to.
    var maxPixel: Int {
        switch self {
        case .standard: 3200
        case .high: 6144
        }
    }

    /// The half-size copy the softness slider renders from while it is dragged.
    var previewMaxPixel: Int { maxPixel / 2 }
}

/// The window's picked picture: one file kept in the library, handed out decoded,
/// downsampled and optionally softened for the backdrop.
///
/// The file lives in the app's support folder (`Storage`), which the UI layer does not depend
/// on, so the store sits beside it in Services; the backdrop that shows the picture stays in
/// UI and takes it as a plain image (`WindowBackdrop`).
@MainActor @Observable
final class WallpaperStore {
    static let shared = WallpaperStore()

    private enum Key {
        static let softness = "wallpaperSoftness"
        static let file = "wallpaperFile"
        static let blurRange = "wallpaperBlurRange"
        static let resolution = "wallpaperResolution"
    }

    /// One blur context for every render: making one per picture stalls the softness slider.
    /// The context draws from any thread, so it sits outside the main actor.
    nonisolated static let ciContext = CIContext(options: [.useSoftwareRenderer: false])

    /// The long side the blur radii are measured against: the standard resolution's. A
    /// larger picture blurs by a proportionally larger radius, so a softness looks the same
    /// at every resolution (see `blurRadius`).
    nonisolated static let referencePixel = Double(WallpaperResolution.standard.maxPixel)

    static let acceptedTypes: [UTType] = [.image]

    /// The rendered picture for the backdrop, downsampled and blurred per `softness`.
    /// Nil when no wallpaper is set.
    private(set) var image: NSImage?
    /// True while a picked file is being copied and decoded.
    private(set) var isInstalling = false
    /// A short sentence for the settings card when a pick fails. Nil otherwise.
    private(set) var lastError: String?

    /// How soft the picture renders, 0 crisp to 1 fully blurred. Saved at once, and a
    /// change re-renders: live from the small copy while the slider is dragged and at
    /// full size when the drag ends.
    var softness: Double {
        didSet {
            // Writing the property from its own observer calls the observer again, and an
            // unconditional clamp here recursed until the stack ran out: dragging the
            // softness slider crashed the app. Only write back a value that was really out
            // of range, and let that one extra pass do the saving and the re-render.
            let clamped = min(1, max(0, softness))
            guard clamped == softness else {
                softness = clamped
                return
            }
            defaults.set(softness, forKey: Key.softness)
            scheduleRerender()
        }
    }

    /// How far full softness blurs the picture. Saved at once, and a change re-renders.
    var blurRange: WallpaperBlurRange {
        didSet {
            guard blurRange != oldValue else { return }
            defaults.set(blurRange.rawValue, forKey: Key.blurRange)
            scheduleRerender()
        }
    }

    /// How large the picture is kept. Saved at once, and a change renders the picture
    /// again from its file at the new size.
    var resolution: WallpaperResolution {
        didSet {
            guard resolution != oldValue else { return }
            defaults.set(resolution.rawValue, forKey: Key.resolution)
            scheduleRerender()
        }
    }

    var hasWallpaper: Bool { image != nil || fileURL != nil }

    /// True while the softness slider is being dragged: each change renders live from the
    /// small copy, and the full-size render happens once when the drag ends.
    var isAdjustingSoftness = false {
        didSet {
            guard isAdjustingSoftness != oldValue, fileURL != nil else { return }
            if isAdjustingSoftness {
                renderLive()
            } else {
                liveRequested = nil
                render()
            }
        }
    }

    /// The installed file inside the library. Ignored by observation: views watch `image`.
    @ObservationIgnored private var fileURL: URL?
    @ObservationIgnored private let defaults = UserDefaults.standard
    @ObservationIgnored private var renderTask: Task<Void, Never>?
    @ObservationIgnored private var liveTask: Task<Void, Never>?
    // The live softness render in flight; one at a time, newest value wins.
    @ObservationIgnored private var liveRequested: Double?
    // The downsampled picture the renders start from, unblurred.
    @ObservationIgnored private var baseImage: CGImage?
    // A half-size copy of the same picture, so a drag renders live in half the time.
    @ObservationIgnored private var previewBase: CGImage?

    private init() {
        let stored = defaults.object(forKey: Key.softness) as? Double ?? 0
        softness = min(1, max(0, stored))
        blurRange = defaults.string(forKey: Key.blurRange).flatMap(WallpaperBlurRange.init(rawValue:)) ?? .standard
        resolution = defaults.string(forKey: Key.resolution).flatMap(WallpaperResolution.init(rawValue:)) ?? .standard
        if let name = defaults.string(forKey: Key.file), !name.isEmpty {
            let url = Storage.wallpaperDirectory.appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: url.path) {
                fileURL = url
                render()
            }
        }
    }

    /// True when the URL points at a picture, by its extension or its content type.
    nonisolated static func accepts(_ url: URL) -> Bool {
        if !url.pathExtension.isEmpty,
           let type = UTType(filenameExtension: url.pathExtension),
           type.conforms(to: .image) {
            return true
        }
        if let type = try? url.resourceValues(forKeys: [.contentTypeKey]).contentType,
           type.conforms(to: .image) {
            return true
        }
        return false
    }

    /// Copies a picked picture into the library and renders it. A pick that is not a
    /// picture or cannot be used reports a sentence and leaves the current wallpaper alone.
    func install(from source: URL) {
        guard Self.accepts(source) else {
            lastError = "That is not a picture."
            return
        }
        isInstalling = true
        lastError = nil
        renderTask?.cancel()
        let softness = self.softness
        let maxRadius = blurRange.maxRadius
        let maxPixel = resolution.maxPixel
        Task.detached(priority: .userInitiated) { [softness, maxRadius, maxPixel] in
            // The panel hands over a scoped URL, which dies with the panel, so the copy
            // takes its access with it.
            let scoped = source.startAccessingSecurityScopedResource()
            defer { if scoped { source.stopAccessingSecurityScopedResource() } }
            let ext = source.pathExtension.isEmpty ? "jpg" : source.pathExtension.lowercased()
            let destination = Storage.wallpaperDirectory.appendingPathComponent("wallpaper-\(UUID().uuidString).\(ext)")
            do {
                try FileManager.default.createDirectory(at: Storage.wallpaperDirectory, withIntermediateDirectories: true)
                try FileManager.default.copyItem(at: source, to: destination)
            } catch {
                try? FileManager.default.removeItem(at: destination)
                await WallpaperStore.shared.failInstall()
                return
            }
            guard let decoded = Self.decode(destination, softness: softness, maxRadius: maxRadius, maxPixel: maxPixel) else {
                try? FileManager.default.removeItem(at: destination)
                await WallpaperStore.shared.failInstall()
                return
            }
            await WallpaperStore.shared.finishInstall(file: destination, picture: DecodedPicture(image: decoded))
        }
    }

    /// Drops the wallpaper: the file goes, the backdrop clears, the complaint clears.
    func remove() {
        renderTask?.cancel()
        if let fileURL { try? FileManager.default.removeItem(at: fileURL) }
        self.fileURL = nil
        defaults.removeObject(forKey: Key.file)
        withAnimation(.easeOut(duration: 0.25)) {
            image = nil
        }
        lastError = nil
    }

    /// Renders the wallpaper at full size from the installed file: downsample to the chosen
    /// resolution, then blur by the chosen range. A newer render cancels this one; a
    /// cancelled or failed render leaves the picture alone.
    private func render() {
        guard let fileURL else { return }
        let softness = self.softness
        let maxRadius = blurRange.maxRadius
        let fullPixel = resolution.maxPixel
        let previewPixel = resolution.previewMaxPixel
        renderTask?.cancel()
        renderTask = Task { [weak self, fileURL, softness, maxRadius, fullPixel, previewPixel] in
            let bases = await Task.detached(priority: .userInitiated) { () -> (CGImage?, CGImage?) in
                (Self.thumbnail(fileURL, maxPixel: fullPixel), Self.thumbnail(fileURL, maxPixel: previewPixel))
            }.value
            guard let base = bases.0 else { return }
            let rendered = await Task.detached(priority: .userInitiated) { () -> CGImage? in
                let radius = Self.blurRadius(softness: softness, maxRadius: maxRadius, longSide: max(base.width, base.height))
                return softness > 0 ? Self.blurred(base, radius: radius) : base
            }.value
            guard let self, !Task.isCancelled, self.fileURL == fileURL, let rendered else { return }
            self.baseImage = base
            self.previewBase = bases.1
            // Deliberately no animation here: an animated bitmap swap makes the picture flash.
            self.image = NSImage(cgImage: rendered, size: NSSize(width: rendered.width, height: rendered.height))
        }
    }

    private func scheduleRerender() {
        guard fileURL != nil else { return }
        if isAdjustingSoftness {
            renderLive()
        } else {
            render()
        }
    }

    /// Renders the live preview for the current drag: one render at a time, and a softness
    /// that arrives while one is in flight is rendered as soon as it finishes, so a fast
    /// drag never queues work or stalls the slider.
    private func renderLive() {
        guard let previewBase, let baseImage else {
            render()
            return
        }
        liveRequested = softness
        guard liveTask == nil else { return }
        // The small copy blurs by the full-size radius scaled down to its size, so the
        // preview under the drag looks the way the full render will.
        let scale = Double(previewBase.width) / Double(baseImage.width)
        let maxRadius = blurRange.maxRadius
        let fullLongSide = max(baseImage.width, baseImage.height)
        liveTask = Task { [weak self] in
            while let value = self?.takeLiveRequest() {
                let radius = WallpaperStore.blurRadius(softness: value, maxRadius: maxRadius, longSide: fullLongSide) * scale
                let rendered = await Task.detached(priority: .userInitiated) {
                    WallpaperStore.blurred(previewBase, radius: radius)
                }.value
                guard let self, let rendered, self.isAdjustingSoftness else { break }
                self.image = NSImage(cgImage: rendered, size: NSSize(width: rendered.width, height: rendered.height))
            }
            self?.liveTask = nil
        }
    }

    /// The newest softness asked for since the last live render, or nil when the drag is over.
    private func takeLiveRequest() -> Double? {
        guard isAdjustingSoftness, let value = liveRequested else { return nil }
        liveRequested = nil
        return value
    }

    private func finishInstall(file: URL, picture: DecodedPicture) {
        if let previous = fileURL { try? FileManager.default.removeItem(at: previous) }
        fileURL = file
        defaults.set(file.lastPathComponent, forKey: Key.file)
        withAnimation(.spring(duration: 0.45, bounce: 0.2)) {
            image = picture.image
        }
        isInstalling = false
    }

    private func failInstall() {
        isInstalling = false
        lastError = "Could not use that picture."
    }

    /// Downsamples the file to at most `maxPixel` px. Runs anywhere: ImageIO does the work,
    /// and nothing here touches the main actor.
    nonisolated static func thumbnail(_ url: URL, maxPixel: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    /// The blur radius for `softness` on a picture whose long side is `longSide` pixels: the
    /// range's radius at full softness, grown for a picture larger than `referencePixel`, so
    /// a softness blurs a high-resolution picture as much, to the eye, as a standard one.
    nonisolated static func blurRadius(softness: Double, maxRadius: Double, longSide: Int) -> Double {
        softness * maxRadius * max(1, Double(longSide) / referencePixel)
    }

    /// Softens the base picture by `radius` pixels (see `blurRadius`). Runs anywhere: Core
    /// Image does the work.
    nonisolated static func blurred(_ base: CGImage, radius: Double) -> CGImage? {
        guard radius > 0.01 else { return base }
        guard let blur = CIFilter(name: "CIGaussianBlur") else { return base }
        // The blur shrinks opaque edges toward transparent, so the image is clamped first
        // and cropped back to its own extent: the middle softens while the frame stays solid.
        let extent = CGRect(origin: .zero, size: CGSize(width: base.width, height: base.height))
        blur.setValue(CIImage(cgImage: base).clampedToExtent(), forKey: kCIInputImageKey)
        blur.setValue(radius, forKey: kCIInputRadiusKey)
        guard let output = blur.outputImage?.cropped(to: extent) else { return nil }
        return ciContext.createCGImage(output, from: extent)
    }

    /// Downsamples the file to at most `maxPixel` px and softens it per `softness` within
    /// the blur range's `maxRadius`. Runs anywhere: ImageIO and Core Image do the work, and
    /// nothing here touches the main actor.
    nonisolated static func decode(_ url: URL, softness: Double, maxRadius: Double, maxPixel: Int) -> NSImage? {
        guard let base = thumbnail(url, maxPixel: maxPixel) else { return nil }
        let radius = blurRadius(softness: softness, maxRadius: maxRadius, longSide: max(base.width, base.height))
        let rendered = softness > 0 ? blurred(base, radius: radius) : base
        guard let rendered else { return nil }
        return NSImage(cgImage: rendered, size: NSSize(width: rendered.width, height: rendered.height))
    }
}
