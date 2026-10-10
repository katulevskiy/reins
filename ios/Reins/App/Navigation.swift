import Foundation

/// The app's top-level sections: tabs on a compact width, the sidebar on a regular one (iPad, an unfolded iPhone
/// Duo).
enum AppSection: String, CaseIterable, Hashable, Identifiable {
    case activity, grants, autopilot, settings

    var id: String { rawValue }

    var title: String {
        switch self {
        case .activity: "Activity"
        case .grants: "Grants"
        case .autopilot: "Autopilot"
        case .settings: "Settings"
        }
    }

    var symbol: String {
        switch self {
        case .activity: "waveform.path.ecg"
        case .grants: "key.horizontal"
        case .autopilot: "bolt.shield"
        case .settings: "gearshape"
        }
    }
}

/// Screens shown on top of a section's root (pushed on a compact width, in the detail column on a regular one).
enum Route: Hashable {
    /// Settings > Sounds & haptics.
    case sounds
    /// Settings > Vault passkeys.
    case vaultPasskeys
    /// Settings > Devices: the account's phones (sign out a lost one) and computers.
    case devices
    /// Integrations > Password vault > Open the vault.
    case vault
    /// Vault > Add: what kind of item.
    case vaultAdd
    /// One vault item.
    case vaultItem(String)
    /// A new vault item of a kind (`id` nil), or the item `id` changed.
    case vaultEdit(id: String?, newItem: NewVaultItem?)
    /// One Autopilot profile.
    case autopilotProfile(String)
    /// Autopilot's "Try it", for a profile (nil = the default one).
    case tryIt(String?)
    /// One AI connection.
    case connection(String)
    /// The services that can be connected (Gmail, GitHub, ...).
    case integrations
    /// The Gmail accounts.
    case gmail
    /// The accounts of one other integration (its id).
    case service(String)
    /// Adding an MCP server by its address.
    case mcpAdd
    /// One added MCP server: its tools, sign-in, removal.
    case mcpServer(String)
    case activityDetail(Int64)
    /// One email listed in an activity entry (the entry id and the email's place in its list).
    case email(entryId: Int64, index: Int)
    case grantDetail(String)
    case newGrant(token: Int)
    /// Autopilot settings reached from Activity's mode pill or Settings (the Autopilot section's root, pushed).
    case autopilot
}

/// The near-full-screen sheet for an item that waits for the user.
enum SheetTarget: Hashable, Identifiable {
    case approval(String)
    case pairing(String)
    case upload(String)
    /// Another phone asks for the account's keys.
    case join(String)
    /// Scanning (or typing) the code a computer shows; the pairing it stands for then takes the sheet's place.
    case connectComputer

    var id: String {
        switch self {
        case let .approval(id), let .pairing(id), let .upload(id), let .join(id): id
        case .connectComputer: "connectComputer"
        }
    }
}

extension PendingItem {
    var sheetTarget: SheetTarget {
        switch kind {
        case .request: .approval(id)
        case .pairing: .pairing(id)
        case .blob: .upload(id)
        case .join: .join(id)
        }
    }

    var snapshotKind: Snapshot.Item.Kind {
        switch kind {
        case .request: .request
        case .pairing: .pairing
        case .blob: .blob
        case .join: .join
        }
    }
}
