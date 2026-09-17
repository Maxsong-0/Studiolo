import Foundation

struct StdioFramer {
    enum Framing: Sendable {
        case lines
        case contentLength
    }

    private let framing: Framing
    private var buffer = Data()
    private var scanned = 0
    private var bodyLength: Int?
    private static let separator = Data("\r\n\r\n".utf8)

    init(framing: Framing) {
        self.framing = framing
    }

    mutating func append(_ data: Data) -> [Data] {
        buffer.append(data)
        var messages: [Data] = []
        var start = buffer.startIndex
        var search = start + scanned
        while true {
            switch framing {
            case .lines:
                guard let newline = buffer[search...].firstIndex(of: 0x0A) else {
                    search = buffer.endIndex
                    break
                }
                messages.append(Data(buffer[start..<newline]))
                start = newline + 1
                search = start
                continue
            case .contentLength:
                if bodyLength == nil {
                    guard let headerEnd = buffer[search...].range(of: Self.separator) else {
                        search = max(start, buffer.endIndex - 3)
                        break
                    }
                    let header = String(decoding: buffer[start..<headerEnd.lowerBound], as: UTF8.self)
                    start = headerEnd.upperBound
                    search = start
                    guard let length = Self.contentLength(in: header) else { continue }
                    bodyLength = length
                }
                if let length = bodyLength, buffer.endIndex - start >= length {
                    let end = start + length
                    messages.append(Data(buffer[start..<end]))
                    bodyLength = nil
                    start = end
                    search = start
                    continue
                }
            }
            break
        }
        scanned = search - start
        if start != buffer.startIndex { buffer = Data(buffer[start...]) }
        return messages
    }

    mutating func finish() -> Data? {
        let remainder = framing == .lines && !buffer.isEmpty ? buffer : nil
        buffer = Data()
        scanned = 0
        bodyLength = nil
        return remainder
    }

    private static func contentLength(in header: String) -> Int? {
        for line in header.split(whereSeparator: \.isNewline) {
            let parts = line.split(separator: ":", maxSplits: 1)
            guard parts.count == 2,
                  parts[0].trimmingCharacters(in: .whitespaces).lowercased() == "content-length" else { continue }
            guard let length = Int(parts[1].trimmingCharacters(in: .whitespaces)), length >= 0 else { return nil }
            return length
        }
        return nil
    }
}
