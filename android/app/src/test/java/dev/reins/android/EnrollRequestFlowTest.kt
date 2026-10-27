package dev.reins.android

import android.os.Looper
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.core.EnrollView
import dev.reins.core.PhoneRole
import dev.reins.core.RosterMemberView
import dev.reins.core.RosterSummary
import java.time.Duration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * On the approving phone: a phone asking to view the account shows its name and, once it revealed, the code; Approve
 * is held and asks for the owner check, Deny is one tap, and Settings › Devices lists the second phones with Remove.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class EnrollRequestFlowTest : FlowHarness() {
    private fun openEnroll(code: String? = "482 193") {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.pending = listOf(TestData.joinItem())
        core.enrollViews["join1"] = EnrollView("join1", "Pixel 9", code, false, 1_700_000_200)
        launch(link("join", "join1"))
        awaitTag("enrollRequest")
    }

    @Test
    fun approvingNeedsTheCodeTheHoldAndTheOwner() {
        openEnroll()
        rule.onNodeWithTag("enrollTitle").assertTextEquals("Add a second phone")
        rule.onNodeWithTag("enrollDevice").assertTextEquals("“Pixel 9”")
        rule.onNodeWithTag("enrollCode").assertTextEquals("482 193")
        assertFalse("never shown as a request for the keys", has("joinCode"))
        hold("approve")
        awaitCore { core.enrollAnswers.isNotEmpty() }
        assertEquals("join1" to true, core.enrollAnswers.single())
        assertEquals(1, prompts.get())
        awaitText("“Pixel 9” sees this account now. It decides nothing.")
    }

    @Test
    fun noApprovalBeforeTheOtherPhoneShowsItsCode() {
        openEnroll(code = null)
        awaitTag("enrollWaiting")
        rule.onNodeWithTag("approve").assertIsNotEnabled()
        // It reveals: the code shows.
        core.enrollViews["join1"] = core.enrollViews["join1"]!!.copy(code = "482 193")
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(1_600))
        awaitTag("enrollCode")
        assertTrue(core.enrollAnswers.isEmpty())
    }

    @Test
    fun aCancelledOwnerCheckAnswersNothing() {
        authResult = AuthResult.Cancelled
        openEnroll()
        hold("approve")
        awaitCore { prompts.get() == 1 }
        assertTrue(core.enrollAnswers.isEmpty())
        assertTrue(has("enrollCode"))
    }

    @Test
    fun denyIsOneTapWithoutTheOwnerCheck() {
        openEnroll()
        tap("deny")
        awaitCore { core.enrollAnswers.isNotEmpty() }
        assertEquals("join1" to false, core.enrollAnswers.single())
        assertEquals(0, prompts.get())
    }

    @Test
    fun devicesListsSecondPhonesAndRemovesOneHeld() {
        core.setupRole = PhoneRole.PRIMARY
        core.setupRoster = RosterSummary(
            2u,
            PhoneRole.PRIMARY,
            listOf(RosterMemberView("this-phone", "Pixel 9", true, true), RosterMemberView("iphone", "iPhone", false, false)),
            "a1b2 c3d4",
            emptyList(),
        )
        launch()
        tap("tabSettings")
        tap("devicesRow")
        awaitTag("viewer:iphone")
        assertFalse("the approving phone is not listed as a second phone", has("viewer:this-phone"))
        tap("removeViewer:iphone")
        hold("confirmRemoveViewer")
        awaitCore { core.removedMembers.isNotEmpty() }
        assertEquals(listOf("iphone"), core.removedMembers.toList())
        awaitTag("noViewers")
    }
}
