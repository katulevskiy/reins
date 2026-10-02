import Foundation

/// `reins://` links from notifications, widgets, controls and Live Activities. Any app can open one, so nothing in a
/// link is trusted: an item opens only if the core really has it parked (see `AppModel.handle(_:)`).
enum DeepLink: Equatable {
    case item(kind: Snapshot.Item.Kind, id: String)
    case activity(id: Int64)
    case grant(id: String)
    case autopilot
    /// The Integrations page (services and MCP servers), over Activity.
    case integrations
    /// A computer's pairing code ("BCDF-GHJK"), to redeem and answer in the pairing sheet.
    case pair(code: String)
    case home

    static let scheme = "reins"

    var url: URL {
        var c = URLComponents()
        c.scheme = Self.scheme
        switch self {
        case let .item(kind, id):
            c.host = "item"
            c.queryItems = [URLQueryItem(name: "kind", value: kind.rawValue), URLQueryItem(name: "id", value: id)]
        case let .activity(id):
            c.host = "activity"
            c.queryItems = [URLQueryItem(name: "id", value: String(id))]
        case let .grant(id):
            c.host = "grant"
            c.queryItems = [URLQueryItem(name: "id", value: id)]
        case .autopilot:
            c.host = "autopilot"
        case .integrations:
            c.host = "integrations"
        case let .pair(code):
            c.host = "pair"
            c.queryItems = [URLQueryItem(name: "code", value: code)]
        case .home:
            c.host = "home"
        }
        return c.url!
    }

    static func isId(_ value: String) -> Bool {
        (1...128).contains(value.count) && value.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") }
    }

    init?(url: URL) {
        guard url.scheme == Self.scheme, let c = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
        let query = Dictionary((c.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { a, _ in a })
        switch c.host {
        case "item":
            guard let kind = query["kind"].flatMap(Snapshot.Item.Kind.init(rawValue:)), let id = query["id"], Self.isId(id) else { return nil }
            self = .item(kind: kind, id: id)
        case "activity":
            guard let id = query["id"].flatMap(Int64.init) else { return nil }
            self = .activity(id: id)
        case "grant":
            guard let id = query["id"], Self.isId(id) else { return nil }
            self = .grant(id: id)
        case "autopilot": self = .autopilot
        case "integrations": self = .integrations
        case "pair":
            guard let code = query["code"].flatMap(PairingCode.normalize) else { return nil }
            self = .pair(code: code)
        case "home": self = .home
        default: return nil
        }
    }

    /// A link the app was opened with: a `reins://` link, or a pairing link on a server's own address (a universal
    /// link or a scanned QR code), of which only the code is used.
    static func opened(_ url: URL) -> DeepLink? {
        if url.scheme == scheme { return DeepLink(url: url) }
        return PairingCode.parse(url.absoluteString).map { .pair(code: $0) }
    }
}
