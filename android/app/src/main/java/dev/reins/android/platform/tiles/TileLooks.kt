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
}

/** The modes a tile may set by itself: never one that lets more happen. */
val TILE_MODES: Set<AutopilotMode> = setOf(AutopilotMode.LOCKDOWN, AutopilotMode.MANUAL)

fun lockdownLook(s: AutopilotSettings?): TileLook =
    if (s?.mode == AutopilotMode.LOCKDOWN) TileLook(true, "Lockdown", "On") else TileLook(false, "Lockdown", "Off")

fun lockdownTap(s: AutopilotSettings?): TileTap = if (s?.mode == AutopilotMode.LOCKDOWN) TileTap.OpenTrust else TileTap.LockDown

fun waitingLook(waiting: Int): TileLook = TileLook(waiting > 0, "Reins", if (waiting == 0) "Nothing waits" else "$waiting waiting")

fun autopilotLook(s: AutopilotSettings?): TileLook {
    val mode = s?.mode ?: AutopilotMode.MANUAL
    return TileLook(mode != AutopilotMode.MANUAL, "Autopilot", AutopilotText.name(mode))
}

fun autopilotTap(s: AutopilotSettings?): TileTap = if (s?.mode == AutopilotMode.AUTO) TileTap.SetManual else TileTap.OpenTrust
