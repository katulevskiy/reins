package dev.reins.android.autopilot

import dev.reins.android.TestData
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot
import dev.reins.core.ModelState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** What a mode change shows before the core confirms it follows the core's rules (`set_autopilot_mode`, `modes::effective`). */
class AutopilotDraftTest {
    private val now = 1_000_000L

    private fun settings(mode: AutopilotMode = AutopilotMode.ASSISTED, connections: List<ConnectionAutopilot> = emptyList()) = AutopilotSettings(
        mode, mode, null, "personal", true, TestData.modelStatus(ModelState.INSTALLED), connections,
    )

    private fun own(id: String, base: AutopilotMode?, mode: AutopilotMode = base ?: AutopilotMode.ASSISTED) =
        ConnectionAutopilot(id, base, null, mode, "personal")

    @Test
    fun `a global mode reaches the connections that follow it, not those with their own`() {
        val s = AutopilotDraft.withMode(settings(connections = listOf(own("a", null), own("b", AutopilotMode.MANUAL))), null, AutopilotMode.AUTO, null, now)
        assertEquals(AutopilotMode.AUTO, s.mode)
        assertEquals(AutopilotMode.AUTO, s.baseMode)
        assertEquals(listOf(AutopilotMode.AUTO, AutopilotMode.MANUAL), s.connections.map { it.mode })
    }

    @Test
    fun `a global lockdown wins over every connection`() {
        val s = AutopilotDraft.withMode(settings(connections = listOf(own("b", AutopilotMode.AUTO))), null, AutopilotMode.LOCKDOWN, null, now)
        assertEquals(AutopilotMode.LOCKDOWN, s.connections.single().mode)
    }

    @Test
    fun `a bypass keeps the base mode and ends a lockdown`() {
        val bypass = AutopilotDraft.withMode(settings(), null, AutopilotMode.BYPASS, 15u, now)
        assertEquals(AutopilotMode.BYPASS, bypass.mode)
        assertEquals(AutopilotMode.ASSISTED, bypass.baseMode)
        assertEquals(now + 15 * 60, bypass.bypassUntil)
        assertEquals(AutopilotMode.ASSISTED, AutopilotDraft.withMode(settings(AutopilotMode.LOCKDOWN), null, AutopilotMode.BYPASS, 15u, now).baseMode)
        val stopped = AutopilotDraft.withoutBypass(bypass, null)
        assertEquals(AutopilotMode.ASSISTED, stopped.mode)
        assertNull(stopped.bypassUntil)
    }

    @Test
    fun `a connection without settings of its own gets them, and following again drops its mode`() {
        val s = AutopilotDraft.withMode(settings(), "new", AutopilotMode.MANUAL, null, now)
        assertEquals(own("new", AutopilotMode.MANUAL), s.connections.single())
        val back = AutopilotDraft.withMode(s, "new", null, null, now)
        assertEquals(AutopilotMode.ASSISTED, back.connections.single().mode)
        assertNull(back.connections.single().baseMode)
    }

    @Test
    fun `profiles move with the default`() {
        val s = settings(connections = listOf(own("a", null), own("b", null).copy(profileId = "work")))
        assertEquals(listOf("home", "work"), AutopilotDraft.withDefaultProfile(s, "home").connections.map { it.profileId })
        assertEquals("personal", AutopilotDraft.withProfile(s, "b", null).connections.last().profileId)
    }
}
