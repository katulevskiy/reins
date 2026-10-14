package dev.reins.android.autopilot

import dev.reins.android.TestData
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot
import dev.reins.core.ModelState
import dev.reins.core.OvernightRisky
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** What a mode change shows before the core confirms it follows the core's rules (`set_autopilot_mode`, `modes::effective`). */
class AutopilotDraftTest {
    private val now = 1_000_000L

    private fun settings(mode: AutopilotMode = AutopilotMode.AUTO, connections: List<ConnectionAutopilot> = emptyList()) = AutopilotSettings(
        mode = mode,
        baseMode = mode,
        bypassUntil = null,
        bypassSince = null,
        overnightWhenRisky = OvernightRisky.WAIT,
        defaultProfileId = "personal",
        wifiOnly = true,
        model = TestData.modelStatus(ModelState.INSTALLED),
        connections = connections,
    )

    private fun own(id: String, base: AutopilotMode?, mode: AutopilotMode = base ?: AutopilotMode.AUTO) =
        ConnectionAutopilot(connectionId = id, baseMode = base, bypassUntil = null, bypassSince = null, mode = mode, profileId = "personal")

    @Test
    fun `a global mode reaches the connections that follow it, not those with their own`() {
        val s = AutopilotDraft.withMode(settings(connections = listOf(own("a", null), own("b", AutopilotMode.LOCKDOWN))), null, AutopilotMode.MANUAL, null, now)
        assertEquals(AutopilotMode.MANUAL, s.mode)
        assertEquals(AutopilotMode.MANUAL, s.baseMode)
        assertEquals(listOf(AutopilotMode.MANUAL, AutopilotMode.LOCKDOWN), s.connections.map { it.mode })
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
        assertEquals(AutopilotMode.AUTO, bypass.baseMode)
        assertEquals(now + 15 * 60, bypass.bypassUntil)
        assertEquals(AutopilotMode.MANUAL, AutopilotDraft.withMode(settings(AutopilotMode.LOCKDOWN), null, AutopilotMode.BYPASS, 15u, now).baseMode)
        val stopped = AutopilotDraft.withoutBypass(bypass, null)
        assertEquals(AutopilotMode.AUTO, stopped.mode)
        assertNull(stopped.bypassUntil)
    }

    @Test
    fun `a connection without settings of its own gets them, and following again drops its mode`() {
        val s = AutopilotDraft.withMode(settings(), "new", AutopilotMode.MANUAL, null, now)
        assertEquals(own("new", AutopilotMode.MANUAL), s.connections.single())
        val back = AutopilotDraft.withMode(s, "new", null, null, now)
        assertEquals(AutopilotMode.AUTO, back.connections.single().mode)
        assertNull(back.connections.single().baseMode)
    }

    @Test
    fun `profiles move with the default`() {
        val s = settings(connections = listOf(own("a", null), own("b", null).copy(profileId = "work")))
        assertEquals(listOf("home", "work"), AutopilotDraft.withDefaultProfile(s, "home").connections.map { it.profileId })
        assertEquals("personal", AutopilotDraft.withProfile(s, "b", null).connections.last().profileId)
    }

    @Test
    fun `a bypass until turned off shows at once with no end, and stops like any other`() {
        val on = AutopilotDraft.withBypass(settings(), null, null, now)
        assertEquals(AutopilotMode.BYPASS, on.mode)
        assertEquals(AutopilotText.UNTIL_OFF, on.bypassUntil)
        assertEquals(now, on.bypassSince)
        val timed = AutopilotDraft.withBypass(settings(), "c1", 3u * 1_440u, now)
        val own = timed.connections.single()
        assertEquals(now + 3 * 86_400, own.bypassUntil)
        assertEquals(AutopilotMode.BYPASS, own.mode)
        val off = AutopilotDraft.withoutBypass(on, null)
        assertEquals(AutopilotMode.AUTO, off.mode)
        assertNull(off.bypassUntil)
        assertNull(off.bypassSince)
    }

    @Test
    fun `a global bypass shows on every AI not locked down, whatever its own mode`() {
        val s = AutopilotDraft.withBypass(
            settings(mode = AutopilotMode.MANUAL, connections = listOf(own("a", AutopilotMode.AUTO), own("b", AutopilotMode.LOCKDOWN))),
            null,
            15u,
            now,
        )
        assertEquals(listOf(AutopilotMode.BYPASS, AutopilotMode.LOCKDOWN), s.connections.map { it.mode })
    }

    @Test
    fun `a global Overnight shows at once, ends a lockdown, and passes over AIs kept on Manual`() {
        val s = AutopilotDraft.withOvernight(
            settings(mode = AutopilotMode.LOCKDOWN, connections = listOf(own("a", null, AutopilotMode.LOCKDOWN), own("m", AutopilotMode.MANUAL, AutopilotMode.LOCKDOWN))),
            null,
            8u * 60u,
            now,
        )
        assertEquals(AutopilotMode.OVERNIGHT, s.mode)
        assertEquals(AutopilotMode.OVERNIGHT, s.baseMode)
        assertEquals(now + 8 * 3_600, s.overnightUntil)
        assertEquals(now, s.overnightSince)
        assertEquals("one kept on Manual stays on it", listOf(AutopilotMode.OVERNIGHT, AutopilotMode.MANUAL), s.connections.map { it.mode })
        val untilOff = AutopilotDraft.withOvernight(settings(), null, null, now)
        assertEquals(AutopilotText.UNTIL_OFF, untilOff.overnightUntil)
    }

    @Test
    fun `a running bypass stays in front of a global Overnight started under it`() {
        val bypass = AutopilotDraft.withBypass(settings(), null, 15u, now)
        val s = AutopilotDraft.withOvernight(bypass, null, 60u, now)
        assertEquals(AutopilotMode.BYPASS, s.mode)
        assertEquals(AutopilotMode.OVERNIGHT, s.baseMode)
    }

    @Test
    fun `an AI's own Overnight wins over the global mode and stops like another mode`() {
        val s = AutopilotDraft.withOvernight(settings(connections = listOf(own("a", AutopilotMode.MANUAL))), "a", 60u, now)
        val a = s.connections.single()
        assertEquals(AutopilotMode.OVERNIGHT, a.mode)
        assertEquals(AutopilotMode.OVERNIGHT, a.baseMode)
        assertEquals(now + 3_600, a.overnightUntil)
        val stopped = AutopilotDraft.withMode(s, "a", AutopilotMode.MANUAL, null, now).connections.single()
        assertEquals(AutopilotMode.MANUAL, stopped.mode)
        assertNull(stopped.overnightUntil)
        assertNull(stopped.overnightSince)
        val global = AutopilotDraft.withMode(AutopilotDraft.withOvernight(settings(), null, null, now), null, AutopilotMode.AUTO, null, now)
        assertEquals(AutopilotMode.AUTO, global.mode)
        assertNull(global.overnightUntil)
    }

    @Test
    fun `Stop returns to the mode an Overnight interrupted, never to a lockdown`() {
        assertEquals(AutopilotDraft.Interrupted.Mode(AutopilotMode.AUTO), AutopilotDraft.interrupted(settings(), null))
        assertEquals(AutopilotDraft.Interrupted.Mode(null), AutopilotDraft.interrupted(settings(AutopilotMode.LOCKDOWN), null))
        assertEquals(
            "starting again keeps what the first one interrupted",
            AutopilotDraft.Interrupted.Keep,
            AutopilotDraft.interrupted(AutopilotDraft.withOvernight(settings(), null, 60u, now), null),
        )
        val s = settings(connections = listOf(own("a", AutopilotMode.MANUAL)))
        assertEquals(AutopilotDraft.Interrupted.Mode(AutopilotMode.MANUAL), AutopilotDraft.interrupted(s, "a"))
        assertEquals("one without its own mode follows the global one again", AutopilotDraft.Interrupted.Mode(null), AutopilotDraft.interrupted(s, "new"))
    }
}
