// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import CryptoKit
import Foundation
import Testing
@testable import BottegaKit

/// Byte for byte against LMAS's `vectors.json`.
struct VectorTests {
    @Test func handshake() throws {
        let vectors = try Fixtures.vectors()
        let vector = vectors.handshake
        let nonce = try #require(Hex.decode(vector.nonce, count: Preimage.nonceBytes))
        let preimage = Preimage.handshake(nonce: nonce, connection: vector.connection)
        #expect(Hex.encode(preimage.bytes) == vector.preimage)
        #expect(preimage.digest == vector.digest)
        #expect(try vectors.daily.verifies(preimage, signature: vector.signature))
        #expect(try !vectors.approval.verifies(preimage, signature: vector.signature))
    }

    @Test func registration() throws {
        let vectors = try Fixtures.vectors()
        let vector = vectors.register
        let nonce = try #require(Hex.decode(vector.nonce, count: Preimage.nonceBytes))
        let registration = vector.registration
        let daily = try vectors.daily, approval = try vectors.approval
        #expect(registration.dailyKey == daily.hex)
        #expect(registration.approvalKey == approval.hex)
        let preimage = Preimage.register(
            kind: registration.kind, name: registration.name, dailyKey: daily.bytes, approvalKey: approval.bytes,
            nonce: nonce, connection: vector.connection)
        #expect(Hex.encode(preimage.bytes) == vector.preimage)
        #expect(preimage.digest == vector.digest)
        #expect(daily.verifies(preimage, signature: registration.dailySignature))
        #expect(approval.verifies(preimage, signature: registration.approvalSignature))
        #expect(registration.isSigned(nonce: nonce, connection: vector.connection))
    }

    @Test func registrationChangedInAnyWayIsNotSigned() throws {
        let vector = try Fixtures.vectors().register
        let nonce = try #require(Hex.decode(vector.nonce, count: Preimage.nonceBytes))
        let connection = vector.connection
        func stillSigned(_ change: (inout Registration) -> Void) -> Bool {
            var registration = vector.registration
            change(&registration)
            return registration.isSigned(nonce: nonce, connection: connection)
        }
        #expect(!stillSigned { $0.name += "x" })
        #expect(!stillSigned { $0.kind = "iphone" })
        #expect(!stillSigned { swap(&$0.dailyKey, &$0.approvalKey) })
        #expect(!stillSigned { swap(&$0.dailySignature, &$0.approvalSignature) })
        #expect(!stillSigned { $0.approvalKey = $0.dailyKey; $0.approvalSignature = $0.dailySignature })
        #expect(!vector.registration.isSigned(nonce: nonce, connection: connection + 1))
        #expect(!vector.registration.isSigned(nonce: Data(repeating: 0xac, count: 32), connection: connection))
    }

    @Test func everyIntent() throws {
        let vectors = try Fixtures.vectors()
        #expect(vectors.intents.count == 7)
        for vector in vectors.intents {
            let intent = vector.intent
            #expect(Hex.encode(intent.preimage.bytes) == vector.preimage, "\(intent.kind) \(intent.number)")
            #expect(intent.digest == vector.digest)
            #expect(IntentDraft.key(forKind: intent.kind)?.rawValue == vector.key)
            let signer = vector.key == "approval" ? try vectors.approval : try vectors.daily
            let other = vector.key == "approval" ? try vectors.daily : try vectors.approval
            #expect(signer.verifies(intent.preimage, signature: intent.signature))
            #expect(!other.verifies(intent.preimage, signature: intent.signature))
            #expect(intent.isSigned(daily: try vectors.daily, approval: try vectors.approval))
        }
        #expect(Set(vectors.intents.map(\.intent.kind)) == ["say", "answer", "approve", "decline", "heartbeat"])
    }

    @Test func everyRefusedIntentIsRefused() throws {
        let vectors = try Fixtures.vectors()
        #expect(vectors.refused_intents.count == 11)
        for refused in vectors.refused_intents {
            #expect(
                !refused.intent.isSigned(daily: try vectors.daily, approval: try vectors.approval), "\(refused.why)")
        }
    }

    @Test func textDigests() throws {
        for vector in try Fixtures.vectors().text_sha256 {
            #expect(Digest.sha256(vector.text) == vector.sha256, "\(vector.text)")
        }
    }

    /// The vectors' bodies are also what this code writes for the same content.
    @Test func bodiesComeOutAsInTheVectors() throws {
        let bodies = try Fixtures.vectors().intents.map(\.intent.body)
        let asks = try Fixtures.vectors().text_sha256
        func id(_ number: Int) -> String { String(repeating: "0", count: 31) + "\(number)" }
        #expect(
            try IntentDraft.say(to: "dispatcher", thread: nil, text: "给设置页加一个「清空缓存」按钮。", files: [], wantsReply: true)
                .body(contentID: id(1)) == bodies[0])
        #expect(
            try IntentDraft.say(to: "coder-card-1", thread: "msg-1", text: "按钮放在最下面。", files: [], wantsReply: false)
                .body(contentID: id(2)) == bodies[1])
        #expect(
            try IntentDraft.answer(
                message: "msg-3", text: "清掉图片缓存就行。",
                files: [.init(name: "范围.txt", content: "只清 ~/Library/Caches 里的\n")]
            ).body(contentID: id(3)) == bodies[2])
        #expect(
            try IntentDraft.approve(ask: "ask-1", textDigest: Digest.sha256(asks[1].text)).body(contentID: id(4)) == bodies[3])
        #expect(try IntentDraft.decline(ask: "ask-2").body(contentID: id(5)) == bodies[4])
        let asked = try #require(try Fixtures.state().asks["ask-3"])
        #expect(try IntentDraft.approve(ask: "ask-3", textDigest: asked.textDigest).body(contentID: id(7)) == bodies[6])
    }

    /// Signing with the vectors' private keys gives something the vectors' public keys take. The signature itself
    /// differs every time.
    @Test func whatTheTestKeysSignVerifies() throws {
        let vectors = try Fixtures.vectors()
        let raw = try #require(Hex.decode(vectors.keys.daily.private, count: 32))
        let key = try P256.Signing.PrivateKey(rawRepresentation: raw)
        #expect(Hex.encode(key.publicKey.x963Representation) == vectors.keys.daily.public)
        let preimage = try #require(vectors.intents.first).intent.preimage
        let signature = Hex.encode(try key.signature(for: preimage.bytes).rawRepresentation)
        #expect(try vectors.daily.verifies(preimage, signature: signature))
        #expect(try !vectors.approval.verifies(preimage, signature: signature))
    }

    @Test func anIntentThatIsNotPlainOrHasNoNumberIsRefused() throws {
        let vectors = try Fixtures.vectors()
        let first = try #require(vectors.intents.first).intent
        let key = try P256.Signing.PrivateKey(
            rawRepresentation: try #require(Hex.decode(vectors.keys.daily.private, count: 32)))
        func signed(_ change: (inout SignedIntent) -> Void) throws -> Bool {
            var intent = first
            change(&intent)
            intent.signature = Hex.encode(try key.signature(for: intent.preimage.bytes).rawRepresentation)
            return intent.isSigned(daily: try vectors.daily, approval: try vectors.approval)
        }
        #expect(try signed { _ in })
        #expect(try !signed { $0.epoch = 1 })
        #expect(try !signed { $0.number = 0 })
        #expect(try !signed { $0.kind = "unheard_of" })
        #expect(try !signed { $0.kind = "approve" })
    }
}

struct HexTests {
    @Test func spelledOneWayOnly() {
        let bytes = Data([0x00, 0x0f, 0xa5, 0xff])
        #expect(Hex.encode(bytes) == "000fa5ff")
        #expect(Hex.decode("000fa5ff", count: 4) == bytes)
        #expect(Hex.decode("", count: 0) == Data())
        for bad in ["000fa5f", "000fa5ff0", "000FA5FF", "0x0fa5ff", "000fa5f ", "000fa5fg", "", "é0é0aa"] {
            #expect(Hex.decode(bad, count: 4) == nil, "\(bad)")
        }
    }

    @Test func publicKeysMustBeOnTheCurveAndUncompressed() throws {
        let good = try Fixtures.vectors().keys.daily.public
        #expect(PublicKey(hex: good) != nil)
        #expect(PublicKey(hex: good.uppercased()) == nil)
        #expect(PublicKey(hex: String(good.dropLast(2))) == nil)
        #expect(PublicKey(hex: "02" + good.dropFirst(2).prefix(64)) == nil)
        #expect(PublicKey(hex: "04" + String(repeating: "11", count: 64)) == nil)
        #expect(PublicKey(hex: "06" + good.dropFirst(2)) == nil)
    }
}

struct IntentTests {
    @Test func limitsAreCheckedBeforeSigning() {
        func problem(_ draft: IntentDraft) -> IntentDraft.Problem? {
            do throws(IntentDraft.Problem) { try draft.check() } catch { return error }
            return nil
        }
        func say(_ text: String, files: [TextFile] = [], thread: String? = nil, wantsReply: Bool = true) -> IntentDraft {
            .say(to: "dispatcher", thread: thread, text: text, files: files, wantsReply: wantsReply)
        }
        #expect(problem(say("hello")) == nil)
        #expect(problem(say(" \n\t")) == .empty)
        #expect(problem(say("x", wantsReply: false)) == .newThreadMustWantReply)
        #expect(problem(say("x", thread: "msg-1", wantsReply: false)) == nil)
        #expect(problem(say(String(repeating: "x", count: 64 * 1024))) == nil)
        // Bytes, not characters.
        #expect(problem(say(String(repeating: "字", count: 22000))) == .textTooLong(bytes: 66000))
        let file = TextFile(name: "a.txt", content: "x")
        #expect(problem(say("x", files: Array(repeating: file, count: 10))) == nil)
        #expect(problem(say("x", files: Array(repeating: file, count: 11))) == .tooManyFiles(11))
        #expect(problem(say("x", files: [.init(name: " ", content: "x")])) == .fileWithoutName)
        let big = TextFile(name: "big", content: String(repeating: "x", count: 64 * 1024))
        #expect(problem(say("x", files: [big, big])) == nil)
        #expect(problem(say("x", files: [big, big, file])) == .filesTooLong(bytes: 128 * 1024 + 1))
        #expect(problem(say("x", files: [.init(name: "big", content: big.content + "x")])) == .fileTooLong(name: "big"))
        #expect(problem(.decline(ask: "ask-1")) == nil)
    }

    @Test func contentIDsAreSixteenFreshBytes() {
        let ids = Set((0..<100).map { _ in ContentID.random() })
        #expect(ids.count == 100)
        #expect(ids.allSatisfy { Hex.decode($0, count: 16) != nil })
    }

    @Test func onlyApprovingTakesTheApprovalKey() {
        #expect(IntentDraft.approve(ask: "a", textDigest: "").key == .approval)
        #expect(IntentDraft.decline(ask: "a").key == .daily)
        #expect(IntentDraft.answer(message: "m", text: "x", files: []).key == .daily)
        for draft: IntentDraft in [
            .say(to: "d", thread: nil, text: "x", files: [], wantsReply: true), .answer(message: "m", text: "x", files: []),
            .approve(ask: "a", textDigest: ""), .decline(ask: "a"),
        ] {
            #expect(IntentDraft.key(forKind: draft.kind) == draft.key)
        }
    }
}
