// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import BottegaKit
import CryptoKit
public import Foundation
import LocalAuthentication
import Security

/// The device's keys, in the Secure Enclave. What is stored is a blob only this Mac's Enclave can turn back into a
/// key: it says nothing by itself, but any program on this Mac that reads the daily key's blob can sign with it. The
/// approval key needs the enrolled finger each time, so its blob alone is worth nothing.
public struct SecureKeys: DeviceKeys {
    private let dailyBlob: Data
    private let approvalBlob: Data
    public let daily: PublicKey
    public let approval: PublicKey

    init(daily dailyBlob: Data, approval approvalBlob: Data) throws {
        self.dailyBlob = dailyBlob
        self.approvalBlob = approvalBlob
        daily = try Self.publicKey(Self.key(dailyBlob, LAContext()))
        approval = try Self.publicKey(Self.key(approvalBlob, LAContext()))
    }

    public func sign(_ preimage: Preimage, with use: KeyUse, reason: String) async throws -> String {
        let blob = use == .daily ? dailyBlob : approvalBlob
        return try await Self.sign(preimage, blob, reason: use == .approval ? reason : nil)
    }

    /// Off the main thread: the Touch ID prompt holds its caller until the finger comes.
    @concurrent
    private static func sign(_ preimage: Preimage, _ blob: Data, reason: String?) async throws -> String {
        // A new one each time, so every approval prompts (vault Bottega-3, section 2.2).
        let context = LAContext()
        context.touchIDAuthenticationAllowableReuseDuration = 0
        if let reason { context.localizedReason = reason }
        do {
            let signature = try key(blob, context).signature(for: preimage.bytes)
            return Hex.encode(signature.rawRepresentation)
        } catch let error as LAError {
            throw SigningRefused(text(error))
        } catch {
            throw SigningRefused("The Secure Enclave would not sign: \(error.localizedDescription)")
        }
    }

    private static func text(_ error: LAError) -> String {
        switch error.code {
        case .userCancel, .appCancel, .systemCancel: "Touch ID was cancelled."
        case .biometryNotEnrolled, .biometryNotAvailable: "Touch ID is not set up on this Mac."
        case .biometryLockout: "Touch ID is locked. Unlock this Mac with its password, then try again."
        case .authenticationFailed: "That finger is not enrolled."
        default: "Touch ID did not go through: \(error.localizedDescription)"
        }
    }

    private static func key(_ blob: Data, _ context: LAContext) throws -> SecureEnclave.P256.Signing.PrivateKey {
        try .init(dataRepresentation: blob, authenticationContext: context)
    }

    private static func publicKey(_ key: SecureEnclave.P256.Signing.PrivateKey) throws -> PublicKey {
        guard let key = PublicKey(x963: key.publicKey.x963Representation) else {
            throw SigningRefused("The Secure Enclave returned a key of another kind.")
        }
        return key
    }

    /// Makes both and returns their blobs.
    static func make() throws -> (daily: Data, approval: Data) {
        guard SecureEnclave.isAvailable else { throw SigningRefused("This Mac has no Secure Enclave.") }
        let daily = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access([.privateKeyUsage]))
        // Only the fingers enrolled now, and never the password: enrolling another retires the key.
        let approval = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access([.privateKeyUsage, .biometryCurrentSet]))
        return (daily.dataRepresentation, approval.dataRepresentation)
    }

    private static func access(_ flags: SecAccessControlCreateFlags) throws -> SecAccessControl {
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, &error) else {
            throw error?.takeRetainedValue() ?? SigningRefused("No access control.") as any Error
        }
        return access
    }
}

/// Where the blobs are. A release keeps them in the keychain, in items only Bottega may read without macOS asking;
/// a Debug build is signed ad hoc, differently every time, so its keychain items would ask at every build. It keeps
/// them in private files instead, under its own name, and never sees the release's.
public struct SecureChest: KeyChest {
    private let isDevelopment: Bool
    private let folder: URL
    private let service: String

    /// `folder` is used by a Debug build, `service` by a release.
    public init(isDevelopment: Bool, folder: URL, service: String) {
        self.isDevelopment = isDevelopment
        self.folder = folder
        self.service = service
    }

    private enum Name: String {
        case daily = "daily-key"
        case approval = "approval-key"
    }

    public func keys() throws -> (any DeviceKeys)? {
        guard let daily = try read(.daily), let approval = try read(.approval) else { return nil }
        return try SecureKeys(daily: daily, approval: approval)
    }

    public func makeKeys() throws -> any DeviceKeys {
        let (daily, approval) = try SecureKeys.make()
        try write(daily, .daily)
        try write(approval, .approval)
        return try SecureKeys(daily: daily, approval: approval)
    }

    private func read(_ name: Name) throws -> Data? {
        if isDevelopment {
            do {
                return try Data(contentsOf: folder.appendingPathComponent(name.rawValue))
            } catch CocoaError.fileReadNoSuchFile {
                return nil
            }
        }
        var query = item(name)
        query[kSecReturnData] = true
        var found: CFTypeRef?
        switch SecItemCopyMatching(query as CFDictionary, &found) {
        case errSecSuccess: return found as? Data
        case errSecItemNotFound: return nil
        case let status: throw Self.error(status)
        }
    }

    private func write(_ blob: Data, _ name: Name) throws {
        if isDevelopment {
            return try PrivateFile.write(blob, to: folder.appendingPathComponent(name.rawValue))
        }
        SecItemDelete(item(name) as CFDictionary)
        var attributes = item(name)
        attributes[kSecValueData] = blob
        attributes[kSecAttrAccessible] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        let status = SecItemAdd(attributes as CFDictionary, nil)
        guard status == errSecSuccess else { throw Self.error(status) }
    }

    private func item(_ name: Name) -> [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: name.rawValue,
            kSecAttrSynchronizable: false,
        ]
    }

    private static func error(_ status: OSStatus) -> any Error {
        NSError(domain: NSOSStatusErrorDomain, code: Int(status), userInfo: [
            NSLocalizedDescriptionKey: SecCopyErrorMessageString(status, nil) as String? ?? "Keychain error \(status)"
        ])
    }
}
