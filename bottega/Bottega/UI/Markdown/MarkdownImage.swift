// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// A picture on a line of its own in Markdown, `![alt](picture.png "title")`, and where a
/// page that shows notes finds it. Chat never draws pictures: a reply's `![…](…)` stays
/// the text it is, and nothing is fetched from wherever it points. A page that can show
/// them (the archive page) says how a reference becomes a file through `markdownImages`.
enum MarkdownImage {
    struct Reference: Equatable {
        let alt: String
        /// The destination as written, percent-encoding and all.
        let reference: String
    }

    /// The pictures drawn: what `NSImage` reads and Obsidian embeds.
    static let extensions: Set<String> = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "heic", "svg"]

    /// The image a paragraph is, when it is one and nothing else.
    static func parse(_ paragraph: String) -> Reference? {
        let text = paragraph.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.hasPrefix("!["), text.hasSuffix(")"), !text.contains("\n"),
              let close = text.range(of: "](") else { return nil }
        let alt = String(text[text.index(text.startIndex, offsetBy: 2)..<close.lowerBound])
        guard !alt.contains("]") else { return nil }
        var inside = String(text[close.upperBound..<text.index(before: text.endIndex)])
        // A title after the destination goes.
        if let quote = inside.firstIndex(of: "\""), inside.hasSuffix("\"") {
            inside = String(inside[..<quote])
        }
        let destination = inside.trimmingCharacters(in: .whitespaces)
        guard !destination.isEmpty, !destination.contains(" "), !destination.contains("("), !destination.contains(")") else { return nil }
        return Reference(alt: alt, reference: destination)
    }
}

/// What turns a picture's reference into the file to draw; nil (the default, and chat's)
/// draws none.
private struct MarkdownImagesKey: EnvironmentKey {
    static let defaultValue: (@MainActor (String) -> URL?)? = nil
}

extension EnvironmentValues {
    var markdownImages: (@MainActor (String) -> URL?)? {
        get { self[MarkdownImagesKey.self] }
        set { self[MarkdownImagesKey.self] = newValue }
    }
}

/// One picture, read off the main thread and drawn at its size or the column's width,
/// whichever is less; its alt text where it cannot be read.
struct MarkdownImageView: View {
    let url: URL
    let alt: String

    @State private var image: NSImage?
    @State private var failed = false

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: min(image.size.width, 760), maxHeight: 640, alignment: .leading)
                    .clipShape(.rect(cornerRadius: 6, style: .continuous))
            } else if failed {
                Label(alt.isEmpty ? url.lastPathComponent : alt, systemImage: "photo")
                    .foregroundStyle(.secondary)
            } else {
                Color.clear.frame(height: 24)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .help(alt.isEmpty ? url.lastPathComponent : alt)
        .accessibilityLabel(Text(verbatim: alt.isEmpty ? url.lastPathComponent : alt))
        .task(id: url) {
            // The bytes come off the main thread; the image is made here, where it is drawn.
            let data = await Task.detached(priority: .userInitiated) { try? Data(contentsOf: url) }.value
            guard !Task.isCancelled else { return }
            if let data, let loaded = NSImage(data: data) { image = loaded } else { failed = true }
        }
    }
}
