// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// How ⌘K orders its rows for what is typed (vault Bottega-6, U4). What a row is and does is the app's; which come
/// first is decided here, where it can be tested.
///
/// What is typed is looked for as it is, as one piece: what matches is always what was typed. A row answers it in its
/// title before its subtitle; at the start of the text before at the start of a word, before anywhere else. Rows that
/// answer equally well keep the order they came in, so the app decides between them.
public enum PaletteSearch {
    /// Where in a text what is typed was found.
    enum Place: Int, Comparable {
        case start, startOfWord, inside

        static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }
    }

    /// The rows that answer `query`, best first. Nothing typed, or only space: all of them, as they came.
    public static func ranked<Row>(
        _ rows: [Row], for query: String, title: (Row) -> String, subtitle: (Row) -> String = { _ in "" }
    ) -> [Row] {
        let needle = Array(IntentDraft.trimmed(query).lowercased())
        guard !needle.isEmpty else { return rows }
        return rows.enumerated()
            .compactMap { index, row in rank(needle, title: title(row), subtitle: subtitle(row)).map { (row, $0, index) } }
            .sorted { ($0.1, $0.2) < ($1.1, $1.2) }
            .map(\.0)
    }

    /// Lower is better; nil when the row does not answer.
    static func rank(_ needle: [Character], title: String, subtitle: String) -> Int? {
        if let place = place(of: needle, in: title) { return place.rawValue }
        if let place = place(of: needle, in: subtitle) { return 3 + place.rawValue }
        return nil
    }

    /// The best place it is found at: a later start of a word beats an earlier middle.
    static func place(of needle: [Character], in text: String) -> Place? {
        let text = Array(text.lowercased())
        guard !needle.isEmpty, needle.count <= text.count else { return nil }
        var best: Place?
        for start in 0...(text.count - needle.count) where text[start..<(start + needle.count)].elementsEqual(needle) {
            if start == 0 { return .start }
            let before = text[start - 1]
            if !before.isLetter && !before.isNumber { return .startOfWord }
            best = .inside
        }
        return best
    }
}
