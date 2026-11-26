// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation

/// Cuts a byte stream into lines.
public struct LineSplitter: Sendable {
    /// A page of events is about 4 MiB, but a single record is always given whole. Past this it is not our peer.
    public static let maxLineBytes = 64 * 1024 * 1024

    public struct LineTooLong: Error, Equatable {}

    private var pending = Data()
    /// How much of `pending` is known to hold no newline, so a long line is not searched again for every chunk.
    private var searched = 0

    public init() {}

    public mutating func take(_ chunk: Data) throws(LineTooLong) -> [Data] {
        pending.append(chunk)
        var lines: [Data] = []
        while let newline = pending[(pending.startIndex + searched)...].firstIndex(of: UInt8(ascii: "\n")) {
            lines.append(Data(pending[pending.startIndex..<newline]))
            pending = Data(pending[(newline + 1)...])
            searched = 0
        }
        searched = pending.count
        if pending.count > Self.maxLineBytes { throw LineTooLong() }
        return lines
    }
}
