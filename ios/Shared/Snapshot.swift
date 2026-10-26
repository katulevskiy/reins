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
        /// The icon the connection shows (the one picked, else what Auto shows: "claude", "macos", "blob", ...), nil
        /// when not known (the name then suggests one).
        var connectionIcon: String? = nil
        /// The blobatar the connection keeps, drawn when it shows its blobatar. Optional so older snapshots still read.
        var connectionBlobatar: BlobatarLook? = nil
        /// A routine request that can be approved without opening it (`PendingItem.quick`): the Live Activity offers
        /// Approve. Optional so snapshots written before it still read.
        var quick: Bool? = nil
        /// Its field (`FieldKind` raw value), picked by the app from the core's kind and action, never from text. Nil
        /// in snapshots written before Signal: `Glance.field` falls back on the item's kind.
        var field: String? = nil
        /// The label line from the core's provider and action ("GitHub · Push"); nil before Signal.
        var label: String? = nil
    }

    struct Entry: Codable, Equatable, Identifiable {
        var id: Int64
        var title: String
        /// "Allowed", "Denied", "Sent", ...
        var outcome: String
        var approved: Bool
        var at: Int64
        /// The integration ("gmail"), for the row's icon; optional so older snapshots still read.
        var service: String? = nil
        /// The core's action ("read", "push"), for the icon's badge.
        var action: String? = nil
    }

    var signedIn: Bool = false
    var accountFingerprint: String?
    var approvalDevice: Bool = false
    var pending: [Item] = []
    var latest: [Entry] = []
    /// "manual", "auto", "overnight", "bypass", "lockdown" (an older "assisted" reads as Manual).
    var autopilotMode: String = "manual"
    /// Unix seconds when a running bypass ends; `Int64.max` (`AutopilotText.untilOff`) while it runs until turned off.
    var bypassUntil: Int64?
    /// The global mode a bypass went back to when it ended ("manual", "auto", ...), so a bypass that ran out
    /// before the next refresh shows the right mode.
    var baseMode: String?
    /// When the last running bypass ends, the global one or any connection's (the Stop button shows while one runs);
    /// `Int64.max` when one runs until turned off.
    var anyBypassUntil: Int64?
    /// When a global Overnight ends (`Int64.max` while it runs until turned off); nil when none runs.
    var overnightUntil: Int64?
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
