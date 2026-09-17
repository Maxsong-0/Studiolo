import Foundation
import Testing

@Test func linesPreserveFragmentedUnicodeAndFinalRemainder() {
    var framer = StdioFramer(framing: .lines)
    let source = Data("α\r\n👩🏽‍💻\nlast".utf8)
    var lines: [Data] = []
    for byte in source { lines += framer.append(Data([byte])) }
    #expect(lines == [Data("α\r".utf8), Data("👩🏽‍💻".utf8)])
    #expect(framer.finish() == Data("last".utf8))
    #expect(framer.finish() == nil)
}

@Test func contentLengthHandlesEverySplit() {
    let body = Data("{\"text\":\"α👩🏽‍💻\"}".utf8)
    let frame = Data("Content-Length: \(body.count)\r\nContent-Type: application/json\r\n\r\n".utf8) + body
    for split in 0...frame.count {
        var framer = StdioFramer(framing: .contentLength)
        let first = framer.append(Data(frame.prefix(split)))
        let second = framer.append(Data(frame.dropFirst(split)))
        #expect(first + second == [body])
        #expect(framer.finish() == nil)
    }
}

@Test func framesHandleBatchesInvalidHeadersAndTruncation() {
    var framer = StdioFramer(framing: .contentLength)
    let invalid = "noise\r\n\r\nContent-Length: -1\r\n\r\nContent-Length: 999999999999999999999\r\n\r\n"
    let valid = "content-length: 2\r\n\r\n{}Content-Length: 0\r\n\r\nContent-Length: 5\r\n\r\n12"
    #expect(framer.append(Data((invalid + valid).utf8)) == [Data("{}".utf8), Data()])
    #expect(framer.finish() == nil)
    #expect(framer.append(Data("Content-Length: 2\r\n\r\n[]".utf8)) == [Data("[]".utf8)])
}

@main struct FramingRegressionTests {
    static func main() async {
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}
