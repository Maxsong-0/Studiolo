// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import BottegaLink

struct LineSplitterTests {
    @Test func linesComeOutHoweverTheBytesArrive() throws {
        let whole = Data("one\n\n三\nrest".utf8)
        for size in 1...whole.count {
            var splitter = LineSplitter()
            var lines: [String] = []
            var start = 0
            while start < whole.count {
                let end = min(start + size, whole.count)
                lines += try splitter.take(whole[start..<end]).map { String(decoding: $0, as: UTF8.self) }
                start = end
            }
            #expect(lines == ["one", "", "三"])
            #expect(try splitter.take(Data("\n".utf8)) == [Data("rest".utf8)])
        }
    }
}
