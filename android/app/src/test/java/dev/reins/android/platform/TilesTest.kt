package dev.reins.android.platform

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.FakeCore
import dev.reins.android.FlowHarness
import dev.reins.android.platform.tiles.AutopilotTileService
import dev.reins.android.platform.tiles.LockdownTileService
import dev.reins.android.platform.tiles.TILE_MODES
import dev.reins.android.platform.tiles.TileTap
import dev.reins.android.platform.tiles.WaitingTileService
import dev.reins.android.platform.tiles.autopilotLook
import dev.reins.android.platform.tiles.autopilotTap
import dev.reins.android.platform.tiles.lockdownLook
import dev.reins.android.platform.tiles.lockdownTap
import dev.reins.android.platform.tiles.waitingLook
import dev.reins.core.AutopilotMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The Quick Settings tiles (And-Outside 03): what they show, and that a tile only ever takes power away. Lockdown turns
 * on from the tile; ending it, or Manual to Auto, opens Reins (where the owner is asked); never a bypass or Overnight.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class TilesTest : FlowHarness() {
    private fun settings(mode: AutopilotMode) = kotlinx.coroutines.runBlocking { FakeCore().apply { apGlobal = FakeCore.ApRow(mode = mode) }.autopilotSettings() }

    @Test
    fun `the looks say what is on`() {
        assertEquals(true to "On", lockdownLook(settings(AutopilotMode.LOCKDOWN)).let { it.active to it.subtitle })
        assertEquals(false to "Off", lockdownLook(settings(AutopilotMode.MANUAL)).let { it.active to it.subtitle })
        assertEquals("2 waiting", waitingLook(2).subtitle)
        assertTrue(waitingLook(2).active)
        assertEquals("Nothing waits", waitingLook(0).subtitle)
        assertEquals("Auto", autopilotLook(settings(AutopilotMode.AUTO)).subtitle)
        assertFalse(autopilotLook(settings(AutopilotMode.MANUAL)).active)
    }

    @Test
    fun `a tile only tightens by itself and never starts a bypass or Overnight`() {
        assertEquals(setOf(AutopilotMode.LOCKDOWN, AutopilotMode.MANUAL), TILE_MODES)
        assertEquals(TileTap.LockDown, lockdownTap(settings(AutopilotMode.AUTO)))
        // Ending Lockdown loosens: it opens Trust, where the owner is asked.
        assertEquals(TileTap.OpenTrust, lockdownTap(settings(AutopilotMode.LOCKDOWN)))
        assertEquals(TileTap.SetManual, autopilotTap(settings(AutopilotMode.AUTO)))
        // Manual to Auto loosens; a bypass or Overnight is never touched from a tile.
        for (mode in listOf(AutopilotMode.MANUAL, AutopilotMode.BYPASS, AutopilotMode.OVERNIGHT, AutopilotMode.LOCKDOWN)) {
            assertEquals(mode.name, TileTap.OpenTrust, autopilotTap(settings(mode)))
        }
    }

    @Test
    fun `tapping Lockdown turns it on with no check`() {
        launch()
        awaitTag("noActivity")
        val tile = Robolectric.buildService(LockdownTileService::class.java).create().get()
        tile.onClick()
        awaitCore { core.modeCalls.any { it.first == null && it.second == AutopilotMode.LOCKDOWN } }
        assertEquals(0, prompts.get())
    }

    @Test
    fun `tapping Lockdown while on does not end it`() {
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.LOCKDOWN)
        launch()
        awaitTag("noActivity")
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.LOCKDOWN }
        core.modeCalls.clear()
        val tile = Robolectric.buildService(LockdownTileService::class.java).create().get()
        tile.onClick()
        settle()
        assertTrue("ending opens Reins; nothing changes from the tile", core.modeCalls.isEmpty())
    }

    @Test
    fun `the Autopilot tile takes Auto back to Manual, and leaves Manual alone`() {
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO)
        launch()
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.AUTO }
        Robolectric.buildService(AutopilotTileService::class.java).create().get().onClick()
        awaitCore { core.modeCalls.any { it.second == AutopilotMode.MANUAL } }
        core.modeCalls.clear()
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.MANUAL }
        Robolectric.buildService(AutopilotTileService::class.java).create().get().onClick()
        settle()
        assertTrue(core.modeCalls.isEmpty())
        // What waits only ever opens Reins.
        Robolectric.buildService(WaitingTileService::class.java).create().get().onClick()
        settle()
        assertTrue(core.modeCalls.isEmpty())
    }
}
