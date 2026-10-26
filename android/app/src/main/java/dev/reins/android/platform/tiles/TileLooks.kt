package dev.reins.android.platform.tiles

import dev.reins.android.autopilot.AutopilotText
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings

/**
 * What a Quick Settings tile shows: lit ([active], STATE_ACTIVE) or not, its name and a subtitle (And-Outside 03:
 * "Lockdown · On", "Reins · 2 waiting", "Autopilot · Auto").
 */
data class TileLook(val active: Boolean, val label: String, val subtitle: String)

/**
 * What a tap on a tile does. A tile only ever tightens by itself (Lockdown on, Auto back to Manual); anything that
 * loosens (ending Lockdown, Manual to Auto) opens Reins, where the owner's check comes first. A tile never starts a
 * bypass or Overnight.
 */
enum class TileTap {
    /** Turns Lockdown on here: no check, it only takes power away. */
    LockDown,

    /** Back to Manual from Auto: also only takes power away. */
    SetManual,

    /** Opens Trust, where ending Lockdown or choosing a mode asks the owner first. */
    OpenTrust,

    /** Opens Reins on the stack of what waits. */
    OpenStack,

    /** Stops the bypass in force: it only takes power away. */
    StopBypass,

    /** Stops the Overnight in force, back to the mode it interrupted: it only takes power away. */
    StopOvernight,
}

/** The modes a tile may set by itself: never one that lets more happen. */
val TILE_MODES: Set<AutopilotMode> = setOf(AutopilotMode.LOCKDOWN, AutopilotMode.MANUAL)

fun lockdownLook(s: AutopilotSettings?): TileLook =
    if (s?.mode == AutopilotMode.LOCKDOWN) TileLook(true, "Lockdown", "On") else TileLook(false, "Lockdown", "Off")

fun lockdownTap(s: AutopilotSettings?): TileTap = if (s?.mode == AutopilotMode.LOCKDOWN) TileTap.OpenTrust else TileTap.LockDown

fun waitingLook(waiting: Int): TileLook = if (waiting == 0) TileLook(false, "Reins", "Nothing waits") else TileLook(true, "$waiting waiting", "Open Reins")

/** Bypass (Next, Outside 04): lit only while it runs, with its time left ("47:12 left", "On"). */
fun bypassLook(s: AutopilotSettings?, nowSeconds: Long): TileLook {
    val until = s?.bypassUntil?.takeIf { s.mode == AutopilotMode.BYPASS }
    return when {
        until == null -> TileLook(false, "Bypass", "Off")
        AutopilotText.isUntilOff(until) -> TileLook(true, "Bypass", "On")
        else -> TileLook(true, "Bypass", "${AutopilotText.clock(until, nowSeconds)} left")
    }
}

/** A tap on the Bypass tile: stops a running one; never starts one (that opens Trust and asks the owner). */
fun bypassTap(s: AutopilotSettings?): TileTap = if (s?.mode == AutopilotMode.BYPASS) TileTap.StopBypass else TileTap.OpenTrust

/** Overnight: lit only while it runs, with when it ends ("Until 7:00 AM"). */
fun overnightLook(s: AutopilotSettings?, zone: java.util.TimeZone = java.util.TimeZone.getDefault()): TileLook {
    val until = s?.overnightUntil?.takeIf { s.mode == AutopilotMode.OVERNIGHT }
    return when {
        until == null -> TileLook(false, "Overnight", "Off")
        AutopilotText.isUntilOff(until) -> TileLook(true, "Overnight", "On")
        else -> TileLook(true, "Overnight", "Until " + java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).apply { timeZone = zone }.format(java.util.Date(until * 1000)))
    }
}

fun overnightTap(s: AutopilotSettings?): TileTap = if (s?.mode == AutopilotMode.OVERNIGHT) TileTap.StopOvernight else TileTap.OpenTrust

fun autopilotLook(s: AutopilotSettings?): TileLook {
    val mode = s?.mode ?: AutopilotMode.MANUAL
    // Next: the tile is Auto, lit only while Auto is on.
    return TileLook(mode == AutopilotMode.AUTO, "Auto", if (mode == AutopilotMode.AUTO) "On" else "Off")
}

fun autopilotTap(s: AutopilotSettings?): TileTap = if (s?.mode == AutopilotMode.AUTO) TileTap.SetManual else TileTap.OpenTrust
