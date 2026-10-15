package dev.reins.android.platform.tiles

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.os.Bundle
import dev.reins.android.MainActivity
import dev.reins.android.platform.AppNotifier

/**
 * Long-press on a Reins tile (ACTION_QS_TILE_PREFERENCES): Lockdown and Autopilot open Trust, what waits opens the
 * stack. It only forwards to [MainActivity] and finishes.
 */
class TilePreferencesActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        startActivity(target(intent?.let { tileOf(it) }).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP))
        finish()
    }

    @Suppress("DEPRECATION")
    private fun tileOf(intent: Intent): ComponentName? = intent.getParcelableExtra(Intent.EXTRA_COMPONENT_NAME)

    private fun target(tile: ComponentName?): Intent = Intent(this, MainActivity::class.java).apply {
        if (tile?.className != WaitingTileService::class.java.name) action = AppNotifier.ACTION_OPEN_AUTOPILOT
    }
}
