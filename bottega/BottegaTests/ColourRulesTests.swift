// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing

/// What a colour means in the Workshop window (ADR-047): red, waiting for the user; amber, an AI at work; the accent
/// colour, a button to press. A view that writes `.red` or `.orange` or the accent colour itself has, three times
/// now, used one for something else: a shut quota gate in red (T-243), a line that did not open in red, and the
/// accent colour as the ring round whatever is open (T-254). So the views do not write them. Red is `.waiting`,
/// orange is `.trouble`, the ring is `vetroRing`, and all three are in `Shared/Look.swift`.
struct ColourRulesTests {
    /// The system colours a view may not name, as they are written where a colour goes.
    static let banned = ["red", "orange", "blue", "accentColor"]

    /// Each place in `source` where one of them is named: `.red`, `Color.orange`, `Color.accentColor.opacity(…)`.
    /// A component of a colour (`Color(red: 1, …)`), a longer name (`.redacted`), and a value's own field that
    /// happens to be called the same (`pixel.red`, `$0.blue`) are not one.
    static func offences(in source: String) -> [String] {
        let names = banned.joined(separator: "|")
        // The dot either starts the expression (`.red`: nothing before it that a value could end with) or follows
        // a colour type (`Color.red`, `NSColor.red`); then the name, and nothing that would make it a longer one.
        let pattern = #"(?:(?<![A-Za-z0-9_)\]}?!])|Color)(\.(?:"# + names + #"))(?![A-Za-z0-9_])"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return ["the pattern does not compile"] }
        var found: [String] = []
        for (number, line) in source.components(separatedBy: "\n").enumerated() {
            let range = NSRange(line.startIndex..., in: line)
            for match in regex.matches(in: line, range: range) {
                if let where_ = Range(match.range(at: 1), in: line) { found.append("\(number + 1): \(line[where_])") }
            }
        }
        return found
    }

    @Test func theScanFindsEveryWayOfNamingThem() {
        #expect(Self.offences(in: "Image(systemName: name).foregroundStyle(.red)") == ["1: .red"])
        #expect(Self.offences(in: "x\n    .foregroundStyle(gate.level == \"closed\" ? .red : .orange)") == ["2: .red", "2: .orange"])
        #expect(Self.offences(in: "Circle().fill(Color.red.opacity(0.2))") == ["1: .red"])
        #expect(Self.offences(in: "shape.strokeBorder(Color.accentColor, lineWidth: 1.5)") == ["1: .accentColor"])
        #expect(Self.offences(in: "AnyShapeStyle(.orange)") == ["1: .orange"])
        #expect(Self.offences(in: ".tint(.blue)") == ["1: .blue"])
        #expect(Self.offences(in: "Banner(symbol: s, tint: .orange, text: t)") == ["1: .orange"])
        #expect(Self.offences(in: "Color(nsColor: NSColor.blue)") == ["1: .blue"])
        // Alone on its line, as a chain is broken.
        #expect(Self.offences(in: "Circle()\n    .fill(\n        .red\n    )") == ["3: .red"])
    }

    @Test func theScanLeavesTheRestAlone() {
        // The two names views are to use, and the parts of a colour.
        #expect(Self.offences(in: ".foregroundStyle(.waiting)").isEmpty)
        #expect(Self.offences(in: ".foregroundStyle(.trouble)").isEmpty)
        #expect(Self.offences(in: "Color(red: 1, green: 194 / 255, blue: 71 / 255)").isEmpty)
        #expect(Self.offences(in: "Color(.sRGB, red: 0.2, green: 0.2, blue : 0.2)").isEmpty)
        // Longer names that only begin the same way.
        #expect(Self.offences(in: "text.redacted(reason: .placeholder)").isEmpty)
        #expect(Self.offences(in: "Text(name)\n    .redacted(reason: .placeholder)\n    .blueprintStyle(.orangeish)").isEmpty)
        #expect(Self.offences(in: "view.blueprint.orangeTree").isEmpty)
        // A value's own field of the same name: the parts of a colour that was read off the screen, say.
        #expect(Self.offences(in: "let level = under.red + pixels.map { $0.blue }.max()!").isEmpty)
        #expect(Self.offences(in: "sample?.red ?? rgb(at: point).blue + grid[0].orange + last!.red").isEmpty)
        // Words, in a comment or a string.
        #expect(Self.offences(in: "/// Red means one thing in this app: waiting for the user. Not orange, not blue.").isEmpty)
        // Glass is tinted through `Look`, and that is not the accent colour.
        #expect(Self.offences(in: ".glassBehind(.clear.tint(look.glassTint), in: shape)").isEmpty)
        #expect(Self.offences(in: "").isEmpty)
    }

    /// The Workshop's own views and what they share with the tests. The app Bottega grew out of (`App/Views`,
    /// opened with `-classic`) is not held to this.
    @Test func noViewNamesASystemColour() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        var found: [String] = []
        for (folder, atLeast) in [("Bottega/App/Workshop", 30), ("Shared", 5)] {
            var seen = 0
            let files = FileManager.default.enumerator(at: root.appendingPathComponent(folder), includingPropertiesForKeys: nil)
            while let file = files?.nextObject() as? URL {
                guard file.pathExtension == "swift" else { continue }
                seen += 1
                let source = try String(contentsOf: file, encoding: .utf8)
                found += Self.offences(in: source).map { "\(file.lastPathComponent):\($0)" }
            }
            // It did read them: a folder that moved would otherwise pass by finding nothing.
            #expect(seen >= atLeast, "only \(seen) files were read in \(folder)")
        }
        #expect(found.isEmpty, "use .waiting (red: waits on the user) or .trouble (orange) from Shared/Look.swift, or vetroRing for what is open: \(found.joined(separator: ", "))")
    }
}
