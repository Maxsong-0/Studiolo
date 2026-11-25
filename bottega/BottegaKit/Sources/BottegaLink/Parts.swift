// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import BottegaKit
public import CryptoKit
public import Foundation

/// One open connection, a line at a time.
public protocol LineChannel: Sendable {
    /// Everything the other end writes, newlines removed. Ends when it hangs up. Taken once.
    func lines() -> AsyncThrowingStream<Data, any Error>
    /// `line` ends in a newline.
    func send(_ line: Data) async throws
    func close()
}

public protocol Connector: Sendable {
    func connect() async throws -> any LineChannel
}

/// The device's two keys. The approval key asks for Touch ID each time.
public protocol DeviceKeys: Sendable {
    var daily: PublicKey { get }
    var approval: PublicKey { get }
    /// Hex of r then s. `reason` is what the Touch ID prompt says.
    func sign(_ preimage: Preimage, with use: KeyUse, reason: String) async throws -> String
}

/// Where the device's keys live.
public protocol KeyChest: Sendable {
    /// nil when this device has none yet.
    func keys() throws -> (any DeviceKeys)?
    /// Makes both. Registering is the only reason to.
    func makeKeys() throws -> any DeviceKeys
}

/// The user said no, or the key would not sign.
public struct SigningRefused: Error, Equatable {
    public let text: String
    public init(_ text: String) { self.text = text }
}

/// What has to outlive the app: which device this is to that LMAS, and what has not been settled.
public struct Keepsake: Codable, Equatable, Sendable {
    public var device: String?
    public var outbox = Outbox()
    public init() {}
}

public protocol KeepsakeStore: Sendable {
    func load() throws -> Keepsake
    func save(_ keepsake: Keepsake) throws
}

/// A private file, replaced whole.
public struct FileKeepsakeStore: KeepsakeStore {
    private let url: URL

    public init(url: URL) { self.url = url }

    public func load() throws -> Keepsake {
        do {
            return try JSONDecoder().decode(Keepsake.self, from: try Data(contentsOf: url))
        } catch CocoaError.fileReadNoSuchFile {
            return Keepsake()
        }
    }

    public func save(_ keepsake: Keepsake) throws {
        try PrivateFile.write(try JSONEncoder().encode(keepsake), to: url)
    }
}

public enum PrivateFile {
    /// Readable by this user only, at every moment: the new file is made 0600 before anything is in it, then takes
    /// the old one's place.
    public static func write(_ data: Data, to url: URL) throws {
        let folder = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(
            at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let new = folder.appendingPathComponent(".\(url.lastPathComponent).\(UUID().uuidString)")
        guard FileManager.default.createFile(atPath: new.path, contents: data, attributes: [.posixPermissions: 0o600]) else {
            throw CocoaError(.fileWriteUnknown)
        }
        do {
            let file = try FileHandle(forWritingTo: new)
            try file.synchronize()
            try file.close()
            guard rename(new.path, url.path) == 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }
        } catch {
            try? FileManager.default.removeItem(at: new)
            throw error
        }
    }
}

/// Keys in memory, no prompt. For tests, and for Debug builds trying out a demo LMAS.
public struct SoftwareKeys: DeviceKeys {
    private let dailyKey: P256.Signing.PrivateKey
    private let approvalKey: P256.Signing.PrivateKey

    public init(daily: P256.Signing.PrivateKey = .init(), approval: P256.Signing.PrivateKey = .init()) {
        dailyKey = daily
        approvalKey = approval
    }

    public var daily: PublicKey { Self.publicKey(dailyKey) }
    public var approval: PublicKey { Self.publicKey(approvalKey) }

    public func sign(_ preimage: Preimage, with use: KeyUse, reason: String) throws -> String {
        let key = use == .daily ? dailyKey : approvalKey
        return Hex.encode(try key.signature(for: preimage.bytes).rawRepresentation)
    }

    private static func publicKey(_ key: P256.Signing.PrivateKey) -> PublicKey {
        // A key CryptoKit just made is on the curve and 65 bytes long.
        PublicKey(x963: key.publicKey.x963Representation)!
    }
}
