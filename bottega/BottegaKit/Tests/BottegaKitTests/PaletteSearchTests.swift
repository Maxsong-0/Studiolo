// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing
@testable import BottegaKit

/// Which ⌘K rows come first for what is typed.
struct PaletteSearchTests {
    private struct Row: Equatable {
        let title: String
        var subtitle = ""
    }

    private func titles(_ rows: [Row], _ query: String) -> [String] {
        PaletteSearch.ranked(rows, for: query, title: \.title, subtitle: \.subtitle).map(\.title)
    }

    @Test func theStartBeatsTheStartOfAWordBeatsTheMiddle() {
        let rows = [Row(title: "Pre"), Row(title: "Go to the rest"), Row(title: "x"), Row(title: "Reload")]
        #expect(titles(rows, "re") == ["Reload", "Go to the rest", "Pre"])
    }

    @Test func theTitleBeatsTheSubtitle() {
        let rows = [Row(title: "Show the cards", subtitle: "Board"), Row(title: "Boarding"), Row(title: "On board")]
        #expect(titles(rows, "board") == ["Boarding", "On board", "Show the cards"])
        // However well the subtitle answers, a title that answers at all comes first.
        let middle = [Row(title: "x", subtitle: "Stop"), Row(title: "nonstop")]
        #expect(titles(middle, "stop") == ["nonstop", "x"])
    }

    @Test func equallyGoodRowsKeepTheirOrder() {
        let rows = [Row(title: "card-2"), Row(title: "card-10"), Row(title: "card-1")]
        #expect(titles(rows, "card") == ["card-2", "card-10", "card-1"])
        #expect(titles(rows.reversed(), "card") == ["card-1", "card-10", "card-2"])
    }

    @Test func caseAndSpaceAroundDoNotCount() {
        let rows = [Row(title: "card-12"), Row(title: "Board")]
        #expect(titles(rows, "  CARD ") == ["card-12"])
        // Nothing typed: everything, as it came.
        #expect(titles(rows, "") == ["card-12", "Board"])
        #expect(titles(rows, " \t") == ["card-12", "Board"])
    }

    @Test func whatIsTypedIsLookedForAsOnePiece() {
        let rows = [Row(title: "Stop this turn"), Row(title: "turn stop")]
        #expect(titles(rows, "stop t") == ["Stop this turn"])
        #expect(titles(rows, "card-12 x") == [])
    }

    /// A later start of a word counts, not only the first place it is found.
    @Test func theBestPlaceCounts() {
        let rows = [Row(title: "torrent"), Row(title: "parent rent")]
        #expect(titles(rows, "rent") == ["parent rent", "torrent"])
        // After a digit is the middle of a word; after a dash or a dot is the start of one.
        #expect(PaletteSearch.place(of: Array("2"), in: "card-12") == .inside)
        #expect(PaletteSearch.place(of: Array("12"), in: "card-12") == .startOfWord)
        #expect(PaletteSearch.place(of: Array("3"), in: "v1.3") == .startOfWord)
        #expect(PaletteSearch.place(of: Array("longer"), in: "long") == nil)
    }

    @Test func textWithoutSpacesBetweenWords() {
        let rows = [Row(title: "清空缓存"), Row(title: "缓存清理")]
        #expect(titles(rows, "缓存") == ["缓存清理", "清空缓存"])
        #expect(titles(rows, "清空") == ["清空缓存"])
    }
}
