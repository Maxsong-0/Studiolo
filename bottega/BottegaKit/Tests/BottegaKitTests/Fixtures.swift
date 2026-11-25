// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import BottegaKit

/// LMAS's own test data, copied as it is (see Fixtures/README.md). The checks compare against these files, never
/// against something this code produced.
enum Fixtures {
    static func data(_ name: String) throws -> Data {
        let url = try #require(Bundle.module.url(forResource: "Fixtures/\(name)", withExtension: nil), "no \(name)")
        return try Data(contentsOf: url)
    }

    static func lines(_ name: String) throws -> [Data] {
        try data(name).split(separator: UInt8(ascii: "\n")).map { Data($0) }
    }

    static func vectors() throws -> Vectors { try JSONDecoder().decode(Vectors.self, from: data("vectors.json")) }

    static func records() throws -> [Record] {
        try lines("events.jsonl").map { line in
            guard case .record(let record) = try ServerLine.read(line) else { throw NotARecord(line: String(decoding: line, as: UTF8.self)) }
            return record
        }
    }

    static func state() throws -> WorkshopState {
        var state = WorkshopState()
        for record in try records() { try state.apply(record) }
        return state
    }

    struct NotARecord: Error { var line: String }
}

struct Vectors: Decodable {
    var keys: Keys
    var handshake: Handshake
    var register: Register
    var intents: [Intent]
    var refused_intents: [Refused]
    var text_sha256: [TextDigest]

    struct Keys: Decodable { var daily, approval: Pair }
    struct Pair: Decodable { var `private`, `public`: String }
    struct Handshake: Decodable { var connection: UInt64; var nonce, preimage, digest, signature: String }
    struct Register: Decodable { var connection: UInt64; var nonce, preimage, digest: String; var registration: Registration }
    struct Intent: Decodable { var intent: SignedIntent; var key, preimage, digest: String }
    struct Refused: Decodable { var intent: SignedIntent; var why: String }
    struct TextDigest: Decodable { var text, sha256: String }

    var daily: PublicKey { get throws { try #require(PublicKey(hex: keys.daily.public)) } }
    var approval: PublicKey { get throws { try #require(PublicKey(hex: keys.approval.public)) } }
}
