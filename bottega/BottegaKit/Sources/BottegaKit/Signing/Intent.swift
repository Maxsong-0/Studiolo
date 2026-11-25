// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Which of the device's two keys signs.
public enum KeyUse: String, Sendable {
    /// Signs without a prompt.
    case daily
    /// Touch ID, every time.
    case approval
}

/// A text file sent along with something the user says.
public struct TextFile: Codable, Equatable, Hashable, Sendable {
    public var name: String
    public var content: String

    public init(name: String, content: String) {
        self.name = name
        self.content = content
    }
}

/// What the user asks LMAS to do. Not signed, not numbered yet.
public enum IntentDraft: Codable, Equatable, Sendable {
    /// Say something to a session. Without `thread` this starts a line; with it, it interjects on that line, where
    /// `wantsReply` false means "for your information".
    case say(to: String, thread: String?, text: String, files: [TextFile], wantsReply: Bool)
    /// Answer a question an agent asked the user.
    case answer(message: String, text: String, files: [TextFile])
    /// Approve what LMAS asked. `textDigest` is the SHA-256 of the text as it was shown.
    case approve(ask: String, textDigest: String)
    case decline(ask: String)

    public var kind: String {
        switch self {
        case .say: "say"
        case .answer: "answer"
        case .approve: "approve"
        case .decline: "decline"
        }
    }

    public var key: KeyUse {
        switch self {
        case .approve: .approval
        case .say, .answer, .decline: .daily
        }
    }

    /// Which key a kind must be signed with, for checking an intent that came from elsewhere.
    public static func key(forKind kind: String) -> KeyUse? {
        switch kind {
        case "approve": .approval
        case "say", "answer", "decline", "heartbeat": .daily
        default: nil
        }
    }

    /// What the user typed, if they typed anything: goes back into the composer when LMAS does not take it.
    public var typed: (text: String, files: [TextFile])? {
        switch self {
        case .say(_, _, let text, let files, _), .answer(_, let text, let files): (text, files)
        case .approve, .decline: nil
        }
    }

    /// The limits LMAS states in section 7. Checked before signing so the user hears about them at the keyboard,
    /// not after a round trip.
    public enum Problem: Error, Equatable, Sendable {
        case empty
        case textTooLong(bytes: Int)
        case tooManyFiles(Int)
        case fileTooLong(name: String)
        case filesTooLong(bytes: Int)
        case fileWithoutName
        case newThreadMustWantReply
    }

    public enum Limit {
        public static let text = 64 * 1024
        public static let files = 10
        public static let file = 64 * 1024
        public static let allFiles = 128 * 1024
    }

    public func check() throws(Problem) {
        if case .say(_, nil, _, _, false) = self { throw .newThreadMustWantReply }
        guard let (text, files) = typed else { return }
        if text.allSatisfy(\.isWhitespace) { throw .empty }
        if text.utf8.count > Limit.text { throw .textTooLong(bytes: text.utf8.count) }
        if files.count > Limit.files { throw .tooManyFiles(files.count) }
        for file in files {
            if file.name.allSatisfy(\.isWhitespace) { throw .fileWithoutName }
            if file.content.utf8.count > Limit.file { throw .fileTooLong(name: file.name) }
        }
        let total = files.reduce(0) { $0 + $1.content.utf8.count }
        if total > Limit.allFiles { throw .filesTooLong(bytes: total) }
    }

    /// The body to sign. LMAS refuses a field it does not know, so nothing is written that section 7 does not list,
    /// and an absent field is left out rather than written as null.
    public func body(contentID: String) throws -> String {
        var fields = BodyFields(content_id: contentID)
        switch self {
        case .say(let to, let thread, let text, let files, let wantsReply):
            fields.to = to
            fields.thread = thread
            fields.text = text
            fields.files = files.isEmpty ? nil : files
            fields.wants_reply = wantsReply ? nil : false
        case .answer(let message, let text, let files):
            fields.message = message
            fields.text = text
            fields.files = files.isEmpty ? nil : files
        case .approve(let ask, let textDigest):
            fields.ask = ask
            fields.text_sha256 = textDigest
        case .decline(let ask):
            fields.ask = ask
        }
        return String(decoding: try Wire.encoder.encode(fields), as: UTF8.self)
    }

    /// Spelled as on the wire; nil fields are not written.
    private struct BodyFields: Encodable {
        var content_id: String
        var to: String?
        var thread: String?
        var text: String?
        var files: [TextFile]?
        var wants_reply: Bool?
        var message: String?
        var ask: String?
        var text_sha256: String?
    }
}

/// The 128 bits a device gives one thing the user wants done. Stays the same when the intent is numbered again.
public enum ContentID {
    public static func random() -> String {
        var generator = SystemRandomNumberGenerator()
        return Hex.encode((0..<16).map { _ in UInt8.random(in: 0...255, using: &generator) })
    }
}

/// An intent as it goes over the wire. Signing the same draft twice gives two of these; resending must reuse one.
public struct SignedIntent: Codable, Equatable, Sendable {
    public var device: String
    public var number: UInt64
    public var createdAt: UInt64
    public var epoch: UInt64
    public var kind: String
    public var body: String
    public var signature: String

    /// The only key epoch this version of the protocol has: the body is not encrypted.
    public static let plainEpoch: UInt64 = 0

    public init(
        device: String, number: UInt64, createdAt: UInt64, epoch: UInt64 = plainEpoch, kind: String, body: String,
        signature: String
    ) {
        self.device = device
        self.number = number
        self.createdAt = createdAt
        self.epoch = epoch
        self.kind = kind
        self.body = body
        self.signature = signature
    }

    private enum CodingKeys: String, CodingKey {
        case device, number, epoch, kind, body, signature
        case createdAt = "created_at_ms"
    }

    public var preimage: Preimage {
        .intent(device: device, number: number, createdAt: createdAt, epoch: epoch, kind: kind, body: body)
    }

    public var digest: String { preimage.digest }

    /// Whether the right one of the two keys signed exactly this.
    public func isSigned(daily: PublicKey, approval: PublicKey) -> Bool {
        guard epoch == Self.plainEpoch, number > 0, let use = IntentDraft.key(forKind: kind) else { return false }
        return (use == .daily ? daily : approval).verifies(preimage, signature: signature)
    }
}
