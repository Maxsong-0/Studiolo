import AppKit
import SwiftUI

struct ProviderIcon: View {
    let provider: ProviderKind
    var size: CGFloat = 14

    var body: some View {
        Image(provider.iconName)
            .resizable()
            .aspectRatio(contentMode: .fit)
            .frame(width: size, height: size)
    }
}

enum RelativeTime {
    /// Dates older than a week by their formatted day: every sidebar row past that age
    /// asked ICU again on each evaluation of the list.
    @MainActor private static var days = RecentCache<Date, String>(limit: 256)

    @MainActor
    static func short(_ date: Date, now: Date = .now) -> String {
        let seconds = now.timeIntervalSince(date)
        switch seconds {
        case ..<60: return "now"
        case ..<3_600: return "\(Int(seconds / 60))m"
        case ..<86_400: return "\(Int(seconds / 3_600))h"
        case ..<604_800: return "\(Int(seconds / 86_400))d"
        default:
            if let day = days.value(for: date) { return day }
            let day = date.formatted(.dateTime.month(.abbreviated).day())
            days.insert(day, for: date)
            return day
        }
    }

    static func duration(_ interval: TimeInterval) -> String {
        let total = max(0, Int(interval.rounded()))
        if total < 60 { return "\(total)s" }
        if total < 3_600 { return "\(total / 60)m \(total % 60)s" }
        return "\(total / 3_600)h \((total % 3_600) / 60)m"
    }
}

struct DiffStatLabel: View {
    let additions: Int
    let deletions: Int

    var body: some View {
        HStack(spacing: 4) {
            Text("+\(additions)").foregroundStyle(Chrome.success)
            Text("−\(deletions)").foregroundStyle(Chrome.danger)
        }
        .font(.caption.monospacedDigit())
    }
}

/// A compact, borderless control used inside glass surfaces, where nested glass would muddy the material.
struct ChipButtonStyle: ButtonStyle {
    var isActive = false

    func makeBody(configuration: Configuration) -> some View {
        ChipBody(configuration: configuration, isActive: isActive)
    }

    private struct ChipBody: View {
        let configuration: Configuration
        let isActive: Bool
        @State private var isHovering = false

        var body: some View {
            configuration.label
                .font(.callout)
                .lineLimit(1)
                .padding(.horizontal, 9)
                .padding(.vertical, 5)
                .foregroundStyle(isActive ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                .background {
                    Capsule().fill(fill)
                }
                .contentShape(.capsule)
                .onHover { isHovering = $0 }
                .animation(.easeOut(duration: 0.12), value: isHovering)
        }

        private var fill: AnyShapeStyle {
            if isActive { return AnyShapeStyle(.tint.opacity(configuration.isPressed ? 0.26 : 0.16)) }
            if configuration.isPressed { return AnyShapeStyle(.primary.opacity(0.12)) }
            return AnyShapeStyle(.primary.opacity(isHovering ? 0.07 : 0))
        }
    }
}

extension ButtonStyle where Self == ChipButtonStyle {
    static var chip: ChipButtonStyle { ChipButtonStyle() }

    static func chip(active: Bool) -> ChipButtonStyle {
        ChipButtonStyle(isActive: active)
    }
}

struct HoverRevealModifier: ViewModifier {
    @State private var isHovering = false
    let content: (Bool) -> AnyView

    func body(content base: Content) -> some View {
        base
            .overlay(alignment: .topTrailing) { self.content(isHovering) }
            .onHover { isHovering = $0 }
    }
}

enum Workspace {
    struct Editor: Identifiable {
        var id: String { bundleID }
        var name: String
        var bundleID: String
    }

    static let editors = [
        Editor(name: "Visual Studio Code", bundleID: "com.microsoft.VSCode"),
        Editor(name: "Cursor", bundleID: "com.todesktop.230313mzl4w4u92"),
        Editor(name: "Zed", bundleID: "dev.zed.Zed"),
        Editor(name: "Xcode", bundleID: "com.apple.dt.Xcode"),
        Editor(name: "Terminal", bundleID: "com.apple.Terminal"),
        Editor(name: "Ghostty", bundleID: "com.mitchellh.ghostty"),
        Editor(name: "iTerm", bundleID: "com.googlecode.iterm2"),
    ]

    /// Looked up once per launch: the chat's controls build this list on every render, and each
    /// entry is a Launch Services query.
    static let installedEditors: [Editor] = editors.filter {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: $0.bundleID) != nil
    }

    static func open(_ path: String, with editor: Editor) {
        guard let application = NSWorkspace.shared.urlForApplication(withBundleIdentifier: editor.bundleID) else { return }
        NSWorkspace.shared.open([URL(fileURLWithPath: path)], withApplicationAt: application, configuration: NSWorkspace.OpenConfiguration())
    }

    /// Icons by bundle id. Each is a Launch Services lookup plus an icon read, and the "Open in"
    /// menu asks for every editor's whenever the chat's chrome is rebuilt.
    @MainActor private static var icons: [String: NSImage] = [:]

    @MainActor
    static func icon(for editor: Editor) -> NSImage? {
        if let cached = icons[editor.bundleID] { return cached }
        guard let application = NSWorkspace.shared.urlForApplication(withBundleIdentifier: editor.bundleID) else { return nil }
        let image = NSWorkspace.shared.icon(forFile: application.path)
        image.size = NSSize(width: 16, height: 16)
        icons[editor.bundleID] = image
        return image
    }

    /// Finder's icon, for the same menu.
    @MainActor static let finderIcon = NSWorkspace.shared.icon(forFile: "/System/Library/CoreServices/Finder.app")

    static func revealInFinder(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }
}
