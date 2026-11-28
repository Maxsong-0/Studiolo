// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing

/// The sidebar and the panel: how wide they go, what is kept, when a drag puts one away, and which gives way in a
/// window too narrow for both (T-245).
struct SidePaneTests {
    private func freshDefaults() -> UserDefaults {
        let name = "SidePaneTests-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @Test func startsAsWideAsTheCanvasDrawsThem() {
        let defaults = freshDefaults()
        #expect(SidePane.sidebar.width(in: defaults) == 276)
        #expect(SidePane.panel.width(in: defaults) == 292)
        #expect(SidePane.sidebar.widths == 220...400)
        #expect(SidePane.panel.widths == 240...440)
        for pane in SidePane.allCases { #expect(pane.widths.contains(pane.startingWidth)) }
    }

    @Test func keepsWhatWasDragged() {
        let defaults = freshDefaults()
        SidePane.sidebar.keep(350, in: defaults)
        SidePane.panel.keep(410, in: defaults)
        #expect(SidePane.sidebar.width(in: defaults) == 350)
        #expect(SidePane.panel.width(in: defaults) == 410)
        // Each under its own name.
        #expect(defaults.double(forKey: "vetro.width.sidebar") == 350)
        #expect(defaults.double(forKey: "vetro.width.panel") == 410)
        // Kept as it will be drawn, not as far as the pointer went.
        SidePane.sidebar.keep(1000, in: defaults)
        #expect(defaults.double(forKey: "vetro.width.sidebar") == 400)
    }

    @Test func readsBackOnlyWhatIsUsable() {
        let defaults = freshDefaults()
        defaults.set(50, forKey: "vetro.width.sidebar")
        #expect(SidePane.sidebar.width(in: defaults) == 220)
        defaults.set(9999, forKey: "vetro.width.sidebar")
        #expect(SidePane.sidebar.width(in: defaults) == 400)
        let unusables: [Any] = ["wide", 0, -30, Double.nan, Double.infinity]
        for unusable in unusables {
            defaults.set(unusable, forKey: "vetro.width.panel")
            #expect(SidePane.panel.width(in: defaults) == 292, "\(unusable)")
        }
    }

    @Test func aDragStaysBetweenNarrowestAndWidest() {
        let sidebar = SidePane.sidebar
        #expect(sidebar.dragged(to: 300, room: 1000) == .width(300))
        #expect(sidebar.dragged(to: 500, room: 1000) == .width(400))
        #expect(sidebar.dragged(to: 200, room: 1000) == .width(220))
        // No wider than leaves the page its narrowest.
        #expect(sidebar.dragged(to: 380, room: 300) == .width(300))
        // And never squeezed below its own narrowest to make room.
        #expect(sidebar.dragged(to: 380, room: 100) == .width(220))
        #expect(sidebar.dragged(to: .nan, room: 1000) == .width(276))
        #expect(SidePane.panel.dragged(to: 600, room: 1000) == .width(440))
        #expect(SidePane.panel.dragged(to: 100, room: 1000) == .closes)
    }

    /// 60 points past its narrowest, not sooner: a pane does not vanish for a drag that only overshot.
    @Test func draggedWellPastItsNarrowestItCloses() {
        #expect(SidePane.closesPast == 60)
        #expect(SidePane.sidebar.dragged(to: 160, room: 1000) == .width(220))
        #expect(SidePane.sidebar.dragged(to: 159.5, room: 1000) == .closes)
        #expect(SidePane.panel.dragged(to: 180, room: 1000) == .width(240))
        #expect(SidePane.panel.dragged(to: 179, room: 1000) == .closes)
        #expect(SidePane.sidebar.dragged(to: -400, room: 1000) == .closes)
    }

    /// What is left for a pane once the margins, the gap, the other pane and the page's narrowest are taken out.
    @Test func roomIsWhatThePageDoesNotNeed() {
        #expect(WindowMeasures.pageNarrowest == 420)
        #expect(WindowMeasures.room(for: .sidebar, window: 1280, other: nil) == 1280 - 20 - 8 - 420)
        #expect(WindowMeasures.room(for: .sidebar, window: 1280, other: 292) == 1280 - 20 - 8 - 420 - 292 - 10)
        #expect(WindowMeasures.room(for: .panel, window: 1280, other: nil) == 1280 - 20 - 420 - 10)
        #expect(WindowMeasures.room(for: .panel, window: 1280, other: 276) == 1280 - 20 - 420 - 10 - 276 - 8)
    }

    @Test func aWideWindowDrawsThemAsAsked() {
        let drawn = WindowMeasures.drawn(sidebar: 276, panel: 292, window: 1280)
        #expect(drawn.sidebar == 276)
        #expect(drawn.panel == 292)
        // Put away stays put away.
        let alone = WindowMeasures.drawn(sidebar: nil, panel: 292, window: 1280)
        #expect(alone.sidebar == nil)
        #expect(alone.panel == 292)
        #expect(WindowMeasures.drawn(sidebar: 276, panel: nil, window: 1280).panel == nil)
        // Not measured yet: as asked, rather than both at their narrowest for a frame.
        let unmeasured = WindowMeasures.drawn(sidebar: 380, panel: 440, window: 0)
        #expect(unmeasured.sidebar == 380)
        #expect(unmeasured.panel == 440)
    }

    /// The panel gives way first, then the sidebar, and the page keeps its 420.
    @Test func aNarrowWindowTakesFromThePanelFirst() {
        let some = WindowMeasures.drawn(sidebar: 276, panel: 292, window: 1000)
        #expect(some.sidebar == 276)
        #expect(some.panel == 266)
        let most = WindowMeasures.drawn(sidebar: 276, panel: 292, window: 940)
        #expect(most.panel == 240)
        #expect(most.sidebar == 242)
        let all = WindowMeasures.drawn(sidebar: 400, panel: 440, window: 918)
        #expect(all.sidebar == 220)
        #expect(all.panel == 240)
        for window in stride(from: 918.0, through: 1600, by: 37) {
            let drawn = WindowMeasures.drawn(sidebar: 400, panel: 440, window: window)
            let page = window - 20 - (drawn.sidebar! + 8) - (drawn.panel! + 10)
            #expect(page >= 420, "\(window)")
        }
    }

    /// The sidebar may be dragged until the panel is at its narrowest; the panel only into what the sidebar leaves.
    @Test func aDragKnowsWhichPaneGivesWay() {
        #expect(WindowMeasures.dragged(.sidebar, to: 390, window: 1000, other: 292) == .width(1000 - 20 - 8 - 420 - 240 - 10))
        #expect(WindowMeasures.dragged(.sidebar, to: 390, window: 1000, other: nil) == .width(390))
        #expect(WindowMeasures.dragged(.panel, to: 400, window: 1000, other: 276) == .width(266))
        #expect(WindowMeasures.dragged(.panel, to: 400, window: 1000, other: nil) == .width(400))
        #expect(WindowMeasures.dragged(.panel, to: 100, window: 1000, other: 276) == .closes)
    }

    /// What a pane was dragged to is what it is then drawn at: it does not jump when the finger lets go.
    @Test func whatWasDraggedIsWhatIsDrawn() {
        for window in [918.0, 1000, 1100, 1280, 1700] {
            for asked in stride(from: 170.0, through: 460, by: 29) {
                if case .width(let sidebar) = WindowMeasures.dragged(.sidebar, to: asked, window: window, other: 330) {
                    #expect(WindowMeasures.drawn(sidebar: sidebar, panel: 330, window: window).sidebar == sidebar, "\(window) \(asked)")
                }
                if case .width(let panel) = WindowMeasures.dragged(.panel, to: asked, window: window, other: 250) {
                    #expect(WindowMeasures.drawn(sidebar: 250, panel: panel, window: window).panel == panel, "\(window) \(asked)")
                }
            }
        }
    }
}
