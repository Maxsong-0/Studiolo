// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The two panes either side of the page: the sidebar on the left, the panel on the right. Each is dragged wider or
/// narrower by its inner edge and can be put away (T-245). The widths are kept on this Mac only.
enum SidePane: String, CaseIterable {
    case sidebar, panel

    /// How narrow and how wide it goes.
    var widths: ClosedRange<Double> {
        switch self {
        case .sidebar: 220...400
        case .panel: 240...440
        }
    }

    /// As wide as the canvas draws it.
    var startingWidth: Double {
        switch self {
        case .sidebar: 276
        case .panel: 292
        }
    }

    var storageKey: String { "vetro.width." + rawValue }

    /// What dragging its edge comes to.
    enum Drag: Equatable {
        case width(Double)
        /// Dragged well past its narrowest: put away, as the menu command does.
        case closes
    }

    /// Dragged this far past its narrowest, a pane closes rather than staying at its narrowest.
    static let closesPast = 60.0

    /// `room` is how wide it may be with the page between the panes still at its narrowest. A pane is never
    /// squeezed below its own narrowest to make room: the window does not get that small
    /// (`WindowMeasures.narrowestWindow`).
    func dragged(to width: Double, room: Double) -> Drag {
        guard width.isFinite else { return .width(startingWidth) }
        if width < widths.lowerBound - Self.closesPast { return .closes }
        return .width(fitted(width, room: room))
    }

    func fitted(_ width: Double, room: Double = .infinity) -> Double {
        let widest = max(widths.lowerBound, min(widths.upperBound, room))
        return min(widest, max(widths.lowerBound, width))
    }

    /// What was kept, or the starting width when nothing usable was.
    func width(in defaults: UserDefaults) -> Double {
        guard defaults.object(forKey: storageKey) != nil else { return startingWidth }
        let kept = defaults.double(forKey: storageKey)
        return kept.isFinite && kept > 0 ? fitted(kept) : startingWidth
    }

    func keep(_ width: Double, in defaults: UserDefaults) { defaults.set(fitted(width), forKey: storageKey) }
}

/// The window's fixed measures around the panes, and from them how much room a pane has.
enum WindowMeasures {
    /// Between the window's edge and the panes.
    static let margin = 10.0
    /// Between the sidebar and the main pane.
    static let gap = 8.0
    /// Between the panel and the main pane's edge.
    static let panelInset = 10.0
    /// The page between the panes is never narrower than this: what its widest rows, the list of cards, need
    /// to still show a title.
    static let pageNarrowest = 640.0
    /// The strip along a pane's inner edge that drags it. The sidebar's is the gap itself.
    static let handle = gap
    /// How far the window's own close, minimise and zoom buttons reach into the page's header once the sidebar,
    /// which they otherwise sit over, is put away.
    static let windowButtons = 62.0
    /// The window is never narrower than this, whatever is shown in it, nor shorter than that.
    static let windowNarrowest = 860.0
    static let windowShortest = 520.0

    /// How narrow the window may get with these panes shown: room for each at its narrowest and the page at its
    /// own, so that nothing is ever pushed past the window's edge.
    static func narrowestWindow(sidebar: Bool, panel: Bool) -> Double {
        let panes = (sidebar ? SidePane.sidebar.widths.lowerBound + gap : 0)
            + (panel ? SidePane.panel.widths.lowerBound + panelInset : 0)
        return max(windowNarrowest, 2 * margin + pageNarrowest + panes)
    }

    /// How wide `pane` may be in a window `window` wide, the other pane being `other` wide (nil when put away).
    /// A window not measured yet (`window` 0) limits nothing: a pane is not squeezed for a width nobody knows.
    static func room(for pane: SidePane, window: Double, other: Double?) -> Double {
        guard window > 0 else { return .infinity }
        let sidebar = pane == .sidebar ? 0 : (other.map { $0 + gap } ?? 0)
        let panel = pane == .panel ? panelInset : (other.map { $0 + panelInset } ?? 0)
        let own = pane == .sidebar ? gap : 0
        return window - 2 * margin - pageNarrowest - sidebar - panel - own
    }

    /// The widths the panes are drawn at: what was asked for (nil when put away), less what has to give for the
    /// page to keep its narrowest. The panel gives first, then the sidebar, neither below its own narrowest.
    static func drawn(sidebar: Double?, panel: Double?, window: Double) -> (sidebar: Double?, panel: Double?) {
        let panelDrawn = panel.map { SidePane.panel.fitted($0, room: room(for: .panel, window: window, other: sidebar)) }
        let sidebarDrawn = sidebar.map {
            SidePane.sidebar.fitted($0, room: room(for: .sidebar, window: window, other: panelDrawn))
        }
        return (sidebarDrawn, panelDrawn)
    }

    /// What dragging `pane`'s edge to `width` comes to, the other pane asking for `other` (nil when put away).
    /// The same order as `drawn`: the sidebar may grow until the panel is at its narrowest; the panel only into
    /// what the sidebar leaves.
    static func dragged(_ pane: SidePane, to width: Double, window: Double, other: Double?) -> SidePane.Drag {
        let held = pane == .sidebar ? other.map { _ in SidePane.panel.widths.lowerBound } : other
        return pane.dragged(to: width, room: room(for: pane, window: window, other: held))
    }

    /// What a drag that began with `pane` drawn `start` wide leaves it at: how wide, and whether it is still
    /// shown. Put away, it keeps the width the drag began from, not the narrowest it passed on the way there:
    /// showing it again brings back what was there.
    static func settled(
        _ pane: SidePane, to width: Double, from start: Double, window: Double, other: Double?
    ) -> (width: Double, isShown: Bool) {
        switch dragged(pane, to: width, window: window, other: other) {
        case .width(let width): (width, true)
        case .closes: (pane.fitted(start), false)
        }
    }
}
