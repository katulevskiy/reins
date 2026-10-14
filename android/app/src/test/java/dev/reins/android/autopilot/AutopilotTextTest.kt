package dev.reins.android.autopilot

import dev.reins.android.TestData
import dev.reins.android.feedback.Event
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot
import dev.reins.core.ModelState
import dev.reins.core.Verdict
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AutopilotTextTest {
    private fun settings(bypass: Long? = null, connections: List<ConnectionAutopilot> = emptyList()) = AutopilotSettings(
        if (bypass != null) AutopilotMode.BYPASS else AutopilotMode.MANUAL, AutopilotMode.MANUAL, bypass, bypass?.let { 500L }, "personal", true,
        TestData.modelStatus(ModelState.INSTALLED), connections,
    )

    @Test
    fun `a profile icon that is a plain word shows the default emoji`() {
        // Accounts made earlier have the words "person" and "work" as the default profiles' icons.
        assertEquals("🙂", AutopilotText.profileIcon(TestData.profile(icon = "person")))
        assertEquals("💼", AutopilotText.profileIcon(TestData.profile(name = "Work", icon = "work")))
        assertEquals("🏠", AutopilotText.profileIcon(TestData.profile(icon = "🏠")))
        assertEquals("🙂", AutopilotText.profileIcon(TestData.profile(icon = " ")))
    }

    @Test
    fun `switching modes sounds like what it lets happen`() {
        assertEquals(Event.AutopilotOn, AutopilotText.modeChangeEvent(AutopilotMode.MANUAL, AutopilotMode.AUTO))
        assertEquals(Event.AutopilotOff, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.MANUAL))
        assertEquals(Event.BypassOn, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.BYPASS))
        assertEquals(Event.BypassOff, AutopilotText.modeChangeEvent(AutopilotMode.BYPASS, AutopilotMode.MANUAL))
        assertEquals(Event.LockdownOn, AutopilotText.modeChangeEvent(AutopilotMode.BYPASS, AutopilotMode.LOCKDOWN))
        assertEquals(Event.LockdownOff, AutopilotText.modeChangeEvent(AutopilotMode.LOCKDOWN, AutopilotMode.AUTO))
        assertNull(AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.AUTO))
    }

    @Test
    fun `the modes run from Manual to Lockdown and step by autonomy`() {
        assertEquals(listOf("Manual", "Auto", "Bypass", "Lockdown"), AutopilotText.modes.map(AutopilotText::name))
        assertEquals(
            listOf(AutopilotMode.LOCKDOWN, AutopilotMode.MANUAL, AutopilotMode.AUTO, AutopilotMode.BYPASS),
            AutopilotText.byRisk,
        )
        assertEquals("Ask, with a hint", AutopilotText.line(AutopilotMode.MANUAL))
        assertTrue(AutopilotText.needsModel(AutopilotMode.AUTO))
        assertTrue("Manual only goes without its hints", !AutopilotText.needsModel(AutopilotMode.MANUAL))
    }

    @Test
    fun `time left reads in minutes, hours or days, and as a clock`() {
        assertEquals("42 min left", AutopilotText.timeLeft(1_000 + 41 * 60 + 5, 1_000))
        assertEquals("1 min left", AutopilotText.timeLeft(1_010, 1_000))
        assertEquals("5 h left", AutopilotText.timeLeft(1_000 + 4 * 3_600 + 10, 1_000))
        assertEquals("3 days left", AutopilotText.timeLeft(1_000 + 3 * 86_400, 1_000))
        assertEquals("14:05", AutopilotText.clock(1_000 + 14 * 60 + 5, 1_000))
        assertEquals("0:00", AutopilotText.clock(900, 1_000))
        assertEquals("5:04:09", AutopilotText.clock(1_000 + 5 * 3_600 + 4 * 60 + 9, 1_000))
        assertEquals("2d 4h", AutopilotText.clock(1_000 + 2 * 86_400 + 4 * 3_600 + 59, 1_000))
    }

    @Test
    fun `a bypass without an end shows no countdown, only how long it has run`() {
        val off = AutopilotText.UNTIL_OFF
        assertTrue(AutopilotText.isUntilOff(off))
        assertTrue(!AutopilotText.isUntilOff(1_000))
        assertEquals("until you stop it", AutopilotText.timeLeft(off, 1_000))
        assertEquals("∞", AutopilotText.clock(off, 1_000))
        assertEquals(1f, AutopilotText.bypassFraction(off, 0, 1_000))
        assertEquals("just started", AutopilotText.ranFor(1_000, 1_030))
        assertEquals("12 min so far", AutopilotText.ranFor(0, 12 * 60 + 5))
        assertEquals("5 h so far", AutopilotText.ranFor(0, 5 * 3_600 + 100))
        assertEquals("2 days so far", AutopilotText.ranFor(0, 2 * 86_400 + 100))
        assertEquals("On until you stop it · 2 days so far", AutopilotText.bypassLine(off, 0, 2 * 86_400 + 100))
        assertEquals("On until you stop it", AutopilotText.bypassLine(off, null, 5))
        assertEquals("Approving without asking", AutopilotText.bypassLine(1_600, 1_000, 1_100))
        val notice = AutopilotText.bypassNotice(settings(bypass = off), { it }, 1_000)!!
        assertEquals("Bypass on · until you stop it", notice.title)
        assertTrue(notice.untilOff)
        assertEquals(500L, notice.since)
        assertNull("nothing to wait for", notice.nextEnd)
    }

    @Test
    fun `the bypass ring measures from its start, or against the length most likely chosen`() {
        assertEquals(0.5f, AutopilotText.bypassFraction(2_000, 1_000, 1_500))
        assertEquals(0f, AutopilotText.bypassFraction(2_000, 1_000, 3_000))
        assertEquals(15 * 60L, AutopilotText.bypassLength(14 * 60))
        assertEquals(60 * 60L, AutopilotText.bypassLength(16 * 60))
        assertEquals(1_440 * 60L, AutopilotText.bypassLength(5 * 3_600))
    }

    @Test
    fun `bypass lengths read as people say them, within a year`() {
        assertEquals(listOf("15 min", "1 hour", "1 day"), AutopilotText.bypassMinutes.map(AutopilotText::lengthLabel))
        assertEquals(listOf("4 hours", "1 week"), AutopilotText.bypassMoreMinutes.map(AutopilotText::lengthLabel))
        assertEquals("3 days", AutopilotText.lengthLabel(AutopilotText.customMinutes(3, AutopilotText.BypassUnit.Days)))
        assertEquals("2 weeks", AutopilotText.lengthLabel(AutopilotText.customMinutes(2, AutopilotText.BypassUnit.Weeks)))
        assertEquals("7 min", AutopilotText.lengthLabel(AutopilotText.customMinutes(7, AutopilotText.BypassUnit.Minutes)))
        assertEquals("Until off", AutopilotText.lengthLabel(null))
        assertEquals(1u, AutopilotText.customMinutes(0, AutopilotText.BypassUnit.Minutes))
        assertEquals(52u * 10_080u, AutopilotText.customMinutes(99, AutopilotText.BypassUnit.Weeks))
        assertTrue(AutopilotText.customMinutes(99, AutopilotText.BypassUnit.Days) <= AutopilotText.MAX_BYPASS_MINUTES)
    }

    @Test
    fun `thresholds, replays, kinds and profiles read in a few words`() {
        val custom = TestData.profile(preset = dev.reins.core.Preset.CUSTOM, approveAt = 0.93f, denyAt = 0.88f)
        assertEquals("Approves when 93% sure, denies when 88% sure", AutopilotText.presetLine(custom))
        assertEquals(
            "Last 30 days at these: 41 decided for you, 12 asked, 0 you'd have undone.",
            AutopilotText.replayLine(dev.reins.core.ThresholdReplay(30u, 41u, 12u, 0u)),
        )
        assertEquals("94%", AutopilotText.share(0.94f))
        assertEquals("—", AutopilotText.share(null))
        assertEquals("On Auto · 26 decisions", AutopilotText.kindLine(TestData.classView(decisions = 26u, autoApprove = true, toUnlock = 0u)))
        assertEquals("8 more to unlock", AutopilotText.kindLine(TestData.classView(toUnlock = 8u)))
        assertEquals("Always asks", AutopilotText.kindLine(TestData.classView(manual = false)))
        val p = TestData.profile(memory = 140u)
        assertEquals("Claude Code, ChatGPT · 140 decisions", AutopilotText.profileLine(p, listOf("Claude Code", "ChatGPT")))
        assertEquals("1 decision", AutopilotText.profileLine(TestData.profile(memory = 1u), emptyList()))
        val s = settings(connections = listOf(ConnectionAutopilot("c2", null, null, null, AutopilotMode.MANUAL, "work")))
        val all = listOf("c1" to "Claude Code", "c2" to "ChatGPT", "c3" to "Cursor")
        assertEquals(listOf("Claude Code", "Cursor"), AutopilotText.profileAis(p, s, all))
        assertEquals(listOf("ChatGPT"), AutopilotText.profileAis(TestData.profile("work", isDefault = false, connections = listOf("c2")), s, all))
    }

    @Test
    fun `the bypass notice covers the global bypass and each connection's`() {
        assertNull(AutopilotText.bypassNotice(settings(), { it }, 1_000))
        assertNull("an ended bypass shows nothing", AutopilotText.bypassNotice(settings(bypass = 900), { it }, 1_000))
        val global = AutopilotText.bypassNotice(settings(bypass = 1_000 + 600), { it }, 1_000)!!
        assertTrue(global.global)
        assertEquals("Bypass on · 10 min left", global.title)
        assertTrue(global.text.contains("every AI"))
        val one = AutopilotText.bypassNotice(
            settings(connections = listOf(ConnectionAutopilot("c1", null, 1_000 + 120, null, AutopilotMode.BYPASS, "personal"))),
            { if (it == "c1") "Claude Code" else "?" },
            1_000,
        )!!
        assertTrue(one.text.contains("Claude Code"))
        assertEquals(listOf("c1"), one.connectionIds)
        assertEquals(1_120L, one.until)
    }

    @Test
    fun `automatic decisions are titled by who decided`() {
        assertEquals("Autopilot approved", AutopilotText.decisionTitle(TestData.autoDecision()))
        assertEquals("Autopilot denied", AutopilotText.decisionTitle(TestData.autoDecision(verdict = Verdict.DENY)))
        assertEquals("Approved by Bypass", AutopilotText.decisionTitle(TestData.autoDecision(decidedBy = "bypass")))
        assertEquals("Denied by Lockdown", AutopilotText.decisionTitle(TestData.autoDecision(verdict = Verdict.DENY, decidedBy = "lockdown")))
        assertEquals("91% sure", AutopilotText.decisionDetail(TestData.autoDecision()))
        assertNull(AutopilotText.decisionDetail(TestData.autoDecision(decidedBy = "bypass")))
    }

    @Test
    fun `suggestions say what Autopilot would do and why it holds back`() {
        assertEquals("Autopilot would approve · 97%", AutopilotText.suggestionHeadline(TestData.suggestion()))
        assertEquals("Autopilot would deny · 90%", AutopilotText.suggestionHeadline(TestData.suggestion(verdict = Verdict.DENY, pDeny = 0.9f)))
        assertEquals("Autopilot always asks you for this", AutopilotText.suggestionHeadline(TestData.suggestion(floor = true, judged = false)))
        assertTrue(AutopilotText.suggestionNotes(TestData.suggestion(novel = true)).single().contains("never approved"))
        assertTrue(AutopilotText.suggestionNotes(TestData.suggestion(judged = false, reason = "No model on this phone")).contains("No model on this phone"))
    }

    @Test
    fun `a class fills its ring with answers and is full once it runs by itself`() {
        assertEquals(0.6f, AutopilotText.unlockProgress(TestData.classView(decisions = 12u, toUnlock = 8u)), 0.001f)
        assertEquals(1f, AutopilotText.unlockProgress(TestData.classView(autoApprove = true, toUnlock = 0u)), 0f)
        assertEquals("8 more decisions to unlock", AutopilotText.classStatus(TestData.classView()))
        assertEquals("Locked by you · always asks", AutopilotText.classStatus(TestData.classView(manual = false)))
        assertEquals("Approves on its own", AutopilotText.classStatus(TestData.classView(autoApprove = true, toUnlock = 0u)))
        assertEquals("11 approved · 1 denied · 96% accurate", AutopilotText.classNumbers(TestData.classView(accuracy = 0.96f)))
    }

    @Test
    fun `a model that fails its check says so plainly`() {
        assertTrue(AutopilotText.modelError("sha-256 of model.onnx does not match").contains("did not match"))
        assertEquals("Connection reset.", AutopilotText.modelError("connection reset"))
        assertTrue(AutopilotText.modelError(null).contains("Try again"))
    }

    @Test
    fun `the model card states sizes in megabytes and knows the base checkpoint's range`() {
        assertEquals("about 300–450 MB", AutopilotText.modelSize(TestData.modelStatus()))
        val downloading = TestData.modelStatus(ModelState.DOWNLOADING, downloaded = 103_000_000u, size = 412_000_000u)
        assertEquals("Downloading · 103 MB of 412 MB", AutopilotText.modelState(downloading, waitingForNetwork = false, wifiOnly = true))
        assertEquals(0.25f, AutopilotText.fraction(downloading)!!, 0.001f)
        assertEquals("Waiting for Wi-Fi", AutopilotText.modelState(TestData.modelStatus(), waitingForNetwork = true, wifiOnly = true))
    }

    @Test
    fun `the examples use the situation format`() {
        AutopilotText.examples.forEach { e ->
            assertTrue(e.title, e.situation.startsWith("connection: "))
            assertTrue(e.title, e.situation.lines().all { it.contains(": ") || it == "--- written by the AI ---" })
        }
    }
}
