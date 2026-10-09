package dev.reins.android.ui.nav

/** The two root tabs. */
enum class Tab { Activity, Grants }

/** Screens pushed on top of the tabs. */
sealed interface Route {
    data object Settings : Route

    /** Settings > Sounds & haptics. */
    data object Sounds : Route

    /** Settings > Account > Vault passkeys: the passkeys that open the vault on a new phone. */
    data object VaultPasskeys : Route

    /** Settings > Autopilot (also the header's mode pill). */
    data object Autopilot : Route

    /** One Autopilot profile. */
    data class AutopilotProfile(val id: String) : Route

    /** Autopilot's "Try it", for a profile (null = the default one). */
    data class TryIt(val profileId: String?) : Route

    data class Connection(val id: String) : Route

    /** Settings > AI connections > Connect a computer: scan (or type) the code the desktop app or `reins login` shows. */
    data object ConnectComputer : Route

    /** The services that can be connected (Gmail, ...). */
    data object Integrations : Route

    /** The Gmail accounts. */
    data object Gmail : Route

    /** The accounts of one other integration (its id). */
    data class Service(val id: String) : Route

    /** Adding an MCP server by its address. */
    data object McpAdd : Route

    /** One added MCP server: its tools, sign-in, removal. */
    data class McpServer(val id: String) : Route

    data class ActivityDetail(val id: Long) : Route

    /** One email listed in an activity entry (the entry id and the email's place in its list). */
    data class Email(val entryId: Long, val index: Int) : Route

    data class GrantDetail(val id: String) : Route

    data object NewGrant : Route
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
