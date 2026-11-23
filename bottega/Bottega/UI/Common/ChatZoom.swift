// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The sizes the main window reads at, smallest to largest. Settings › General steps
/// through them, with a sample of a chat drawn at the chosen one; the choice is kept as
/// its percentage (see `AppSettings.chatZoom`), so a scale with more steps never reads a
/// stored choice as another size.
enum ChatZoom {
    /// What each step reads as against the default, in percent. macOS has no Dynamic Type,
    /// so the window scales its own fonts by this, through the `chatZoom` environment value:
    /// the conversation with `Font.chat`, the chrome around it (the sidebar, the chat box,
    /// the chrome row, the right sidebar, the panels) with `Font.zoomed` and `zoomedFont`.
    private static let percentages = [70, 80, 90, 95, 100, 110, 125, 135, 150, 160]
    static var stepCount: Int { percentages.count }
    /// The 100% step: a fresh install reads exactly as it did before the setting.
    static let defaultIndex = 4

    /// The scale earlier builds had, by the index they stored: 80 to 135%.
    private static let legacyPercentages = [80, 90, 95, 100, 110, 125, 135]

    /// The conversation's prose at the stock step, in points: the markdown's own size
    /// wherever no smaller one is set (`markdownPointSize` defaults to it) and the bubble of
    /// a message sent. `Font.chatBaseSize(.body)` says 14, and the text-size sample in
    /// Settings is drawn at that; nothing in the conversation reads that entry, and the prose
    /// has always been drawn at this size.
    static let proseBaseSize: CGFloat = 13

    /// The terminal's type at the stock step, in points, until Settings sets another.
    static let terminalBaseSize: CGFloat = 12

    /// The index brought into range, for a stored value from another build.
    static func clamped(_ index: Int) -> Int {
        min(stepCount - 1, max(0, index))
    }

    static func percentage(at index: Int) -> Int {
        percentages[clamped(index)]
    }

    /// The step nearest a stored percentage; a tie goes to the smaller step.
    static func index(forPercentage percentage: Int) -> Int {
        percentages.indices.min { abs(percentages[$0] - percentage) < abs(percentages[$1] - percentage) } ?? defaultIndex
    }

    /// The step an index stored by an earlier build meant, read on that build's scale.
    static func index(forLegacyIndex index: Int) -> Int {
        self.index(forPercentage: legacyPercentages[min(legacyPercentages.count - 1, max(0, index))])
    }

    /// The factor the window's type is drawn at for a step: 1 at the default step.
    static func scale(at index: Int) -> CGFloat {
        CGFloat(percentage(at: index)) / 100
    }

    /// A stock point size at `zoom`: exactly the size given at the stock step, so the
    /// stock look is untouched to the fraction, and the scaled size to the half point at
    /// any other step.
    static func scaled(_ size: CGFloat, by zoom: CGFloat) -> CGFloat {
        zoom == 1 ? size : (size * zoom * 2).rounded() / 2
    }
}

private struct ChatZoomKey: EnvironmentKey {
    static let defaultValue: CGFloat = 1
}

extension EnvironmentValues {
    /// The factor the main window's text is drawn at, from the text size in Settings (see
    /// `WindowTypeEnvironment`): the conversation, the sidebar, the chat box, the chrome
    /// row, the right sidebar and the panels all read it. Views that never see it, the
    /// Settings window and the popovers AppKit hosts on their own, keep their stock size.
    var chatZoom: CGFloat {
        get { self[ChatZoomKey.self] }
        set { self[ChatZoomKey.self] = newValue }
    }

    /// The faces the reader picked for the conversation's prose and code (see
    /// `ChatTypefaces`), set beside `chatZoom`.
    @Entry var chatTypefaces = ChatTypefaces()

    /// The terminal's type at the stock step, in points, from Settings; the terminal scales
    /// it by `chatZoom` like the rest of the window and sets it in the code face.
    @Entry var terminalFontSize: CGFloat = ChatZoom.terminalBaseSize
}

/// The families the reader picked for the conversation's prose and its code, nil for the
/// system's own faces. Only the prose (the markdown and a sent message's bubble), the code
/// blocks and the terminal are set in them; every control keeps the system face.
struct ChatTypefaces: Hashable, Sendable {
    var text: String?
    var code: String?
}

extension Font {
    /// The point size macOS lays a text style out at, for `chat`.
    static func chatBaseSize(_ style: Font.TextStyle) -> CGFloat {
        switch style {
        case .largeTitle: 26
        case .title: 22
        case .title2: 17
        case .title3: 15
        case .headline, .body: 14
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
        // Always sized from the table, so the chat's own base sizes (body 14) apply at zoom 1
        // too; the table mirrors the system styles for everything else.
        let size = (chatBaseSize(style) * zoom).rounded()
        return Font.system(size: size, weight: resolvedWeight ?? .regular, design: design)
    }

    /// `chat` in the family the reader picked for the prose or the code (see
    /// `ChatTypefaces`): the system face, as above, while `family` is nil or not installed.
    @MainActor
    static func chat(_ style: Font.TextStyle, weight: Font.Weight? = nil, design: Font.Design = .default, zoom: CGFloat, family: String?) -> Font {
        guard let family else { return chat(style, weight: weight, design: design, zoom: zoom) }
        let resolvedWeight = weight ?? (style == .headline ? .semibold : .regular)
        return Typefaces.font(family: family, size: (chatBaseSize(style) * zoom).rounded(), weight: resolvedWeight, design: design)
    }

    /// The conversation's prose at a base size of its own (the markdown's point size), at
    /// the window's text size and in the reader's family: the system face at `size` when
    /// `family` is nil, so the stock step reads exactly as the plain face did.
    @MainActor
    static func chat(size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default, zoom: CGFloat, family: String?) -> Font {
        Typefaces.font(family: family, size: ChatZoom.scaled(size, by: zoom), weight: weight, design: design)
    }

    /// A system face at a stock size, at the window's text size: exactly `size` at the
    /// stock step (see `ChatZoom.scaled`). The chrome's fonts, where the conversation's
    /// text styles would not keep its sizes: the sidebar's 13, the chips' 11.
    static func zoomed(size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default, zoom: CGFloat) -> Font {
        .system(size: ChatZoom.scaled(size, by: zoom), weight: weight, design: design)
    }

    /// A system text style at the window's text size: the style itself at the stock step,
    /// built the way the views asked for it (`.caption2.weight(.bold)`, `.system(.caption,
    /// design: .monospaced)`), so it reads exactly as it did; the style's own size on macOS,
    /// scaled, at any other step, in its own weight unless another is asked for.
    static func zoomed(_ style: Font.TextStyle, weight: Font.Weight? = nil, design: Font.Design? = nil, zoom: CGFloat) -> Font {
        guard zoom != 1 else {
            let font = Font.system(style, design: design ?? .default)
            return weight.map { font.weight($0) } ?? font
        }
        let stock = zoomedStock(style)
        return .system(size: ChatZoom.scaled(stock.size, by: zoom), weight: weight ?? stock.weight, design: design ?? .default)
    }

    /// The size and weight macOS sets a text style in, for `zoomed(_:weight:design:zoom:)`.
    /// Unlike `chatBaseSize`, body and headline stay at the system's 13.
    private static func zoomedStock(_ style: Font.TextStyle) -> (size: CGFloat, weight: Font.Weight) {
        switch style {
        case .largeTitle: (26, .regular)
        case .title: (22, .regular)
        case .title2: (17, .regular)
        case .title3: (15, .regular)
        case .headline: (13, .bold)
        case .body: (13, .regular)
        case .callout: (12, .regular)
        case .subheadline: (11, .regular)
        case .footnote, .caption: (10, .regular)
        case .caption2: (10, .medium)
        @unknown default: (13, .regular)
        }
    }
}

extension View {
    /// `.font(.system(size:weight:design:))` at the window's text size, read from the
    /// environment: the stock size at the stock step. `monospacedDigit` stands for the
    /// font's own `.monospacedDigit()`. A `Text` that needs its font as a value (one run of
    /// a concatenation) takes `Font.zoomed` with its view's `chatZoom` instead.
    func zoomedFont(size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default, monospacedDigit: Bool = false) -> some View {
        modifier(ZoomedFontModifier(face: .size(size, weight: weight, design: design), monospacedDigit: monospacedDigit))
    }

    /// `.font(style)` at the window's text size: the style itself at the stock step (see
    /// `Font.zoomed(_:weight:design:zoom:)`).
    func zoomedFont(_ style: Font.TextStyle, weight: Font.Weight? = nil, design: Font.Design? = nil, monospacedDigit: Bool = false) -> some View {
        modifier(ZoomedFontModifier(face: .style(style, weight: weight, design: design), monospacedDigit: monospacedDigit))
    }
}

private struct ZoomedFontModifier: ViewModifier {
    enum Face {
        case size(CGFloat, weight: Font.Weight, design: Font.Design)
        case style(Font.TextStyle, weight: Font.Weight?, design: Font.Design?)
    }

    let face: Face
    let monospacedDigit: Bool
    @Environment(\.chatZoom) private var zoom

    func body(content: Content) -> some View {
        content.font(font)
    }

    private var font: Font {
        let font: Font
        switch face {
        case .size(let size, let weight, let design):
            font = Font.zoomed(size: size, weight: weight, design: design, zoom: zoom)
        case .style(let style, let weight, let design):
            font = Font.zoomed(style, weight: weight, design: design, zoom: zoom)
        }
        return monospacedDigit ? font.monospacedDigit() : font
    }
}
