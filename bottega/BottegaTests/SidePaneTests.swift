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
        #expect(WindowMeasures.pageNarrowest == 640)
        #expect(WindowMeasures.room(for: .sidebar, window: 1280, other: nil) == 1280 - 20 - 8 - 640)
        #expect(WindowMeasures.room(for: .sidebar, window: 1280, other: 292) == 1280 - 20 - 8 - 640 - 292 - 10)
        #expect(WindowMeasures.room(for: .panel, window: 1280, other: nil) == 1280 - 20 - 640 - 10)
        #expect(WindowMeasures.room(for: .panel, window: 1280, other: 276) == 1280 - 20 - 640 - 10 - 276 - 8)
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
    }

    /// Before the window has been measured (0) nothing is squeezed: not what is drawn, and not a drag either.
    @Test func aWindowNotMeasuredYetLimitsNothing() {
        #expect(WindowMeasures.room(for: .sidebar, window: 0, other: 292) == .infinity)
        #expect(WindowMeasures.room(for: .panel, window: 0, other: nil) == .infinity)
        let unmeasured = WindowMeasures.drawn(sidebar: 380, panel: 440, window: 0)
        #expect(unmeasured.sidebar == 380)
        #expect(unmeasured.panel == 440)
        #expect(WindowMeasures.dragged(.sidebar, to: 380, window: 0, other: 440) == .width(380))
        #expect(WindowMeasures.dragged(.panel, to: 430, window: 0, other: 400) == .width(430))
        // Its own range and the way it closes are not the window's to lift.
        #expect(WindowMeasures.dragged(.panel, to: 900, window: 0, other: nil) == .width(440))
        #expect(WindowMeasures.dragged(.panel, to: 100, window: 0, other: nil) == .closes)
        // Measured, it limits again.
        #expect(WindowMeasures.room(for: .panel, window: 1, other: nil) < 0)
    }

    /// The window cannot be made so narrow that a pane would be pushed past its edge: with both shown it stops
    /// where each is at its narrowest and the page at its own.
    @Test func theWindowIsNoNarrowerThanWhatItShows() {
        #expect(WindowMeasures.windowNarrowest == 860)
        // The page alone needs less than the window's own narrowest.
        #expect(WindowMeasures.narrowestWindow(sidebar: false, panel: false) == 860)
        #expect(WindowMeasures.narrowestWindow(sidebar: true, panel: false) == 20 + 220 + 8 + 640)
        #expect(WindowMeasures.narrowestWindow(sidebar: false, panel: true) == 20 + 640 + 240 + 10)
        #expect(WindowMeasures.narrowestWindow(sidebar: true, panel: true) == 20 + 220 + 8 + 640 + 240 + 10)
        // At that width everything is at its narrowest and the page has exactly its own.
        let narrowest = WindowMeasures.narrowestWindow(sidebar: true, panel: true)
        let drawn = WindowMeasures.drawn(sidebar: 400, panel: 440, window: narrowest)
        #expect(drawn.sidebar == 220)
        #expect(drawn.panel == 240)
        #expect(narrowest - 20 - (220 + 8) - (240 + 10) == WindowMeasures.pageNarrowest)
        for (sidebar, panel) in [(true, false), (false, true), (true, true)] {
            let window = WindowMeasures.narrowestWindow(sidebar: sidebar, panel: panel)
            let drawn = WindowMeasures.drawn(sidebar: sidebar ? 400 : nil, panel: panel ? 440 : nil, window: window)
            let taken = (drawn.sidebar.map { $0 + 8 } ?? 0) + (drawn.panel.map { $0 + 10 } ?? 0)
            #expect(window - 20 - taken >= WindowMeasures.pageNarrowest, "\(sidebar) \(panel)")
        }
    }

    /// The panel gives way first, then the sidebar, and the page keeps its 640.
    @Test func aNarrowWindowTakesFromThePanelFirst() {
        // As asked while there is room for it: 276 + 8 and 292 + 10 beside a page of 640, in 20 of margins.
        let roomy = WindowMeasures.drawn(sidebar: 276, panel: 292, window: 1246)
        #expect(roomy.sidebar == 276)
        #expect(roomy.panel == 292)
        let some = WindowMeasures.drawn(sidebar: 276, panel: 292, window: 1220)
        #expect(some.sidebar == 276)
        #expect(some.panel == 266)
        let most = WindowMeasures.drawn(sidebar: 276, panel: 292, window: 1160)
        #expect(most.panel == 240)
        #expect(most.sidebar == 242)
        let all = WindowMeasures.drawn(sidebar: 400, panel: 440, window: 1138)
        #expect(all.sidebar == 220)
        #expect(all.panel == 240)
        for window in stride(from: 1138.0, through: 1900, by: 37) {
            let drawn = WindowMeasures.drawn(sidebar: 400, panel: 440, window: window)
            let page = window - 20 - (drawn.sidebar! + 8) - (drawn.panel! + 10)
            #expect(page >= 640, "\(window)")
        }
    }

    /// The sidebar may be dragged until the panel is at its narrowest; the panel only into what the sidebar leaves.
    @Test func aDragKnowsWhichPaneGivesWay() {
        #expect(WindowMeasures.dragged(.sidebar, to: 390, window: 1220, other: 292) == .width(1220 - 20 - 8 - 640 - 240 - 10))
        #expect(WindowMeasures.dragged(.sidebar, to: 390, window: 1220, other: nil) == .width(390))
        #expect(WindowMeasures.dragged(.panel, to: 400, window: 1220, other: 276) == .width(266))
        #expect(WindowMeasures.dragged(.panel, to: 400, window: 1220, other: nil) == .width(400))
        #expect(WindowMeasures.dragged(.panel, to: 100, window: 1220, other: 276) == .closes)
    }

    /// A drag that only changes the width leaves the pane shown at that width; one that closes it leaves the width
    /// the drag began from, so that showing it again brings back what was there, not its narrowest.
    @Test func aPanePutAwayKeepsTheWidthTheDragBeganFrom() {
        let wider = WindowMeasures.settled(.sidebar, to: 340, from: 300, window: 1280, other: nil)
        #expect(wider.width == 340)
        #expect(wider.isShown)
        let squeezed = WindowMeasures.settled(.panel, to: 400, from: 300, window: 1220, other: 276)
        #expect(squeezed.width == 266)
        #expect(squeezed.isShown)
        let closed = WindowMeasures.settled(.sidebar, to: 100, from: 300, window: 1280, other: nil)
        #expect(closed.width == 300)
        #expect(!closed.isShown)
        let panel = WindowMeasures.settled(.panel, to: 120, from: 410, window: 1280, other: 276)
        #expect(panel.width == 410)
        #expect(!panel.isShown)
        // Whatever it began from, what is kept is a width it can have.
        #expect(WindowMeasures.settled(.sidebar, to: 0, from: 5000, window: 1280, other: nil).width == 400)
        #expect(WindowMeasures.settled(.sidebar, to: 0, from: .nan, window: 1280, other: nil).width == 220)
    }

    /// What a pane was dragged to is what it is then drawn at: it does not jump when the finger lets go.
    @Test func whatWasDraggedIsWhatIsDrawn() {
        for window in [1138.0, 1220, 1320, 1500, 1920] {
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
