#if DEBUG
/// `-show <place>` at launch (screenshots, manual checks): a tab (`activity`, `trust`, `connections`, `settings`) or a
/// page in it: `sounds`, `devices`, `passkeys`, `appearance`, `wallpaper`, `ownphoto[:light]`, `vault`, `vaultAdd`, `vaultItem:<id>`, `vaultNew:<kind>`, `connection:<id>`,
/// `gmail`, `service:<id>`, `mcp:<id>`, `connectSheet`, `addService`, `typeCode`, `join:<id>`, `grant:<id>`, `profile:<id>`, `tryit`, `ended`, `permissions`, `accountPermissions:<service>/<account>[/temp]`, `newgrant`, `entry:<id>` (an Activity entry), and `tour` (its page
/// from `-tourPage <n>`, 0 to 4).
@MainActor
enum DebugShow {
    static func open(_ value: String, in model: AppModel) {
        let (place, id) = value.split(separator: ":", maxSplits: 1).map(String.init).splitPair
        switch place {
        case "sounds": model.show(.sounds, in: .settings)
        case "devices": model.show(.devices, in: .settings)
        case "passkeys": model.show(.vaultPasskeys, in: .settings)
        case "appearance": model.show(.appearance, in: .settings)
        case "wallpaper": model.show(.wallpaper, in: .settings)
        case "ownphoto": model.show(.ownWallpaper(dark: id != "light"), in: .settings)
        case "vault": model.show(.vault, in: .connections)
        case "vaultAdd": model.show(.vaultAdd, in: .connections)
        case "vaultItem": model.show(.vaultItem(id), in: .connections)
        case "vaultNew": model.show(.vaultEdit(id: nil, newItem: NewVaultItem(rawValue: id) ?? .login), in: .connections)
        case "connection": model.show(.connection(id), in: .connections)
        case "gmail": model.show(.gmail, in: .connections)
        case "service": model.show(.service(id), in: .connections)
        case "mcp": model.show(.mcpServer(id), in: .connections)
        case "grant": model.show(.grantDetail(id), in: .trust)
        case "profile": model.show(.autopilotProfile(id), in: .trust)
        case "tryit": model.show(.tryIt(nil), in: .trust)
        case "ended": model.show(.expiredGrants, in: .trust)
        case "permissions": model.show(.permissions, in: .trust)
        case "accountPermissions":
            // `accountPermissions:gmail/me@gmail.com`, or `.../temp` for a temporary one.
            let parts = id.split(separator: "/").map(String.init)
            model.show(.accountPermissions(service: parts.first ?? "gmail", account: parts.count > 1 ? parts[1] : "", temporary: parts.count > 2), in: .trust)
        case "newgrant": model.show(.newGrant(token: 1), in: .trust)
        case "tour": model.startTour()
        case "typeCode":
            model.select(.connections)
            model.connectStartsTyped = true
            model.openSheet(.connectComputer)
        case "join":
            model.select(.connections)
            model.openSheet(.join(id))
        case "connectSheet", "addService":
            // Connections opens its own sheet when it sees the place (ConnectionsScreen).
            model.select(.connections)
        case "entry": model.show(.activityDetail(Int64(id) ?? 0), in: .activity)
        case "email":
            // `email:<entry>`: the entry's first email, opened.
            model.show(.activityDetail(Int64(id) ?? 0), in: .activity)
            model.push(.email(entryId: Int64(id) ?? 0, index: 0))
        default:
            if let section = AppSection(rawValue: place) { model.select(section) }
        }
    }
}

private extension Array where Element == String {
    var splitPair: (String, String) { (first ?? "", count > 1 ? self[1] : "") }
}
#endif
