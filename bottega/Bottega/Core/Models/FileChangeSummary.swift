// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// What a turn changed, file by file, as the timeline and the diff panel show it.
struct FileChangeSummary: Codable, Hashable, Sendable {
    struct File: Codable, Hashable, Sendable {
        var path: String
        var additions: Int
        var deletions: Int
        var isBinary: Bool
    }

    let files: [File]
    let additions: Int
    let deletions: Int
}

extension FileChangeSummary {
    /// Read as the synthesized conformance reads it, except that the files are read one by
    /// one: a file this build cannot read goes alone, not the turn's summary row with it.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        files = container.values(.files)
        additions = try container.decode(Int.self, forKey: .additions)
        deletions = try container.decode(Int.self, forKey: .deletions)
    }
}
