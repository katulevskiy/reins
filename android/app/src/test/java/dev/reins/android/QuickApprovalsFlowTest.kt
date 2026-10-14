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
    fun oneHourInTheDockRemembersOnlyWhatIsShown() {
        val view = TestData.quick(TestData.searchView())
        openRequest(view)
        tap("check:m2")
        tap("lifetime:HOUR")
        assertFalse("nothing wider than the screen is on", has("likeThisNote"))
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val choice = core.approvals.single().second
        val standing = choice.standing!!
        assertEquals(3_600uL, standing.durationSecs)
        assertFalse("never all mail implicitly", standing.scope.allMail)
        assertTrue(standing.scope.selectedMessagesOnly)
        assertEquals(listOf("m1", "m3"), choice.selectedMessageIds)
        assertEquals("one screen lock or biometric check", 1, prompts.get())
    }

    @Test
    fun anythingLikeThisIsAnExplicitChoiceTheDockShows() {
        val view = TestData.quick(TestData.searchView())
        openRequest(view)
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
        rule.onNodeWithTag("allowWhat", useUnmergedTree = true).assertTextContains("searching and reading me@gmail.com", substring = true)
        tap("scope:like")
        tap("rememberConfirm")
        awaitTag("likeThisNote")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(view.quick!!.allow, core.approvals.single().second.standing)
    }

    @Test
    fun repeatedApprovalsOfferEightHours() {
        openRequest(TestData.quick(TestData.searchView(), repeats = 3u))
        rule.onNodeWithTag("lifetime:HOUR").assertTextContains("8 hours", substring = true)
        tap("lifetime:HOUR")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(28_800uL, core.approvals.single().second.standing!!.durationSecs)
    }

    @Test
    fun whatIsAskedEveryTimeHasNoShortcut() {
        openRequest(TestData.vaultView())
        awaitTag("noStanding")
        assertFalse(has("lifetime:HOUR"))
    }

    @Test
    fun anAisRoutineRequestsAreOneCardAllowedWithOneCheckAndWhatNeedsALookStays() {
        core.pending = listOf(
            TestData.pending("r1", quick = true),
            TestData.pending("r2", "read", quick = true),
            TestData.pending("r3", quick = true),
            TestData.pending("r4", "grant", 1u),
            TestData.pending("r5", label = "Codex", conn = "c2", quick = true),
        )
        for (id in listOf("r1", "r2", "r3")) core.suggestions[id] = TestData.suggestion(id)
        dev.reins.android.platform.Foreground.autoPopup = false
        launch()
        awaitTag("burst")
        // Codex's one routine request is no group: it keeps its own card.
        assertTrue(has("pending:r5"))
        rule.onNodeWithTag("burstTitle", useUnmergedTree = true).assertTextEquals("Wants to search Gmail 2 times and read 2 emails")
        awaitTag("approveAll")
        rule.onNodeWithTag("approveAll").assertTextEquals("Allow 3")
        tap("approveAll")
        awaitCore { core.quickApprovals.size == 3 }
        assertEquals(listOf("r1", "r2", "r3"), core.quickApprovals.toList().sorted())
        assertEquals(1, prompts.get())
        awaitGone("burst")
        assertTrue(has("pending:r4"))
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun eachAiHasItsOwnGroupAndDenyTakesBackOnlyItsRequests() {
        core.pending = listOf(
            TestData.pending("r1", quick = true),
            TestData.pending("r2", quick = true),
            TestData.pending("r3", "grant", 1u),
            TestData.pending("r4", label = "Codex", conn = "c2", quick = true),
            TestData.pending("r5", label = "Codex", conn = "c2", quick = true),
        )
        for (id in listOf("r1", "r2")) core.suggestions[id] = TestData.suggestion(id, verdict = dev.reins.core.Verdict.DENY, pApprove = 0.02f, pDeny = 0.95f)
        dev.reins.android.platform.Foreground.autoPopup = false
        launch()
        awaitTag("denyAll")
        assertEquals(2, rule.onAllNodes(androidx.compose.ui.test.hasTestTag("burst")).fetchSemanticsNodes().size)
        // Codex's group has no suggestion: Review only.
        assertEquals(1, rule.onAllNodes(androidx.compose.ui.test.hasTestTag("denyAll")).fetchSemanticsNodes().size)
        tap("denyAll")
        awaitCore { core.denials.size == 2 }
        assertEquals(listOf("r1", "r2"), core.denials.toList().sorted())
        assertEquals("deny needs no check", 0, prompts.get())
        assertTrue("what needs a look is not denied with them", has("pending:r3"))
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
