import CryptoKit
import Foundation
import Security

/// Wraps the core's data key with an AES-256-GCM key that never leaves this device's keychain (the Android app's
/// Keystore wrapper). The key is readable after the first unlock so pushes can be handled on a locked phone, is
/// not synced or backed up (`ThisDeviceOnly`), and lives in the shared keychain group so the notification extension
/// opens the same store. Output is nonce || ciphertext || tag.
///
/// The core treats `Failed` from `unwrap` as a lost key and starts over (it deletes the session and what waits), so
/// only a definite answer may say that: a keychain that cannot be read right now (locked before the first unlock,
/// interaction not allowed) is `NeedsUserInteraction`, which makes opening the store fail without changing anything.
final class KeychainKeyWrapper: KeyWrapper, @unchecked Sendable {
    private let service = "com.reins2fa.app.key-wrap"
    private let account = "core-dek-wrap-v1"
    private let lock = NSLock()
    private var cached: SymmetricKey?
    /// False in the notification extension: it never creates a key or a data key and never lets the core start over
    /// (a keychain group it cannot see would otherwise wipe the app's store); the app is the only one that may.
    private let mayReset: Bool

    init(mayReset: Bool = true) {
        self.mayReset = mayReset
    }

    func wrap(plaintext: Data) throws -> Data {
        guard mayReset else { throw ForeignError.NeedsUserInteraction }
        do {
            let box = try AES.GCM.seal(plaintext, using: try key(create: true))
            guard let combined = box.combined else { throw ForeignError.Failed(reason: "sealing failed") }
            return combined
        } catch let error as ForeignError {
            throw error
        } catch {
            throw ForeignError.Failed(reason: "sealing failed")
        }
    }

    func unwrap(wrapped: Data) throws -> Data {
        do {
            return try AES.GCM.open(AES.GCM.SealedBox(combined: wrapped), using: try key(create: false))
        } catch let error as ForeignError {
            if !mayReset { throw ForeignError.NeedsUserInteraction }
            throw error
        } catch {
            if !mayReset { throw ForeignError.NeedsUserInteraction }
            throw ForeignError.Failed(reason: "the stored key does not open this data")
        }
    }

    private func baseQuery() -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let group = AppGroup.keychainGroup { query[kSecAttrAccessGroup as String] = group }
        return query
    }

    private func key(create: Bool) throws -> SymmetricKey {
        lock.lock()
        defer { lock.unlock() }
        if let cached { return cached }
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecSuccess, let data = item as? Data, data.count == 32 {
            let key = SymmetricKey(data: data)
            cached = key
            return key
        }
        guard status == errSecItemNotFound else {
            // Locked, interaction not allowed, or another passing state: the key is there but cannot be read now.
            throw ForeignError.NeedsUserInteraction
        }
        guard create else { throw ForeignError.Failed(reason: "the key store has no key for this data") }
        let key = SymmetricKey(size: .bits256)
        var add = baseQuery()
        add[kSecValueData as String] = key.withUnsafeBytes { Data($0) }
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(add as CFDictionary, nil)
        guard added == errSecSuccess else { throw ForeignError.Failed(reason: "the key store refused a new key (\(added))") }
        cached = key
        return key
    }
}
