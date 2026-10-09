package dev.reins.android

import android.content.Intent
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AppNotifier
import dev.reins.android.platform.ApprovalActionReceiver
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Fewer taps for routine requests: the headline, "Approve and allow for a while", "Approve all", the notification's buttons. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class QuickApprovalsFlowTest : FlowHarness() {
    @Test
    fun theSheetSaysWhatHappensFirstAndAllowsTheSameForAnHourInOneTap() {
        val view = TestData.quick(TestData.searchView())
        openRequest(view)
        rule.onNodeWithTag("headline").assertTextEquals("Claude gets the 3 emails found for \"from:bank\".")
        rule.onNodeWithTag("approveAllow").assertTextEquals("Approve and allow for 1 hour")
        rule.onNodeWithTag("allowWhat").assertTextContains("searching and reading me@gmail.com", substring = true)
        assertFalse(has("repeatHint"))
        tap("approveAllow")
        awaitCore { core.approvals.isNotEmpty() }
        val choice = core.approvals.single().second
        assertEquals(view.quick!!.allow, choice.standing)
        assertEquals(listOf("m1", "m2", "m3"), choice.selectedMessageIds)
        assertEquals("one screen lock or biometric check", 1, prompts.get())
    }

    @Test
    fun repeatedApprovalsAreNoticedAndAnswerForEightHours() {
        openRequest(TestData.quick(TestData.searchView(), repeats = 3u))
        awaitTag("repeatHint")
        awaitText("You approved this 3 times in the last 24 hours.")
        rule.onNodeWithTag("approveAllow").assertTextEquals("Approve and allow for 8 hours")
    }

    @Test
    fun whatIsAskedEveryTimeHasNoShortcut() {
        openRequest(TestData.vaultView())
        assertFalse(has("quickAllow"))
        assertFalse(has("approveAllow"))
    }

    @Test
    fun moreOptionsReplaceTheShortcut() {
        openRequest(TestData.quick(TestData.searchView()))
        awaitTag("approveAllow")
        tap("moreToggle")
        awaitGone("approveAllow")
    }

    @Test
    fun aBurstFromOneAiIsApprovedWithOneCheckAndWhatNeedsALookStays() {
        core.pending = listOf(
            TestData.pending("r1", quick = true),
            TestData.pending("r2", "read", quick = true),
            TestData.pending("r3", quick = true),
            TestData.pending("r4", "grant", 1u),
            TestData.pending("r5", label = "Codex", conn = "c2", quick = true),
        )
        dev.reins.android.platform.Foreground.autoPopup = false
        launch()
        awaitTag("burst")
        // Codex's one routine request is no burst: the bar is Claude's alone.
        rule.onNodeWithTag("burstTitle").assertTextEquals("Claude · 4 waiting")
        rule.onNodeWithTag("burstHeld").assertTextEquals("1 needs a closer look")
        rule.onNodeWithTag("approveAll").assertTextEquals("Approve 3")
        tap("approveAll")
        awaitCore { core.quickApprovals.size == 3 }
        assertEquals(listOf("r1", "r2", "r3"), core.quickApprovals.toList())
        assertEquals(1, prompts.get())
        awaitGone("burst")
        assertTrue(has("pending:r4"))
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun burstsOfSeveralAisAreOneCompactBarThatAnswersOnlyTheirRoutineRequests() {
        core.pending = listOf(
            TestData.pending("r1", quick = true),
            TestData.pending("r2", quick = true),
            TestData.pending("r3", "grant", 1u),
            TestData.pending("r4", label = "Codex", conn = "c2", quick = true),
            TestData.pending("r5", label = "Codex", conn = "c2", quick = true),
        )
        dev.reins.android.platform.Foreground.autoPopup = false
        launch()
        awaitTag("burst")
        assertEquals(1, rule.onAllNodes(androidx.compose.ui.test.hasTestTag("burst")).fetchSemanticsNodes().size)
        rule.onNodeWithTag("burstTitle").assertTextEquals("4 routine · Claude and Codex")
        assertFalse(has("burstHeld"))
        tap("denyAll")
        awaitCore { core.denials.size == 4 }
        assertEquals(listOf("r1", "r2", "r4", "r5"), core.denials.toList().sorted())
        assertTrue("what needs a look is not denied with them", has("pending:r3"))
    }

    @Test
    fun denyAllDeniesEverythingThatAiAsked() {
        core.pending = listOf(TestData.pending("r1", quick = true), TestData.pending("r2", quick = true), TestData.pending("r4", "grant", 1u))
        dev.reins.android.platform.Foreground.autoPopup = false
        launch()
        tap("denyAll")
        awaitCore { core.denials.size == 3 }
        assertEquals(0, prompts.get())
    }

    @Test
    fun theNotificationsButtonsAnswerWithoutOpeningTheApp() {
        core.pending = listOf(TestData.pending("r1", quick = true), TestData.pending("r2", quick = true))
        context.sendBroadcast(
            Intent(context, ApprovalActionReceiver::class.java).setAction(ApprovalActionReceiver.ACTION_APPROVE).putExtra(AppNotifier.EXTRA_ID, "r1"),
        )
        context.sendBroadcast(
            Intent(context, ApprovalActionReceiver::class.java).setAction(ApprovalActionReceiver.ACTION_DENY).putExtra(AppNotifier.EXTRA_ID, "r2"),
        )
        settleBackground { core.quickApprovals.contains("r1") && core.denials.contains("r2") }
    }

    @Test
    fun anApproveThatFailsLeavesANoteToOpenIt() {
        core.pending = listOf(TestData.pending("r1", "grant", 1u))
        context.sendBroadcast(
            Intent(context, ApprovalActionReceiver::class.java).setAction(ApprovalActionReceiver.ACTION_APPROVE).putExtra(AppNotifier.EXTRA_ID, "r1"),
        )
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        settleBackground { org.robolectric.Shadows.shadowOf(manager).allNotifications.isNotEmpty() }
        val posted = org.robolectric.Shadows.shadowOf(manager).allNotifications.single()
        assertEquals("Not approved yet", posted.extras.getCharSequence("android.title").toString())
        assertTrue(core.quickApprovals.isEmpty())
    }

    private fun settleBackground(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!condition()) {
            org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
            check(System.currentTimeMillis() < deadline) { "timed out" }
            Thread.sleep(20)
        }
    }
}
