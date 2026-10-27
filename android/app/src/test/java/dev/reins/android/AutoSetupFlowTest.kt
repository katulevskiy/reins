package dev.reins.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.core.AutopilotMode
import dev.reins.core.CellChoice
import dev.reins.core.FactChoice
import dev.reins.core.ModelState
import dev.reins.core.WorldItem
import dev.reins.core.ZoneChoice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Turning on Auto the first time (Decisions v2, 01 §9.1): Your world, the matrix, then Turn on. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class AutoSetupFlowTest : FlowHarness() {
    private fun world() = listOf(
        WorldItem(FactChoice.OWNER, "dkat", "dkat", "github", ZoneChoice.MINE, true, 14u),
        WorldItem(FactChoice.OWNER, "acme", "acme", "github", ZoneChoice.WORK, false, 6u),
        WorldItem(FactChoice.PERSON, "ann@example.com", "ann@example.com", "email", ZoneChoice.OUTSIDE, false, 2u),
    )

    private fun openTrust() {
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u)
        core.autoSetupDone = false
        core.world = world()
        launch()
        tap("tabTrust")
        awaitTag("mode:MANUAL")
    }

    @Test
    fun turningOnAutoGuidesFirst() {
        openTrust()
        tap("mode:AUTO")
        awaitTag("autoSetup")
        assertTrue("picking Auto changes nothing until the setup is done", core.modeCalls.isEmpty())
        assertTrue(has("autoSetupStep:1"))
        // Your world: the services with something in them, each item with its count and zone; proposals marked.
        assertTrue(has("worldSection:github") && has("worldSection:email"))
        assertFalse("an empty section is hidden", has("worldSection:messages") || has("worldSection:computers"))
        assertTrue(has("count:OWNER:dkat"))
        assertTrue(has("proposed:OWNER:acme"))
        assertFalse("confirmed is not a proposal", has("proposed:OWNER:dkat"))
        tap("autoSetupNext")
        // The matrix: real names under the zones, a tap moves a cell along, the last column stays Ask.
        awaitTag("matrix")
        assertTrue(showsText("dkat"))
        tap("cell:WORK:1")
        awaitCore { core.matrixCellCalls.isNotEmpty() }
        assertEquals(Triple(ZoneChoice.WORK, 1.toUByte(), CellChoice.AUTO), core.matrixCellCalls.single())
        assertEquals("looser asks the owner", 1, prompts.get())
        tap("cell:MINE:3")
        settle()
        assertEquals("can't be undone always asks", 1, core.matrixCellCalls.size)
        tap("autoSetupNext")
        awaitTag("turnOnAuto")
        // Back steps back through the setup.
        tap("back")
        awaitTag("matrix")
        tap("back")
        awaitTag("worldSection:github")
        tap("back")
        awaitGone("autoSetup")
        assertTrue("leaving the setup leaves the mode", core.modeCalls.isEmpty())
        assertEquals(AutopilotMode.MANUAL, container.state.autopilot.value?.mode)
    }

    @Test
    fun zoneChangeTowardMineAsksTheOwner() {
        openTrust()
        tap("mode:AUTO")
        awaitTag("autoSetup")
        // Stricter (toward Outside) needs nothing.
        tap("zone:OWNER:dkat:OUTSIDE")
        awaitCore { core.worldZoneCalls.isNotEmpty() }
        assertEquals(Triple(FactChoice.OWNER, "dkat", ZoneChoice.OUTSIDE), core.worldZoneCalls.single())
        assertEquals(0, prompts.get())
        // Looser asks the owner; refused, nothing changes and the old zone shows again.
        authResult = AuthResult.Cancelled
        tap("zone:PERSON:ann@example.com:FRIENDS")
        awaitCore { prompts.get() == 1 }
        assertEquals(1, core.worldZoneCalls.size)
        assertEquals("Treat ann@example.com as Friends", core.autoOwnerReasons.last())
        awaitCore { core.world.first { it.key == "ann@example.com" }.zone == ZoneChoice.OUTSIDE }
        assertFalse("a cancelled check says nothing", has("autoSetupError"))
        // Passed, it lands.
        authResult = AuthResult.Success
        tap("zone:PERSON:ann@example.com:MINE")
        awaitCore { core.worldZoneCalls.size == 2 }
        assertEquals(2, prompts.get())
        assertEquals(Triple(FactChoice.PERSON, "ann@example.com", ZoneChoice.MINE), core.worldZoneCalls.last())
        awaitGone("proposed:PERSON:ann@example.com")
    }

    @Test
    fun finishTurnsAutoOn() {
        openTrust()
        tap("mode:AUTO")
        awaitTag("autoSetup")
        tap("autoSetupNext")
        tap("autoSetupNext")
        // Refused, Auto stays off and the setup stays.
        authResult = AuthResult.Cancelled
        hold("turnOnAuto")
        awaitCore { prompts.get() == 1 }
        assertEquals(0, core.finishSetupCalls.get())
        assertTrue(has("autoSetup"))
        authResult = AuthResult.Success
        hold("turnOnAuto")
        awaitCore { core.finishSetupCalls.get() == 1 }
        assertEquals("Turn on Auto", core.autoOwnerReasons.last())
        // Back on Trust with Auto on; every proposal was confirmed as shown.
        awaitGone("autoSetup")
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.AUTO }
        awaitTag("mode:AUTO")
        assertTrue(core.world.all { it.confirmed })
        // Done once: Manual and back to Auto sets it straight away.
        tap("mode:MANUAL")
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.MANUAL }
        tap("mode:AUTO")
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.AUTO }
        assertFalse(has("autoSetup"))
    }
}
