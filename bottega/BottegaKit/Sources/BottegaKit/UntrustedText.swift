// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// Text an agent or a test command wrote. It is data: shown as plain text, never as markup, and without the
/// characters that show nothing and change what the rest looks like (terminal escapes, bidi overrides, zero-width
/// ones). Views print it with `Text(verbatim:)`.
public enum UntrustedText {
    /// For reading: line breaks and tabs stay, hidden characters go, terminal escape sequences go whole.
    public static func clean(_ text: String) -> String {
        var kept = String.UnicodeScalarView()
        let scalars = Array(text.unicodeScalars)
        var index = 0
        while index < scalars.count {
            let scalar = scalars[index]
            if scalar == escape {
                index = end(ofEscapeAt: index, in: scalars)
                continue
            }
            if isLineBreak(scalar) {
                // "\r\n" is one break; a lone "\r" is one too, so nothing can overprint the line before it.
                if scalar == "\r", index + 1 < scalars.count, scalars[index + 1] == "\n" { index += 1 }
                kept.append("\n")
            } else if scalar == "\t" || isShown(at: index, in: scalars) {
                kept.append(scalar)
            }
            index += 1
        }
        return String(kept)
    }

    /// One line of it, for titles and rows.
    public static func line(_ text: String, limit: Int = 240) -> String {
        let words = clean(text).split(whereSeparator: \.isWhitespace)
        let joined = words.joined(separator: " ")
        return joined.count > limit ? joined.prefix(limit) + "…" : joined
    }

    /// For a text whose exact bytes are about to be signed for: nothing is dropped. A hidden character is written
    /// out as its code point, so what is read is what is signed.
    public static func revealing(_ text: String) -> (text: String, hidden: Int) {
        var shown = String.UnicodeScalarView()
        var hidden = 0
        let scalars = Array(text.unicodeScalars)
        for index in scalars.indices {
            let scalar = scalars[index]
            if scalar == "\n" || scalar == "\t" || (!isLineBreak(scalar) && isShown(at: index, in: scalars)) {
                shown.append(scalar)
            } else {
                hidden += 1
                let code = String(scalar.value, radix: 16, uppercase: true)
                shown.append(contentsOf: "⟨U+\(String(repeating: "0", count: max(0, 4 - code.count)))\(code)⟩".unicodeScalars)
            }
        }
        return (String(shown), hidden)
    }

    private static let escape: Unicode.Scalar = "\u{1B}"

    private static func isLineBreak(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar {
        case "\n", "\r", "\u{85}", "\u{2028}", "\u{2029}": true
        default: false
        }
    }

    /// Whether the reader can see it, by itself or in how it changes the picture it belongs to.
    private static func isShown(at index: Int, in scalars: [Unicode.Scalar]) -> Bool {
        let scalar = scalars[index]
        switch scalar.properties.generalCategory {
        case .control, .format: return scalar == joiner && joinsPictures(at: index, in: scalars)
        default: break
        }
        if scalar.properties.isVariationSelector { return drawsEmoji(at: index, in: scalars) }
        // Ignorable by definition though classed as letters or marks: Hangul fillers and the like.
        return !scalar.properties.isDefaultIgnorableCodePoint
    }

    private static let joiner: Unicode.Scalar = "\u{200D}"

    /// A zero-width joiner between two pictures makes them one; anywhere else it is just hidden.
    private static func joinsPictures(at index: Int, in scalars: [Unicode.Scalar]) -> Bool {
        guard index > 0, index + 1 < scalars.count, isPicture(scalars[index + 1]) else { return false }
        let before = scalars[index - 1]
        if before.properties.isVariationSelector {
            return index > 1 && isPicture(scalars[index - 2]) && drawsEmoji(at: index - 1, in: scalars)
        }
        return isPicture(before)
    }

    /// Variation selectors show nothing themselves, and runs of them can carry any bytes. The only ones let through
    /// pick how an emoji right before them is drawn: one per emoji, and for a digit only as part of a keycap.
    private static func drawsEmoji(at index: Int, in scalars: [Unicode.Scalar]) -> Bool {
        guard scalars[index] == "\u{FE0E}" || scalars[index] == "\u{FE0F}", index > 0 else { return false }
        let base = scalars[index - 1]
        guard base.properties.isEmoji else { return false }
        return !base.isASCII || (index + 1 < scalars.count && scalars[index + 1] == "\u{20E3}")
    }

    /// Unicode also counts digits, `#`, `*`, © and a few letterlike signs as emoji; nothing joins to those.
    private static func isPicture(_ scalar: Unicode.Scalar) -> Bool {
        scalar.properties.isEmoji && scalar.value >= 0x2190
    }

    /// The index after the escape sequence starting at `start`. An unfinished one costs only its ESC.
    private static func end(ofEscapeAt start: Int, in scalars: [Unicode.Scalar]) -> Int {
        guard start + 1 < scalars.count else { return start + 1 }
        switch scalars[start + 1] {
        case "[":  // CSI: parameters, then one final byte.
            for index in (start + 2)..<min(start + 66, max(start + 2, scalars.count)) {
                switch scalars[index].value {
                case 0x40...0x7E: return index + 1
                case 0x20...0x3F: continue
                default: return start + 1
                }
            }
            return start + 1
        case "]":  // OSC: up to BEL, or ESC and a backslash.
            for index in (start + 2)..<min(start + 4098, max(start + 2, scalars.count)) {
                if scalars[index] == "\u{07}" { return index + 1 }
                if scalars[index] == escape, index + 1 < scalars.count, scalars[index + 1] == "\\" { return index + 2 }
                if scalars[index] == "\n" { return start + 1 }
            }
            return start + 1
        default:
            return start + 1
        }
    }
}
