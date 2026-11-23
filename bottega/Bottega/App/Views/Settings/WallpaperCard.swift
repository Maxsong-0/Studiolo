// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI
import UniformTypeIdentifiers

// A picture of the user's own behind the glass, with a sample showing how it
// sits under the app's glass sheet, and a slider to blur it for readability.
struct WallpaperCard: View {
    let store = WallpaperStore.shared
    @State private var isTargeted = false

    private var isOffStock: Bool { store.softness > 0.005 }

    var body: some View {
        ChromeCard {
            preview
            ChromeRowDivider()
            wallpaperRow
            if store.hasWallpaper {
                ChromeRowDivider()
                softnessRow
                ChromeRowDivider()
                blurRangeRow
                ChromeRowDivider()
                resolutionRow
            }
        }
    }

    private var preview: some View {
        Button { choose() } label: {
            ZStack {
                if let image = store.image {
                    Image(nsImage: image)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(maxWidth: .infinity)
                        .frame(height: 150)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .overlay {
                            // A small stand-in for the app window, so the picture reads
                            // as sitting behind the glass rather than as a photo itself.
                            GeometryReader { geo in
                                let mockShape = RoundedRectangle(cornerRadius: 8, style: .continuous)
                                mockShape
                                    .fill(.clear)
                                    .glassEffect(in: mockShape)
                                    .overlay { mockShape.fill(Color.black.opacity(0.2)) }
                                    .overlay(alignment: .topLeading) {
                                        HStack(spacing: 4) {
                                            ForEach(0..<3, id: \.self) { _ in
                                                Circle()
                                                    .fill(Chrome.overlay(0.35))
                                                    .frame(width: 5, height: 5)
                                            }
                                        }
                                        .padding(.top, 8)
                                        .padding(.leading, 8)
                                    }
                                    .frame(width: geo.size.width * 0.6, height: geo.size.height * 0.6)
                                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                            }
                        }
                        .transition(.scale(scale: 0.96).combined(with: .opacity))
                } else {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(Chrome.overlay(0.04))
                        .frame(height: 150)
                        .overlay {
                            VStack(spacing: 6) {
                                Image(systemName: "photo.on.rectangle.angled")
                                    .font(.system(size: 22))
                                    .foregroundStyle(Chrome.secondaryText)
                                Text("Drop a picture here, or choose one")
                                    .font(.system(size: 12))
                                    .foregroundStyle(Chrome.secondaryText)
                            }
                        }
                        .transition(.scale(scale: 0.96).combined(with: .opacity))
                }
                if store.isInstalling {
                    ProgressView()
                        .controlSize(.small)
                }
                if isTargeted {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(Chrome.accent.opacity(0.18))
                        .frame(height: 150)
                        .transition(.opacity)
                }
            }
            .frame(height: 150)
            .frame(maxWidth: .infinity)
            .scaleEffect(isTargeted ? 1.02 : 1)
            .animation(.snappy(duration: 0.2), value: isTargeted)
            .animation(.spring(duration: 0.45, bounce: 0.2), value: store.image != nil)
        }
        .buttonStyle(.plain)
        .contentShape(.rect)
        .dropDestination(for: URL.self, action: { urls, _ in
            guard let url = urls.first(where: WallpaperStore.accepts) else { return false }
            store.install(from: url)
            return true
        }, isTargeted: { isTargeted = $0 })
        .padding(16)
        .frame(maxWidth: .infinity)
    }

    private var wallpaperRow: some View {
        ChromeRow(
            title: "Wallpaper",
            detail: store.lastError ?? (store.hasWallpaper
                ? "Fills the window edge to edge under the glass; the transparency slider lets the desktop show through it"
                : "A picture of your own behind the glass, mixed with the desktop by the transparency slider")
        ) {
            HStack(spacing: 8) {
                if store.hasWallpaper {
                    Button("Remove") { store.remove() }
                        .buttonStyle(.glass)
                        .controlSize(.small)
                        .disabled(store.isInstalling)
                }
                Button(store.hasWallpaper ? "Change…" : "Choose…") { choose() }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                    .disabled(store.isInstalling)
            }
        }
    }

    private var softnessRow: some View {
        ChromeRow(title: "Softness", detail: "Blurs the picture so text stays easy to read") {
            HStack(spacing: 8) {
                Image(systemName: "drop")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .help("Sharp")
                Slider(
                    value: Binding(get: { store.softness }, set: { store.softness = $0 }),
                    in: 0...1,
                    onEditingChanged: { editing in store.isAdjustingSoftness = editing }
                )
                .controlSize(.small)
                .frame(width: 140)
                Image(systemName: "drop.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .help("Blurred")
            }
            .overlay(alignment: .leading) {
                if isOffStock {
                    Button {
                        store.softness = 0
                    } label: {
                        Image(systemName: "arrow.counterclockwise")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(Chrome.secondaryText)
                            .frame(width: 22, height: 22)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .help("Back to sharp")
                    .offset(x: -28)
                    .transition(.opacity.combined(with: .offset(x: 6)))
                }
            }
            .animation(.snappy(duration: 0.2), value: isOffStock)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text("Wallpaper softness"))
        }
    }

    /// How far the softness slider reaches: the stock blur, or heavier for a frosted look.
    private var blurRangeRow: some View {
        ChromeRow(title: "Blur strength", detail: "How blurred the picture gets with softness all the way up") {
            ChromeSegmentedPicker(
                options: WallpaperBlurRange.allCases.map { ChromeSegmentedOption(value: $0, title: $0.title) },
                selection: Binding(get: { store.blurRange }, set: { store.blurRange = $0 })
            )
        }
    }

    /// How large the picture is kept, for a display too sharp for the standard size.
    private var resolutionRow: some View {
        ChromeRow(title: "Resolution", detail: "High keeps the picture sharp on a 5K or 6K display, at more memory") {
            ChromeSegmentedPicker(
                options: WallpaperResolution.allCases.map { ChromeSegmentedOption(value: $0, title: $0.title) },
                selection: Binding(get: { store.resolution }, set: { store.resolution = $0 })
            )
        }
    }

    private func choose() {
        // The sheet must not block, so the picker runs async against the key window.
        let panel = NSOpenPanel()
        panel.allowedContentTypes = WallpaperStore.acceptedTypes
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = "Choose a picture for the window"
        panel.prompt = "Use as Wallpaper"
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window) { response in
                if response == .OK, let url = panel.url {
                    store.install(from: url)
                }
            }
        } else {
            panel.begin { response in
                if response == .OK, let url = panel.url {
                    store.install(from: url)
                }
            }
        }
    }
}
