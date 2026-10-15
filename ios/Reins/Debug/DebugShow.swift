#if DEBUG
/// `-show <place>` at launch (screenshots, manual checks): a tab (`activity`, `trust`, `connections`, `settings`) or a
/// page in it: `sounds`, `devices`, `passkeys`, `appearance`, `vault`, `vaultAdd`, `vaultItem:<id>`, `connection:<id>`,
/// `gmail`, `service:<id>`, `mcp:<id>`, `grant:<id>`, `profile:<id>`, `tryit`, `ended`, `newgrant`.
@MainActor
enum DebugShow {
    static func open(_ value: String, in model: AppModel) {
        let (place, id) = value.split(separator: ":", maxSplits: 1).map(String.init).splitPair
        switch place {
        case "sounds": model.show(.sounds, in: .settings)
        case "devices": model.show(.devices, in: .settings)
        case "passkeys": model.show(.vaultPasskeys, in: .settings)
        case "appearance": model.show(.appearance, in: .settings)
        case "vault": model.show(.vault, in: .connections)
        case "vaultAdd": model.show(.vaultAdd, in: .connections)
        case "vaultItem": model.show(.vaultItem(id), in: .connections)
        case "connection": model.show(.connection(id), in: .connections)
        case "gmail": model.show(.gmail, in: .connections)
        case "service": model.show(.service(id), in: .connections)
        case "mcp": model.show(.mcpServer(id), in: .connections)
        case "grant": model.show(.grantDetail(id), in: .trust)
        case "profile": model.show(.autopilotProfile(id), in: .trust)
        case "tryit": model.show(.tryIt(nil), in: .trust)
        case "ended": model.show(.expiredGrants, in: .trust)
        case "newgrant": model.show(.newGrant(token: 1), in: .trust)
        default:
            if let section = AppSection(rawValue: place) { model.select(section) }
        }
    }
}

private extension Array where Element == String {
    var splitPair: (String, String) { (first ?? "", count > 1 ? self[1] : "") }
}
#endif
