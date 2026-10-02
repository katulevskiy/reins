import Foundation

/// A validated push from the server (contracts §A, the Android app's `PushPayload`): `t` is `req`, `pair`, `blob` or
/// `replaced`, `id` a server-issued id. Push is only a hint that something waits; the phone fetches the real data
/// itself, so anything that is not exactly this shape is dropped.
struct PushPayload: Equatable {
    var kind: String
    var id: String

    static let kinds: Set<String> = ["req", "pair", "blob", "replaced"]

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
    var link: DeepLink? { itemKind.map { .item(kind: $0, id: id) } }
}
