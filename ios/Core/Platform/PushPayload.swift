import Foundation

/// A validated push from the server (contracts §A, the Android app's `PushPayload`): `t` is `req`, `pair`, `blob` or
/// `replaced`, `id` a server-issued id. Push is only a hint that something waits; the phone fetches the real data
/// itself, so anything that is not exactly this shape is dropped.
struct PushPayload: Equatable {
    var kind: String
    var id: String

    /// An org review waits for this member (O2): the app opens the work profile's reviews; the core fetches nothing.
    static let orgReview = "orgreview"

    static let kinds: Set<String> = ["req", "pair", "blob", "replaced", orgReview]

    init?(kind: String?, id: String?) {
        guard let kind, let id, Self.kinds.contains(kind) else { return nil }
        // "replaced" names no item (the server sends an empty id); every other kind needs a well-formed one.
        if kind == "replaced" {
            guard id.isEmpty || DeepLink.isId(id) else { return nil }
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

    /// Where tapping the notification goes.
    var link: DeepLink? {
        if kind == Self.orgReview { return .orgReviews }
        return itemKind.map { .item(kind: $0, id: id) }
    }
}
