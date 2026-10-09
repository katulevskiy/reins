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
        if (bypass != null) AutopilotMode.BYPASS else AutopilotMode.ASSISTED, AutopilotMode.ASSISTED, bypass, "personal", true,
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
        assertEquals(Event.AutopilotOff, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.ASSISTED))
        assertEquals(Event.BypassOn, AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.BYPASS))
        assertEquals(Event.BypassOff, AutopilotText.modeChangeEvent(AutopilotMode.BYPASS, AutopilotMode.MANUAL))
        assertEquals(Event.LockdownOn, AutopilotText.modeChangeEvent(AutopilotMode.BYPASS, AutopilotMode.LOCKDOWN))
        assertEquals(Event.LockdownOff, AutopilotText.modeChangeEvent(AutopilotMode.LOCKDOWN, AutopilotMode.AUTO))
        assertNull(AutopilotText.modeChangeEvent(AutopilotMode.AUTO, AutopilotMode.AUTO))
    }

    @Test
    fun `time left reads in whole minutes and as a clock`() {
        assertEquals("42 min left", AutopilotText.minutesLeft(1_000 + 41 * 60 + 5, 1_000))
        assertEquals("1 min left", AutopilotText.minutesLeft(1_010, 1_000))
        assertEquals("14:05", AutopilotText.clock(1_000 + 14 * 60 + 5, 1_000))
        assertEquals("0:00", AutopilotText.clock(900, 1_000))
    }

    @Test
    fun `the bypass ring measures against the length most likely chosen`() {
        assertEquals(15 * 60L, AutopilotText.bypassLength(14 * 60))
        assertEquals(30 * 60L, AutopilotText.bypassLength(16 * 60))
        assertEquals(60 * 60L, AutopilotText.bypassLength(45 * 60))
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
            settings(connections = listOf(ConnectionAutopilot("c1", null, 1_000 + 120, AutopilotMode.BYPASS, "personal"))),
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
