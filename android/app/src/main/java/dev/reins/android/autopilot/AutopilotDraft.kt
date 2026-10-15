package dev.reins.android.autopilot

import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot

/**
 * What Autopilot's settings become once a change the user just made is in, shown before the core confirms it. The
 * re-read after the core's answer replaces it with the real thing, so this only has to be right for what the screens
 * show at once: the global mode, a connection's mode, its profile, the default profile and Wi-Fi only.
 */
object AutopilotDraft {
    /** [mode] for everything ([connectionId] null) or one connection (null [mode]: it follows the global one again). */
    fun withMode(s: AutopilotSettings, connectionId: String?, mode: AutopilotMode?, minutes: UInt?, nowSecs: Long): AutopilotSettings =
        draft(s, connectionId, mode, minutes?.let { nowSecs + it.toLong() * 60 }, nowSecs)

    /** A bypass of everything or of [connectionId] for [minutes], or until turned off (null). */
    fun withBypass(s: AutopilotSettings, connectionId: String?, minutes: UInt?, nowSecs: Long): AutopilotSettings =
        draft(s, connectionId, AutopilotMode.BYPASS, minutes?.let { nowSecs + it.toLong() * 60 } ?: AutopilotText.UNTIL_OFF, nowSecs)

    /** Overnight for everything or for [connectionId], for [minutes] or until turned off (null). */
    fun withOvernight(s: AutopilotSettings, connectionId: String?, minutes: UInt?, nowSecs: Long): AutopilotSettings =
        draft(s, connectionId, AutopilotMode.OVERNIGHT, minutes?.let { nowSecs + it.toLong() * 60 } ?: AutopilotText.UNTIL_OFF, nowSecs)

    private fun draft(s: AutopilotSettings, connectionId: String?, mode: AutopilotMode?, until: Long?, nowSecs: Long): AutopilotSettings {
        val since = if (AutopilotText.isTimed(mode)) nowSecs else null
        if (connectionId == null) {
            // Manual is the mode before the user picks one (`modes::global_base`); a bypass also ends a lockdown.
            val global = when (mode) {
                AutopilotMode.BYPASS -> {
                    val base = if (s.baseMode == AutopilotMode.LOCKDOWN) AutopilotMode.MANUAL else s.baseMode
                    s.copy(mode = AutopilotMode.BYPASS, baseMode = base, bypassUntil = until, bypassSince = since)
                }
                // Overnight is the global mode itself (it ends a lockdown); a running bypass still comes first.
                AutopilotMode.OVERNIGHT -> s.copy(
                    mode = if (s.bypassUntil != null) AutopilotMode.BYPASS else AutopilotMode.OVERNIGHT,
                    baseMode = AutopilotMode.OVERNIGHT,
                    overnightUntil = until,
                    overnightSince = since,
                )
                else -> {
                    val base = mode ?: AutopilotMode.MANUAL
                    s.copy(mode = base, baseMode = base, bypassUntil = null, bypassSince = null, overnightUntil = null, overnightSince = null)
                }
            }
            // Connections without a mode or a bypass of their own follow the global one (and a lockdown).
            return global.copy(connections = global.connections.map { c -> c.copy(mode = effective(global, c)) })
        }
        return withConnection(s, connectionId) { c ->
            when (mode) {
                AutopilotMode.BYPASS -> c.copy(baseMode = c.baseMode.takeIf { it != AutopilotMode.LOCKDOWN }, bypassUntil = until, bypassSince = since)
                AutopilotMode.OVERNIGHT -> c.copy(baseMode = AutopilotMode.OVERNIGHT, overnightUntil = until, overnightSince = since)
                else -> c.copy(baseMode = mode, bypassUntil = null, bypassSince = null, overnightUntil = null, overnightSince = null)
            }.let { it.copy(mode = effective(s, it)) }
        }
    }

    /** Ends a bypass: the global one goes back to its base mode, a connection's to its own setting. */
    fun withoutBypass(s: AutopilotSettings, connectionId: String?): AutopilotSettings =
        if (connectionId == null) withMode(s, null, s.baseMode, null, 0) else withMode(s, connectionId, s.connections.firstOrNull { it.connectionId == connectionId }?.baseMode, null, 0)

    /**
     * The mode an Overnight about to start in [connectionId]'s scope interrupts, which Stop returns to (the core keeps
     * its own for when it ends by itself, but does not tell it). [Interrupted.Keep] when an Overnight already runs there
     * (starting again keeps what the first one interrupted); a lockdown it ends is not returned to, as in the core.
     */
    fun interrupted(s: AutopilotSettings, connectionId: String?): Interrupted {
        val base = if (connectionId == null) s.baseMode else s.connections.firstOrNull { it.connectionId == connectionId }?.baseMode
        return when (base) {
            AutopilotMode.OVERNIGHT -> Interrupted.Keep
            AutopilotMode.LOCKDOWN -> Interrupted.Mode(null)
            else -> Interrupted.Mode(base)
        }
    }

    sealed interface Interrupted {
        data object Keep : Interrupted

        /** null: the default (global), or following the global mode (a connection). */
        data class Mode(val mode: AutopilotMode?) : Interrupted
    }

    /** Which profile [connectionId] trains (null: the default one). */
    fun withProfile(s: AutopilotSettings, connectionId: String, profileId: String?): AutopilotSettings =
        withConnection(s, connectionId) { it.copy(profileId = profileId ?: s.defaultProfileId) }

    fun withDefaultProfile(s: AutopilotSettings, profileId: String): AutopilotSettings = s.copy(
        defaultProfileId = profileId,
        connections = s.connections.map { if (it.profileId == s.defaultProfileId) it.copy(profileId = profileId) else it },
    )

    private fun withConnection(s: AutopilotSettings, connectionId: String, edit: (ConnectionAutopilot) -> ConnectionAutopilot): AutopilotSettings {
        val existing = s.connections.firstOrNull { it.connectionId == connectionId }
        val connections = if (existing != null) {
            s.connections.map { if (it.connectionId == connectionId) edit(it) else it }
        } else {
            s.connections + edit(
                ConnectionAutopilot(
                    connectionId = connectionId,
                    baseMode = null,
                    bypassUntil = null,
                    bypassSince = null,
                    mode = s.mode,
                    profileId = s.defaultProfileId,
                ),
            )
        }
        return s.copy(connections = connections)
    }

    /**
     * What applies to a connection's requests, in the core's order (`modes::effective`): lockdowns, its bypass, a
     * global bypass, its own Overnight, a global one (unless it is kept on Manual), its own mode.
     */
    private fun effective(s: AutopilotSettings, c: ConnectionAutopilot): AutopilotMode = when {
        s.mode == AutopilotMode.LOCKDOWN || c.baseMode == AutopilotMode.LOCKDOWN -> AutopilotMode.LOCKDOWN
        c.bypassUntil != null || s.bypassUntil != null -> AutopilotMode.BYPASS
        c.overnightUntil != null -> AutopilotMode.OVERNIGHT
        s.overnightUntil != null && c.baseMode != AutopilotMode.MANUAL -> AutopilotMode.OVERNIGHT
        else -> c.baseMode ?: s.mode
    }
}
