import CryptoKit
import Foundation
import Security

/// Wraps the core's data key with an AES-256-GCM key that never leaves this device's keychain (the Android app's
/// Keystore wrapper). The key is readable after the first unlock so pushes can be handled on a locked phone, is
/// not synced or backed up (`ThisDeviceOnly`), and lives in the shared keychain group so the notification extension
/// opens the same store. Output is nonce || ciphertext || tag.
final class KeychainKeyWrapper: KeyWrapper, @unchecked Sendable {
    private let service = "dev.rewarden.ios.key-wrap"
    private let account = "core-dek-wrap-v1"
    private let lock = NSLock()
    private var cached: SymmetricKey?

    func wrap(plaintext: Data) throws -> Data {
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
            throw error
        } catch {
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
        guard status == errSecItemNotFound, create else {
            throw ForeignError.Failed(reason: "the key store is unavailable (\(status))")
        }
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
