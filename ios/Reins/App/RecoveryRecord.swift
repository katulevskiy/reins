import CryptoKit
import Foundation

/// Acknowledgement only: the recovery secret stays in the core's encrypted store. Its fingerprint is stable through
/// WorkOS email changes and different for a replaced recovery code or another server.
enum RecoveryRecord {
    static func matchesLastGroup(code: String, entered: String) -> Bool {
        entered.trimmingCharacters(in: .whitespacesAndNewlines).uppercased() == code.split(separator: "-").last.map(String.init)
    }

    static func key(server: String, code: String) -> String {
        let server = server.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let digest = SHA256.hash(data: Data("\(server)|\(code)".utf8))
        return "recovery-recorded:" + digest.map { String(format: "%02x", $0) }.joined()
    }

    static func confirmed(server: String, code: String, defaults: UserDefaults = AppGroup.defaults) -> Bool {
        defaults.bool(forKey: key(server: server, code: code))
    }

    static func confirm(server: String, code: String, defaults: UserDefaults = AppGroup.defaults) {
        defaults.set(true, forKey: key(server: server, code: code))
    }
}
