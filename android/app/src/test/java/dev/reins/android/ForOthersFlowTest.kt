package dev.reins.android

import android.app.Notification
import android.app.NotificationManager
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Foreground
import dev.reins.core.CircleEvent
import dev.reins.core.CircleReviewState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Circles on the helper's side: For others in Activity, its answers, and the one Circle notification. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ForOthersFlowTest : FlowHarness() {
    private fun open(vararg reviews: dev.reins.core.CircleReviewView) {
        core.circleReviewList = reviews.toList()
        Foreground.autoPopup = false
        launch()
        awaitTag("forOthers")
    }

    @Test
    fun approveAnswersWithNoNote() {
        open(TestData.circleReview("rv1"))
        tap("reviewApprove:rv1")
        awaitCore { core.circleAnswers.isNotEmpty() }
        assertEquals(Triple("rv1", true, null), core.circleAnswers.single())
        // The phone's own check first, as for one's own requests.
        assertEquals(1, prompts.get())
        // Settled: it stays, dimmed, with its outcome.
        awaitTag("reviewSettled:rv1")
        rule.onNodeWithTag("reviewState:rv1").assertTextContains("Approved")
    }

    @Test
    fun denyWithANotePassesTheNote() {
        open(TestData.circleReview("rv1"))
        tap("reviewDeny:rv1")
        awaitTag("reviewNote:rv1")
        assertTrue("nothing sent before the note", core.circleAnswers.isEmpty())
        rule.onNodeWithTag("reviewNote:rv1").performTextInput("Not from that store")
        tap("reviewDenyConfirm:rv1")
        awaitCore { core.circleAnswers.isNotEmpty() }
        assertEquals(Triple("rv1", false, "Not from that store"), core.circleAnswers.single())
        assertEquals("a denial asks no check", 0, prompts.get())
    }

    @Test
    fun denyWithoutANoteSendsNone() {
        open(TestData.circleReview("rv1"))
        tap("reviewDeny:rv1")
        tap("reviewDenyConfirm:rv1")
        awaitCore { core.circleAnswers.isNotEmpty() }
        assertEquals(Triple("rv1", false, null), core.circleAnswers.single())
    }

    @Test
    fun aCancelledCheckAnswersNothing() {
        authResult = AuthResult.Cancelled
        open(TestData.circleReview("rv1"))
        tap("reviewApprove:rv1")
        awaitCore { prompts.get() == 1 }
        assertTrue(core.circleAnswers.isEmpty())
        assertTrue(has("reviewApprove:rv1"))
    }

    @Test
    fun aFailedAnswerSaysWhyAndKeepsTheButtons() {
        core.circleAnswerFailure = dev.reins.core.CoreException.Invalid("The owner settled it.")
        open(TestData.circleReview("rv1"))
        tap("reviewApprove:rv1")
        awaitTag("reviewError:rv1")
        assertTrue(has("reviewApprove:rv1"))
    }

    @Test
    fun reviewsGroupByPersonWithDeadlinesAndScope() {
        open(
            TestData.circleReview("s1", "sam@example.com", left = 5 * 3_600),
            TestData.circleReview("d1", "dana@example.com", title = "Send the deck", left = 1_200),
        )
        assertTrue(has("forOthersPerson:sam@example.com"))
        assertTrue(has("forOthersPerson:dana@example.com"))
        rule.onNodeWithTag("forOthersCount").assertTextContains("2")
        rule.onNodeWithTag("reviewWho:s1").assertTextContains("Sam · ChatGPT")
        rule.onNodeWithTag("reviewDue:d1", useUnmergedTree = false).assertTextContains("Due in 20 min", substring = true)
        rule.onNodeWithTag("reviewScope:s1", useUnmergedTree = false).assertTextContains("Money", substring = true)
    }

    @Test
    fun theFullCardIsBehindMore() {
        open(TestData.circleReview("rv1", detail = "Sony WH-1000XM6, ships Friday"))
        assertFalse(has("reviewDetail:rv1"))
        tap("reviewMore:rv1")
        awaitTag("reviewDetail:rv1")
    }

    @Test
    fun settledOnesAndNoticesHaveNoButtons() {
        core.circleNoticeList = listOf(TestData.circleNotice("n1"))
        open(TestData.circleReview("rv1", state = CircleReviewState.WITHDRAWN, createdAt = 1_700_000_000))
        awaitTag("reviewSettled:rv1")
        awaitTag("circleNotice:n1")
        rule.onNodeWithTag("reviewState:rv1").assertTextContains("Withdrawn")
        assertFalse(has("reviewApprove:rv1"))
        assertFalse(has("reviewDeny:rv1"))
    }

    @Test
    fun aChangeRefreshesForOthersQuietly() {
        Foreground.autoPopup = false
        launch()
        awaitTag("activityList")
        assertFalse(has("forOthers"))
        core.circleReviewList = listOf(TestData.circleReview("rv1"))
        container.notifier.circleChanged(CircleEvent.Changed)
        awaitTag("circleReview:rv1")
        val manager = context.getSystemService(NotificationManager::class.java)
        assertTrue("no notification for a change", shadowOf(manager).activeNotifications.isEmpty())
    }

    @Test
    fun aWaitingReviewIsTheOneNotificationWithOpenOnly() {
        val review = TestData.circleReview("rv1")
        container.notifier.circleChanged(CircleEvent.ReviewWaiting(review))
        val manager = context.getSystemService(NotificationManager::class.java)
        val posted = shadowOf(manager).activeNotifications.single().notification
        assertEquals("Sam: Buy headphones waiting", posted.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals(listOf("Open"), posted.actions.map { it.title.toString() })
        assertEquals(Notification.VISIBILITY_PRIVATE, posted.visibility)
    }

    @Test
    fun theOwnersCardSaysWhoElseDecides() {
        core.pending = listOf(TestData.pending("r1", circle = TestData.circleProgress()))
        Foreground.autoPopup = false
        launch()
        awaitTag("pending:r1")
        // Inside the card, which reads as one button: its own node in the unmerged tree.
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(androidx.compose.ui.test.hasTestTag("cardCircle:r1"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        rule.onNode(
            androidx.compose.ui.test.hasText("Needs Alex too") and androidx.compose.ui.test.hasParent(androidx.compose.ui.test.hasTestTag("cardCircle:r1")),
            useUnmergedTree = true,
        ).assertExists()
    }

    @Test
    fun onceTheOwnerApprovedTheButtonReadsAsDone() {
        core.pending = listOf(
            TestData.pending("r1", circle = TestData.circleProgress(waitingForYou = false, youApproved = true, line = "You approved · waiting for Alex")),
        )
        core.approval = TestData.searchView().copy(requestId = "r1")
        Foreground.autoPopup = false
        launch()
        awaitTag("allowDone:r1")
        assertFalse(has("allow:r1"))
        tap("review:r1")
        awaitTag("approvedByMe")
        assertFalse(has("approve"))
        rule.onNodeWithTag("circleLine", useUnmergedTree = false).assertTextContains("waiting for Alex", substring = true)
    }
}
