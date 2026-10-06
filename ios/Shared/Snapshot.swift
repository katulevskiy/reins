import Foundation
import CryptoKit

/// What widgets, controls and Live Activities show, written by the app (and the notification extension) after every
/// refresh and read by the widget extension, which never runs the core. Titles are the same content-safe one-liners
/// the notifications show; the complete snapshot is encrypted in preferences.
struct Snapshot: Codable, Equatable {
    struct Item: Codable, Equatable, Identifiable {
        /// `join`: another phone asks for the account's keys.
        enum Kind: String, Codable { case request, pairing, blob, join }

        var id: String
        var kind: Kind
        /// "Claude: Send email to 2", "Connect an AI", "File to check".
        var title: String
        /// The account or target, may be empty.
        var subtitle: String
        /// The AI connection's label.
        var connection: String
        /// Integration id ("gmail", "github", ...), empty for pairings.
        var service: String
        /// Unix seconds.
        var createdAt: Int64
        /// Unix seconds when the server stops waiting for an answer.
        var expiresAt: Int64
        /// Autopilot's suggestion line, when it made one.
        var suggestion: String?
        /// The provider logo picked for the connection (`ConnectionView.icon`: "claude", "blob", ...), nil when none
        /// was picked (the name then suggests one).
        var connectionIcon: String? = nil
    }

    struct Entry: Codable, Equatable, Identifiable {
        var id: Int64
        var title: String
        /// "Allowed", "Denied", "Sent", ...
        var outcome: String
        var approved: Bool
        var at: Int64
    }

    var signedIn: Bool = false
    var accountFingerprint: String?
    var approvalDevice: Bool = false
    var pending: [Item] = []
    var latest: [Entry] = []
    /// "manual", "assisted", "auto", "bypass", "lockdown".
    var autopilotMode: String = "manual"
    /// Unix seconds when a running bypass ends.
    var bypassUntil: Int64?
    /// The global mode a bypass went back to when it ended ("manual", "assisted", ...), so a bypass that ran out
    /// before the next refresh shows the right mode.
    var baseMode: String?
    /// When the last running bypass ends, the global one or any connection's (the Stop button shows while one runs).
    var anyBypassUntil: Int64?
    var activeGrants: Int = 0
    var updatedAt: Int64 = 0

    static let key = "widget.snapshot.v1"

    static func owner(server: String, email: String) -> String {
        SHA256.hash(data: Data((server + "\u{0}" + email).utf8)).map { String(format: "%02x", $0) }.joined()
    }

    static func load(from defaults: UserDefaults = AppGroup.defaults) -> Snapshot {
        guard let data = defaults.data(forKey: key), let plain = try? SealedSnapshot.open(data), let snapshot = try? JSONDecoder().decode(Snapshot.self, from: plain) else {
            return Snapshot()
        }
        return snapshot
    }

    func save(to defaults: UserDefaults = AppGroup.defaults) {
        guard let data = try? JSONEncoder().encode(self) else { return }
        guard let sealed = try? SealedSnapshot.seal(data) else { defaults.removeObject(forKey: Self.key); return }
        defaults.set(sealed, forKey: Self.key)
    }

    /// Items whose answer window has not closed yet.
    func waiting(now: Int64 = Int64(Date().timeIntervalSince1970)) -> [Item] {
        pending.filter { $0.expiresAt > now }
    }
}
