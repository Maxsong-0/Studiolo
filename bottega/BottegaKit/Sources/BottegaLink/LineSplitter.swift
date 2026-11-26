// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation

/// Cuts a byte stream into lines.
public struct LineSplitter: Sendable {
    /// A page of events is about 4 MiB, but a single record is always given whole. Past this it is not our peer.
    public static let maxLineBytes = 64 * 1024 * 1024

    public struct LineTooLong: Error, Equatable {}

    private let limit: Int
    private var pending = Data()
    /// How much of `pending` is known to hold no newline, so a long line is not searched again for every chunk.
    private var searched = 0

    /// `limit`: how much of a line may be waiting for its newline. Other than the default only in tests.
    public init(limit: Int = LineSplitter.maxLineBytes) { self.limit = limit }

    public mutating func take(_ chunk: Data) throws(LineTooLong) -> [Data] {
        pending.append(chunk)
        var lines: [Data] = []
        var start = pending.startIndex
        var from = start + searched
        while let newline = pending[from...].firstIndex(of: UInt8(ascii: "\n")) {
            lines.append(Data(pending[start..<newline]))
            start = newline + 1
            from = start
        }
        // Once per chunk, not once per line: a chunk of many short lines is not copied over and over.
        if start != pending.startIndex { pending = Data(pending[start...]) }
        searched = pending.count
        if pending.count > limit { throw LineTooLong() }
        return lines
    }
}
