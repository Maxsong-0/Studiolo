import AppKit
import SwiftUI

/// The sizes the conversation reads at, smallest to largest. Settings › General steps
/// through them, with a sample of a chat drawn at the chosen one, and keeps the index.
enum ChatZoom {
    /// What each step reads as against the default, in percent. macOS has no Dynamic Type,
    /// so the conversation scales its own fonts by this, through the `chatZoom` environment
    /// value and `Font.chat`.
    private static let percentages = [80, 90, 95, 100, 110, 125, 135]
    static var stepCount: Int { percentages.count }
    /// The 100% step: a fresh install reads exactly as it did before the setting.
    static let defaultIndex = 3

    /// The index brought into range, for a stored value from another build.
    static func clamped(_ index: Int) -> Int {
        min(stepCount - 1, max(0, index))
    }

    static func percentage(at index: Int) -> Int {
        percentages[clamped(index)]
    }

    /// The factor the conversation's type is drawn at for a step: 1 at the default step.
    static func scale(at index: Int) -> CGFloat {
        CGFloat(percentage(at: index)) / 100
    }
}

private struct ChatZoomKey: EnvironmentKey {
    static let defaultValue: CGFloat = 1
}

extension EnvironmentValues {
    /// The factor the conversation's text is drawn at, from the text size in Settings.
    /// Views outside the conversation never see it and keep their size.
    var chatZoom: CGFloat {
        get { self[ChatZoomKey.self] }
        set { self[ChatZoomKey.self] = newValue }
    }
}

extension Font {
    /// The point size macOS lays a text style out at, for `chat`.
    static func chatBaseSize(_ style: Font.TextStyle) -> CGFloat {
        switch style {
        case .largeTitle: 26
        case .title: 22
        case .title2: 17
        case .title3: 15
        case .headline, .body: 13
        case .callout: 12
        case .subheadline: 11
        case .footnote, .caption, .caption2: 10
        @unknown default: 13
        }
    }

    /// A text style at the conversation's zoom: the plain style at zoom 1, so the default
    /// reads exactly as before, and the same size scaled otherwise. `headline` keeps its
    /// semibold weight unless another is asked for.
    static func chat(_ style: Font.TextStyle, weight: Font.Weight? = nil, design: Font.Design = .default, zoom: CGFloat) -> Font {
        let resolvedWeight = weight ?? (style == .headline ? .semibold : nil)
        if zoom == 1 {
            let font = Font.system(style, design: design)
            return resolvedWeight.map { font.weight($0) } ?? font
        }
        let size = (chatBaseSize(style) * zoom).rounded()
        return Font.system(size: size, weight: resolvedWeight ?? .regular, design: design)
    }
}
