package dev.reins.android.ui.nav

/**
 * The root tabs, as on iOS (Reins Signal): what happened, what runs without asking (Autopilot and grants), both ends
 * of the wire, and you and this phone.
 */
enum class Tab(val label: String, val tag: String) {
    Activity("Activity", "tabActivity"),
    Trust("Trust", "tabTrust"),
    Connections("Connections", "tabConnections"),
    Settings("Settings", "tabSettings"),
}

/** Screens pushed on top of the tabs. */
sealed interface Route {
    /** Settings > Sounds and vibration. */
    data object Sounds : Route

    /** Settings > Appearance: paper or graphite, the grain on fields, the sans. */
    data object Appearance : Route

    /** Settings > Account > Vault passkeys: the passkeys that open the vault on a new phone. */
    data object VaultPasskeys : Route

    /** Settings > Account > Devices: the account's phones (sign out a lost one) and computers. */
    data object Devices : Route

    /** Trust > Ended: the grants that ran out, to resume or delete. */
    data object EndedGrants : Route

    /** One Autopilot profile. */
    data class AutopilotProfile(val id: String) : Route

    /** Autopilot's "Try it", for a profile (null = the default one). */
    data class TryIt(val profileId: String?) : Route

    data class Connection(val id: String) : Route

    /** Connections > Connect something > Computer: scan (or type) the code the desktop app or `reins login` shows. */
    data object ConnectComputer : Route

    /** The Gmail accounts. */
    data object Gmail : Route
    data object Permissions : Route
    data class AccountPermissions(val service: String, val account: String, val temporary: Boolean = false) : Route

    /** The accounts of one other integration (its id). */
    data class Service(val id: String) : Route

    /** Integrations > Password vault > Open the vault: the items and a search. */
    data object Vault : Route

    /** Vault > Add: what kind of item. */
    data object VaultAdd : Route

    /** One vault item. */
    data class VaultItem(val id: String) : Route

    /** A new vault item of a kind ([id] null), or the item [id] changed. */
    data class VaultEdit(val id: String?, val newItem: dev.reins.android.ui.vault.NewItem?) : Route

    /** Integrations > Payments: payment methods, the virtual card provider, addresses, spend limits, budgets. */
    data object Payments : Route

    /** Payments > Spending: every purchase, what was spent this month. */
    data object Spending : Route

    /** Adding an MCP server by its address. */
    data object McpAdd : Route

    /** One added MCP server: its tools, sign-in, removal. */
    data class McpServer(val id: String) : Route

    data class ActivityDetail(val id: Long) : Route

    /** One email listed in an activity entry (the entry id and the email's place in its list). */
    data class Email(val entryId: Long, val index: Int) : Route

    data class GrantDetail(val id: String) : Route

    data object NewGrant : Route

    /** Debug builds: Settings > Developer > Signal gallery, every Signal component. */
    data object SignalGallery : Route
}

/** The near-full-screen sheet for an item that waits for the user. */
sealed interface SheetTarget {
    val id: String

    data class Approval(override val id: String) : SheetTarget

    data class Pairing(override val id: String) : SheetTarget

    /** A file an AI uploaded, waiting for the user's decision. */
    data class Upload(override val id: String) : SheetTarget

    /** Another phone of the account asks for its keys ("Add another phone"). */
    data class Join(override val id: String) : SheetTarget
}
