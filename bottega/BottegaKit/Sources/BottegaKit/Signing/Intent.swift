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
    /// Name, pin or put away a line. What is nil stays as it is; an empty title takes the title away. `line` is the
    /// number of its first letter, or a session's name.
    case arrange(line: String, title: String?, pinned: Bool?, archived: Bool?)
    /// A line has been read up to this event.
    case markRead(line: String, through: UInt64)
    /// Let agents work on copies of a repository.
    case addProject(name: String, path: String)
    case removeProject(name: String)
    case openLine(LineToOpen)
    /// From its next turn on, `session` uses this. Without `effort`, the model's own default.
    case setModel(line: String, session: String, model: String, effort: String?)
    case stopTurn(String)

    public var kind: String {
        switch self {
        case .say: "say"
        case .answer: "answer"
        case .approve: "approve"
        case .decline: "decline"
        case .arrange: "arrange"
        case .markRead: "mark_read"
        case .addProject: "add_project"
        case .removeProject: "remove_project"
        case .openLine: "open_line"
        case .setModel: "set_model"
        case .stopTurn: "stop_turn"
        }
    }

    public var key: KeyUse {
        switch self {
        case .approve, .addProject, .removeProject: .approval
        case .say, .answer, .decline, .arrange, .markRead, .openLine, .setModel, .stopTurn: .daily
        }
    }

    /// Which key a kind must be signed with, for checking an intent that came from elsewhere.
    public static func key(forKind kind: String) -> KeyUse? {
        switch kind {
        case "approve", "add_project", "remove_project": .approval
        case "say", "answer", "decline", "heartbeat", "arrange", "mark_read", "open_line", "set_model", "stop_turn": .daily
        default: nil
        }
    }

    /// What the user typed, if they typed anything: goes back into the composer when LMAS does not take it.
    public var typed: (text: String, files: [TextFile])? {
        switch self {
        case .say(_, _, let text, let files, _), .answer(_, let text, let files): (text, files)
        case .openLine(let line): line.text.map { ($0, line.files) }
        case .approve, .decline, .arrange, .markRead, .addProject, .removeProject, .setModel, .stopTurn: nil
        }
    }

    /// Whether it means the same when it is signed later, once LMAS is back. What was decided about the moment does
    /// not: it is dropped rather than done when the user no longer looks.
    public var keeps: Bool {
        switch self {
        case .say, .answer, .openLine, .arrange, .markRead: true
        case .approve, .decline, .addProject, .removeProject, .setModel, .stopTurn: false
        }
    }

    /// The limits LMAS states in sections 7 and 11. Checked before signing so the user hears about them at the keyboard,
    /// not after a round trip.
    public enum Problem: Error, Equatable, Sendable {
        case empty
        case textTooLong(bytes: Int)
        case tooManyFiles(Int)
        case fileTooLong(name: String)
        case filesTooLong(bytes: Int)
        case fileWithoutName
        case newThreadMustWantReply
        case nothingToArrange
        case badTitle
        case badProjectName
        case badProjectPath
        case filesWithoutText
        /// Within every limit above as typed, and still more than a line holds once it is written out.
        case tooLongOnTheWire(bytes: Int)
    }

    public enum Limit {
        public static let text = 64 * 1024
        public static let files = 10
        public static let file = 64 * 1024
        public static let allFiles = 128 * 1024
        public static let title = 200
        public static let projectName = 64
        /// The longest device number reckoned with when asking whether a line fits. LMAS's are a few bytes.
        static let device = 64
    }

    public func check() throws(Problem) {
        switch self {
        case .say(_, nil, _, _, false): throw .newThreadMustWantReply
        case .arrange(_, nil, nil, nil): throw .nothingToArrange
        case .arrange(_, let title?, _, _) where !Self.isTitle(title): throw .badTitle
        case .openLine(let line):
            if let title = line.title, !Self.isTitle(title) { throw .badTitle }
            if line.text == nil, !line.files.isEmpty { throw .filesWithoutText }
        case .addProject(let name, let path):
            if !Self.isProjectName(name) { throw .badProjectName }
            if !path.hasPrefix("/") { throw .badProjectPath }
        case .say, .answer, .approve, .decline, .arrange, .markRead, .removeProject, .setModel, .stopTurn: break
        }
        if let (text, files) = typed {
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
        // The limits above count what was typed. On the wire the body is JSON inside JSON, where one control
        // character takes seven bytes, and LMAS hangs up on a line over 1 MiB: signed and kept, such an intent could
        // never go out and would hold up everything behind it.
        let bytes = longestLineBytes
        if bytes > Wire.maxLineBytes { throw .tooLongOnTheWire(bytes: bytes) }
    }

    /// The most bytes the line carrying this can come to, newline not counted, whatever number it gets and whenever
    /// it is signed.
    var longestLineBytes: Int {
        let longest = SignedIntent(
            device: String(repeating: "d", count: Limit.device), number: Wire.farthest, createdAt: Wire.farthest,
            kind: kind, body: (try? body(contentID: String(repeating: "0", count: 32))) ?? "",
            signature: String(repeating: "0", count: 128))
        return ClientLine.bytes(id: Wire.farthest, .intent(longest))
    }

    /// LMAS leaves the space around a title out before it looks at the rest.
    private static func isTitle(_ text: String) -> Bool {
        let title = trimmed(text)
        return title.utf8.count <= Limit.title && !hasControls(title)
    }

    /// A name with space around it is refused, not trimmed.
    private static func isProjectName(_ text: String) -> Bool {
        (1...Limit.projectName).contains(text.utf8.count) && !hasControls(text) && trimmed(text) == text
    }

    /// Without what Unicode calls white space at either end, as LMAS (Rust's `trim`) sees it: scalar by scalar.
    /// Without the space around it, as LMAS reckons space (Unicode White_Space).
    static func trimmed(_ text: String) -> String {
        let isSpace = { (scalar: Unicode.Scalar) in scalar.properties.isWhitespace }
        let kept = text.unicodeScalars.drop(while: isSpace).reversed().drop(while: isSpace).reversed()
        return String(String.UnicodeScalarView(kept))
    }

    private static func hasControls(_ text: String) -> Bool {
        text.unicodeScalars.contains { $0.properties.generalCategory == .control }
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
        case .arrange(let line, let title, let pinned, let archived):
            fields.line = line
            fields.title = title
            fields.pinned = pinned
            fields.archived = archived
        case .markRead(let line, let through):
            fields.line = line
            fields.through = through
        case .addProject(let name, let path):
            fields.project = name
            fields.path = path
        case .removeProject(let name):
            fields.project = name
        case .openLine(let line):
            fields.project = line.project
            fields.vendor = line.vendor
            fields.role = line.role
            fields.model = line.model
            fields.effort = line.effort
            fields.title = line.title
            fields.text = line.text
            fields.files = line.files.isEmpty ? nil : line.files
        case .setModel(let line, let session, let model, let effort):
            fields.line = line
            fields.session = session
            fields.model = model
            fields.effort = effort
        case .stopTurn(let turn):
            fields.turn = turn
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
        var line, title: String?
        var pinned, archived: Bool?
        var through: UInt64?
        var project, path, vendor, role, model, effort, session, turn: String?
    }
}

/// A line straight to one agent, as the user wants it.
public struct LineToOpen: Codable, Equatable, Sendable {
    public var project, vendor, role: String
    /// nil for the vendor's default.
    public var model, effort: String?
    public var title: String?
    /// The first thing said on it, and what goes with that.
    public var text: String?
    public var files: [TextFile]

    public init(
        project: String, vendor: String, role: String, model: String? = nil, effort: String? = nil, title: String? = nil,
        text: String? = nil, files: [TextFile] = []
    ) {
        self.project = project
        self.vendor = vendor
        self.role = role
        self.model = model
        self.effort = effort
        self.title = title
        self.text = text
        self.files = files
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
