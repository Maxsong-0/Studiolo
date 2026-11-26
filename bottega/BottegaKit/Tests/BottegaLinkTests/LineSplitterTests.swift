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

    /// As much as the limit may wait for its newline; one byte more is not from our peer.
    @Test func aLineMayWaitUpToTheLimitAndNoFurther() throws {
        func takes(_ chunks: String..., limit: Int) throws -> [String] {
            var splitter = LineSplitter(limit: limit)
            return try chunks.flatMap { try splitter.take(Data($0.utf8)) }.map { String(decoding: $0, as: UTF8.self) }
        }
        #expect(try takes("12345678", limit: 8).isEmpty)
        #expect(try takes("12345678", "\n", limit: 8) == ["12345678"])
        #expect(throws: LineSplitter.LineTooLong.self) { try takes("123456789", limit: 8) }
        #expect(throws: LineSplitter.LineTooLong.self) { try takes("12345678", "9", limit: 8) }
        // Lines that are done do not count towards what waits.
        #expect(try takes("ab\ncd\nef\n1234567", limit: 8) == ["ab", "cd", "ef"])
        #expect(LineSplitter.maxLineBytes == 64 * 1024 * 1024)
    }

    @Test func manyLinesInOneChunkAndOneLineInManyChunks() throws {
        var splitter = LineSplitter()
        #expect(try splitter.take(Data(String(repeating: "ab\n", count: 20_000).utf8)).count == 20_000)
        for _ in 0..<1000 { #expect(try splitter.take(Data("xyz".utf8)).isEmpty) }
        #expect(try splitter.take(Data("\nq".utf8)) == [Data(String(repeating: "xyz", count: 1000).utf8)])
        #expect(try splitter.take(Data("\n".utf8)) == [Data("q".utf8)])
    }
}
