package dev.reins.android.platform.tiles

import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dev.reins.android.MainActivity
import dev.reins.android.ReinsApp
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AppNotifier
import dev.reins.core.AutopilotMode
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The three Quick Settings tiles (And-Outside 03), the Android counterpart of the iPhone's Control Center controls.
 * Each shows its [TileLook] and does its [TileTap]: Lockdown turns on right here; anything that loosens opens Reins.
 * Long-press goes through [TilePreferencesActivity].
 */
abstract class ReinsTile : TileService() {
    private val app get() = application as ReinsApp

    protected val container get() = app.container

    protected abstract fun look(): TileLook

    protected abstract fun tap(): TileTap

    override fun onStartListening() {
        render()
        // Opened from the shade with Reins not running: read the state once, then show it.
        container.appScope.launch {
            runCatching {
                container.refreshAutopilot()
                container.refreshPending()
            }
            withContext(Dispatchers.Main) { render() }
        }
    }

    override fun onClick() {
        when (tap()) {
            TileTap.LockDown -> setMode(AutopilotMode.LOCKDOWN)
            TileTap.SetManual -> setMode(AutopilotMode.MANUAL)
            TileTap.OpenTrust -> open(Intent(this, MainActivity::class.java).setAction(AppNotifier.ACTION_OPEN_AUTOPILOT))
            TileTap.OpenStack -> open(Intent(this, MainActivity::class.java))
            TileTap.StopBypass -> stop { container.stopBypasses() }
            TileTap.StopOvernight -> stop { container.stopOvernight(null) }
        }
    }

    /** Ends a running mode right here (it only takes power away), then redraws. */
    private fun stop(block: suspend () -> Unit) {
        container.appScope.launch {
            runCatching {
                block()
                container.refreshAutopilot()
            }
            withContext(Dispatchers.Main) {
                render()
                Tiles.refresh(this@ReinsTile)
            }
        }
    }

    /** Only ever a mode that takes power away ([TILE_MODES]); the heavy thud for Lockdown. */
    private fun setMode(mode: AutopilotMode) {
        require(mode in TILE_MODES)
        if (mode == AutopilotMode.LOCKDOWN) container.feedback.play(Event.LockdownOn)
        container.appScope.launch {
            runCatching {
                container.core.setAutopilotMode(null, mode, null)
                container.refreshAutopilot()
            }
            withContext(Dispatchers.Main) {
                render()
                Tiles.refresh(this@ReinsTile)
            }
        }
    }

    private fun open(intent: Intent) {
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val go = {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                startActivityAndCollapse(PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            } else {
                @Suppress("DEPRECATION")
                startActivityAndCollapse(intent)
            }
        }
        // Not bound to the shade (it went away meanwhile): open Reins plainly.
        runCatching { unlockAndRun { runCatching(go).onFailure { startActivity(intent) } } }.onFailure { startActivity(intent) }
    }

    protected fun render() {
        val tile = qsTile ?: return
        val look = look()
        tile.state = if (look.active) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = look.label
        tile.subtitle = look.subtitle
        tile.stateDescription = look.subtitle
        tile.updateTile()
    }
}

/** Lockdown: lit only while on. A tap turns it on (the heavy thud); while on, a tap opens Trust to end it. */
class LockdownTileService : ReinsTile() {
    override fun look() = lockdownLook(container.state.autopilot.value)

    override fun tap() = lockdownTap(container.state.autopilot.value)
}

/** What waits: lit while something does; a tap opens the stack. */
class WaitingTileService : ReinsTile() {
    override fun look() = waitingLook(container.state.pending.value.size)

    override fun tap() = TileTap.OpenStack
}

/** Autopilot's mode: lit unless Manual. From Auto a tap goes back to Manual; otherwise it opens Trust. */
class AutopilotTileService : ReinsTile() {
    override fun look() = autopilotLook(container.state.autopilot.value)

    override fun tap() = autopilotTap(container.state.autopilot.value)
}

/** Bypass: lit with its time left while it runs; a tap stops it. Off, a tap opens Trust (starting asks the owner). */
class BypassTileService : ReinsTile() {
    override fun look() = bypassLook(container.state.autopilot.value, System.currentTimeMillis() / 1000)

    override fun tap() = bypassTap(container.state.autopilot.value)
}

/** Overnight: lit with when it ends while it runs; a tap stops it. Off, a tap opens Trust. */
class OvernightTileService : ReinsTile() {
    override fun look() = overnightLook(container.state.autopilot.value)

    override fun tap() = overnightTap(container.state.autopilot.value)
}

object Tiles {
    private val services = listOf(
        LockdownTileService::class.java,
        WaitingTileService::class.java,
        AutopilotTileService::class.java,
        BypassTileService::class.java,
        OvernightTileService::class.java,
    )

    /** Asks the system to redraw the tiles (what waits or the mode changed). */
    fun refresh(context: Context) {
        services.forEach { runCatching { TileService.requestListeningState(context, ComponentName(context, it)) } }
    }
}
