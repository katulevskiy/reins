import Foundation

/// A validated push from the server (contracts §A, the Android app's `PushPayload`): `t` is `req`, `pair`, `blob`,
/// `replaced` or `circle`, `id` a server-issued id (a Circle push's id says why it came: `review:`, `alert:` or `news:`
/// before the id). Push is only a hint that something waits; the phone fetches the real data
/// itself, so anything that is not exactly this shape is dropped.
struct PushPayload: Equatable {
    var kind: String
    var id: String

    static let kinds: Set<String> = ["req", "pair", "blob", "replaced", "circle"]

    init?(kind: String?, id: String?) {
        guard let kind, let id, Self.kinds.contains(kind) else { return nil }
        // "replaced" names no item (the server sends an empty id); every other kind needs a well-formed one.
        if kind == "replaced" {
            guard id.isEmpty || DeepLink.isId(id) else { return nil }
        } else if kind == "circle" {
            guard CirclePush(id) != nil else { return nil }
        } else {
            guard DeepLink.isId(id) else { return nil }
        }
        self.kind = kind
        self.id = id
    }

    init?(userInfo: [AnyHashable: Any]) {
        self.init(kind: userInfo["t"] as? String, id: userInfo["id"] as? String)
    }

    /// The tag of the account a push is for (`p`, 16 hex digits; `reins_proto::pairing::push_profile_tag`): which
    /// profile on this phone it belongs to. nil when absent or malformed.
    static func tag(_ userInfo: [AnyHashable: Any]) -> String? {
        guard let p = userInfo["p"] as? String, p.count == 16, p.allSatisfy({ ("0"..."9").contains($0) || ("a"..."f").contains($0) }) else {
            return nil
        }
        return p
    }

    /// The kind of item a `req` / `pair` / `blob` push announces; nil for `replaced`.
    var itemKind: Snapshot.Item.Kind? {
        switch kind {
        case "req": .request
        case "pair": .pairing
        case "blob": .blob
        case "join": .join
        default: nil
        }
    }

    /// The push kind for an item kind (what `handlePush` expects).
    static func kind(of item: Snapshot.Item.Kind) -> String {
        switch item {
        case .request: "req"
        case .pairing: "pair"
        case .blob: "blob"
        case .join: "join"
        }
    }

    /// A Circle push: why it came, and the id it names.
    var circle: CirclePush? { kind == "circle" ? CirclePush(id) : nil }

    /// Where tapping the notification goes: a Circle's to Activity, where "For others" is.
    var link: DeepLink? {
        if kind == "circle" { return .home }
        return itemKind.map { .item(kind: $0, id: id) }
    }
}

/// A Circle push's id (`reins_server::circles::push_id`): `review:<packet>` (a request of another account waits for
/// this phone: the one Circle notification), `alert:<link>` (a guardianship ended: told at once) or `news:<id>` (anything
/// else, fetched quietly; it comes as a background push).
struct CirclePush: Equatable {
    enum Wake: String {
        case review, alert, news
    }

    var wake: Wake
    var id: String

    init?(_ raw: String) {
        let parts = raw.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2, let wake = Wake(rawValue: String(parts[0])), DeepLink.isId(String(parts[1])) else { return nil }
        self.wake = wake
        id = String(parts[1])
    }

    /// The server sent it as an alert (a review or a guardianship ending); news comes silently.
    var alerts: Bool { wake != .news }
}
