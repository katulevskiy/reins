import CryptoKit
import Foundation
import Security

/// Widget data is sensitive too. Its key stays in this device's shared keychain, outside preferences and backups.
enum SealedSnapshot {
    private static let marker = Data("RSC1".utf8)
    private static func key(create: Bool) throws -> SymmetricKey {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.reins2fa.app.widget-cache",
            kSecAttrAccount as String: "snapshot-v1",
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let group = AppGroup.keychainGroup { query[kSecAttrAccessGroup as String] = group }
        var read = query
        read[kSecReturnData as String] = true
        read[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(read as CFDictionary, &item)
        if status == errSecSuccess, let data = item as? Data, data.count == 32 { return SymmetricKey(data: data) }
        guard create && status == errSecItemNotFound else { throw CocoaError(.fileReadNoPermission) }
        let key = SymmetricKey(size: .bits256)
        query[kSecValueData as String] = key.withUnsafeBytes { Data($0) }
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(query as CFDictionary, nil)
        if added == errSecDuplicateItem { return try self.key(create: false) }
        guard added == errSecSuccess else { throw CocoaError(.fileWriteNoPermission) }
        return key
    }
    static func seal(_ data: Data) throws -> Data {
        let box = try AES.GCM.seal(data, using: key(create: true), authenticating: marker)
        guard let combined = box.combined else { throw CocoaError(.fileWriteUnknown) }
        return marker + combined
    }
    static func open(_ data: Data) throws -> Data {
        guard data.starts(with: marker) else { throw CocoaError(.fileReadCorruptFile) }
        return try AES.GCM.open(AES.GCM.SealedBox(combined: data.dropFirst(marker.count)), using: key(create: false), authenticating: marker)
    }
}
