package dev.reins.android

import android.os.Looper
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Foreground
import dev.reins.core.CircleInviteProgress
import dev.reins.core.CircleInviteView
import dev.reins.core.CircleJoinStage
import dev.reins.core.CircleLinkState
import dev.reins.core.CircleRole
import dev.reins.core.CircleScopePreset
import java.time.Duration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Settings › Circles: the people, Pause and End, adding someone with a link and the code, joining someone's. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class CirclesFlowTest : FlowHarness() {
    private fun openCircles() {
        Foreground.autoPopup = false
        launch()
        tap("tabSettings")
        tap("circlesRow")
    }

    /** Lets the invitation's next poll run. */
    private fun nextPoll() {
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(2_600))
        settle()
    }

    @Test
    fun peopleAreGroupedBySide() {
        core.circlePeopleList = listOf(
            TestData.circlePerson("l1", "alex@example.com"),
            TestData.circlePerson("l2", "sam@example.com", role = CircleRole.GUARDIAN, mine = false, scopeLine = "Purchases"),
            TestData.circlePerson("l3", "kim@example.com", state = CircleLinkState.PAUSED),
        )
        openCircles()
        awaitTag("circlePerson:l1")
        rule.onNodeWithTag("circleLine:l1").assertTextContains("Co-sign · Money · Deferred")
        rule.onNodeWithTag("circleState:l3").assertTextContains("Paused")
        // The owner pauses; a helper only leaves.
        assertTrue(has("circlePause:l1"))
        assertFalse(has("circlePause:l2"))
        rule.onNodeWithTag("circleEnd:l2").assertTextContains("Leave")
        rule.onNodeWithTag("circlePause:l3").assertTextContains("Resume")
    }

    @Test
    fun pauseAndEndBehindAQuestion() {
        core.circlePeopleList = listOf(TestData.circlePerson("l1", "alex@example.com"))
        openCircles()
        tap("circlePause:l1")
        awaitCore { core.circlePauses.isNotEmpty() }
        assertEquals("l1" to true, core.circlePauses.single())
        awaitTag("circleState:l1")
        tap("circleEnd:l1")
        awaitText("End with Alex?")
        assertTrue("nothing ends before the answer", core.circleEnds.isEmpty())
        tap("circleEndConfirm")
        awaitCore { core.circleEnds.isNotEmpty() }
        assertEquals(listOf("l1"), core.circleEnds.toList())
        awaitGone("circlePerson:l1")
    }

    @Test
    fun aPhoneThatCantTakePartOffersThePrimarySetup() {
        core.circlesOn = false
        openCircles()
        awaitTag("circlesUnavailable")
        assertFalse(has("circleAdd"))
    }

    @Test
    fun addingSomeoneShowsTheLinkThenTheCodeThenConfirmsAfterTheHoldAndCheck() {
        openCircles()
        tap("circleAdd")
        tap("role:cosign")
        tap("preset:money")
        tap("classInstant")
        tap("circleCreate")
        awaitCore { core.circleDrafts.isNotEmpty() }
        val draft = core.circleDrafts.single()
        assertEquals(CircleRole.COSIGN, draft.role)
        assertEquals(CircleScopePreset.Money, draft.preset)
        assertFalse(draft.deferred)
        awaitTag("circleQr")
        rule.onNodeWithTag("circleLink").assertTextContains("reins-circle:127.0.0.1:8000/secret123")
        assertTrue(has("circleShare"))
        // They open it: both phones show the code.
        core.circleInviteSteps += CircleInviteProgress.Compare("482913", "alex@example.com")
        nextPoll()
        awaitTag("circleCode")
        rule.onNodeWithTag("circleCode").assertTextContains("482 913")
        rule.onNodeWithTag("circleHelper").assertTextContains("alex@example.com")
        assertFalse("no confirm before they accept", has("circleConfirm"))
        // They accept: hold, the phone's own check, then it starts.
        core.circleInviteSteps += CircleInviteProgress.Accepted("482913", "alex@example.com")
        nextPoll()
        hold("circleConfirm")
        awaitCore { core.circleInviteConfirms.isNotEmpty() }
        assertEquals(listOf("inv1"), core.circleInviteConfirms.toList())
        assertEquals(1, prompts.get())
        awaitTag("circleAdded")
    }

    @Test
    fun aCancelledCheckConfirmsNothing() {
        authResult = AuthResult.Cancelled
        openCircles()
        tap("circleAdd")
        tap("role:delegate")
        tap("preset:everything")
        tap("circleCreate")
        awaitTag("circleQr")
        core.circleInviteSteps += CircleInviteProgress.Accepted("482913", "alex@example.com")
        nextPoll()
        hold("circleConfirm")
        awaitCore { prompts.get() == 1 }
        assertTrue(core.circleInviteConfirms.isEmpty())
        assertTrue(has("circleConfirm"))
    }

    @Test
    fun cancellingWithdrawsTheInvitation() {
        openCircles()
        tap("circleAdd")
        tap("role:watcher")
        tap("preset:risky")
        // A watcher has no class to pick.
        assertFalse(has("classInstant"))
        tap("circleCreate")
        awaitTag("circleQr")
        tap("circleCancel")
        awaitCore { core.circleInviteCancels.isNotEmpty() }
        assertEquals(listOf("inv1"), core.circleInviteCancels.toList())
        awaitGone("addCircleSheet")
    }

    @Test
    fun joiningShowsTheTermsAndTheCodeThenAccepts() {
        core.circleJoinView = CircleInviteView(
            "j1", "sam@example.com", CircleRole.GUARDIAN, "Purchases", true, 14_400uL, false, "730215", CircleJoinStage.COMPARE,
        )
        openCircles()
        tap("circleJoin")
        rule.onNodeWithTag("joinLinkField").performTextInput("reins-circle:127.0.0.1:8000/abc")
        tap("joinOpen")
        awaitCore { core.circleJoinLinks.isNotEmpty() }
        assertEquals("reins-circle:127.0.0.1:8000/abc", core.circleJoinLinks.single())
        awaitTag("joinInviter")
        rule.onNodeWithTag("joinRole").assertTextContains("Guardian")
        rule.onNodeWithTag("joinScope").assertTextContains("Purchases")
        rule.onNodeWithTag("joinClass").assertTextContains("Summary · Deferred · 4 h")
        rule.onNodeWithTag("circleCode").assertTextContains("730 215")
        tap("joinAccept")
        awaitCore { core.circleJoinAccepts.isNotEmpty() }
        assertEquals(listOf("j1"), core.circleJoinAccepts.toList())
        awaitTag("joinAccepted")
    }

    @Test
    fun declineAnswersNo() {
        core.circleJoinView = CircleInviteView(
            "j1", "sam@example.com", CircleRole.DELEGATE, "Everything", false, 120uL, true, "730215", CircleJoinStage.COMPARE,
        )
        openCircles()
        tap("circleJoin")
        rule.onNodeWithTag("joinLinkField").performTextInput("reins-circle:127.0.0.1:8000/abc")
        tap("joinOpen")
        tap("joinDecline")
        awaitCore { core.circleJoinDeclines.isNotEmpty() }
        awaitTag("joinEnded")
        assertTrue(core.circleJoinAccepts.isEmpty())
    }

    @Test
    fun notALinkIsNotOpened() {
        openCircles()
        tap("circleJoin")
        rule.onNodeWithTag("joinLinkField").performTextInput("https://example.com")
        tap("joinOpen")
        awaitTag("joinCircleError")
        assertTrue(core.circleJoinLinks.isEmpty())
    }
}
