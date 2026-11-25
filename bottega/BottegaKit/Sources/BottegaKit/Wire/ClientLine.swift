// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation

/// A device's registration. Both signatures cover the same bytes (`Preimage.register`).
public struct Registration: Codable, Equatable, Sendable {
    public var kind: String
    public var name: String
    public var dailyKey: String
    public var approvalKey: String
    public var dailySignature: String
    public var approvalSignature: String

    /// The only kind this version registers.
    public static let mac = "mac"
    public static let maxNameBytes = 64

    private enum CodingKeys: String, CodingKey {
        case kind, name
        case dailyKey = "daily_key"
        case approvalKey = "approval_key"
        case dailySignature = "daily_signature"
        case approvalSignature = "approval_signature"
    }

    public init(
        kind: String = mac, name: String, dailyKey: String, approvalKey: String, dailySignature: String,
        approvalSignature: String
    ) {
        self.kind = kind
        self.name = name
        self.dailyKey = dailyKey
        self.approvalKey = approvalKey
        self.dailySignature = dailySignature
        self.approvalSignature = approvalSignature
    }

    /// A name LMAS will take: one line, no control characters, at most 64 bytes, cut between characters.
    public static func usableName(_ name: String) -> String? {
        var kept = ""
        for character in UntrustedText.line(name) {
            if kept.utf8.count + String(character).utf8.count > maxNameBytes { break }
            kept.append(character)
        }
        kept = kept.trimmingCharacters(in: .whitespaces)
        return kept.isEmpty ? nil : kept
    }

    /// Both keys well formed and different, both signatures over this connection's numbers.
    public func isSigned(nonce: Data, connection: UInt64) -> Bool {
        guard let daily = PublicKey(hex: dailyKey), let approval = PublicKey(hex: approvalKey), daily != approval else {
            return false
        }
        let preimage = Preimage.register(
            kind: kind, name: name, dailyKey: daily.bytes, approvalKey: approval.bytes, nonce: nonce,
            connection: connection)
        return daily.verifies(preimage, signature: dailySignature)
            && approval.verifies(preimage, signature: approvalSignature)
    }
}

public enum Request: Equatable, Sendable {
    case overview
    case events(from: UInt64, limit: Int)
    case follow(from: UInt64)
    case seen(through: UInt64)
    case intent(SignedIntent)
    case register(Registration)
    case ping
}

/// A line to LMAS, newline included.
public enum ClientLine {
    /// LMAS disconnects on a line over 1 MiB, so one is never sent.
    public struct TooLong: Error, Equatable { public var bytes: Int }

    public static func hello(device: String, signature: String) throws -> Data {
        try line(Fields(hello: "bottega", version: Wire.version, device: device, signature: signature))
    }

    public static func helloUnregistered() throws -> Data {
        try line(Fields(hello: "bottega-unregistered", version: Wire.version))
    }

    public static func request(id: UInt64, _ request: Request) throws -> Data {
        var fields = Fields(id: id)
        switch request {
        case .overview:
            fields.verb = "overview"
        case .events(let from, let limit):
            fields.verb = "events"
            fields.from = from
            fields.limit = limit
        case .follow(let from):
            fields.verb = "follow"
            fields.from = from
        case .seen(let through):
            fields.verb = "seen"
            fields.through = through
        case .intent(let intent):
            fields.verb = "intent"
            fields.intent = intent
        case .register(let registration):
            fields.verb = "register"
            fields.registration = registration
        case .ping:
            fields.verb = "ping"
        }
        return try line(fields)
    }

    private static func line(_ fields: Fields) throws -> Data {
        var line = try Wire.encoder.encode(fields)
        line.append(UInt8(ascii: "\n"))
        if line.count > Wire.maxLineBytes { throw TooLong(bytes: line.count) }
        return line
    }

    private struct Fields: Encodable {
        var id: UInt64?
        var verb: String?
        var hello: String?
        var version: Int?
        var device: String?
        var signature: String?
        var from: UInt64?
        var limit: Int?
        var through: UInt64?
        var intent: SignedIntent?
        var registration: Registration?

        enum CodingKeys: String, CodingKey {
            case id, hello, version, device, signature, from, limit, through, intent, registration
            case verb = "do"
        }
    }
}
