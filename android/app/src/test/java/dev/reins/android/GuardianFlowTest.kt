package dev.reins.android

import android.app.Notification
import android.app.NotificationManager
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Foreground
import dev.reins.core.CircleEvent
import dev.reins.core.CircleInviteView
import dev.reins.core.CircleJoinStage
import dev.reins.core.CircleRole
import dev.reins.core.GuardianDecisionView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Personal guardianship (Decisions v2 P7, 04 §4): the ward picks the notice before the code, leaves with a countdown
 * (a minor's leaving waits for the guardian's Allow), and the guardian's notices say which ones only the server reported.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class GuardianFlowTest : FlowHarness() {
    private fun openCircles() {
        Foreground.autoPopup = false
        launch()
        tap("tabSettings")
        tap("circlesRow")
    }

    /** A tag inside a row that reads as one button (its own node only in the unmerged tree). */
    private fun awaitRowTag(tag: String) {
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
    }

    /** This account is the ward of Mom on link g1. */
    private fun ward(view: dev.reins.core.WardView = TestData.wardView()) {
        core.circlePeopleList = core.circlePeopleList +
            TestData.circlePerson(view.linkId, view.guardian, role = CircleRole.GUARDIAN, mine = true, scopeLine = "Purchases")
        core.guardianWards[view.linkId] = view
    }

    @Test
    fun wardChoosesNoticeUpFront() {
        openCircles()
        tap("circleAdd")
        tap("role:guardian")
        tap("preset:money")
        // No default: the ward picks one before there is a link (and a code).
        awaitTag("notice:3")
        rule.onNodeWithTag("circleCreate").assertIsNotEnabled()
        tap("notice:7")
        rule.onNodeWithTag("noticeLine").assertTextContains("Ends after 7 days' notice")
        tap("notice:0")
        rule.onNodeWithTag("noticeLine").assertTextContains("Ends right away")
        tap("notice:3")
        rule.onNodeWithTag("noticeLine").assertTextContains("Ends after 3 days' notice")
        rule.onNodeWithTag("circleCreate").assertIsEnabled()
        tap("circleCreate")
        awaitCore { core.circleDrafts.isNotEmpty() }
        val draft = core.circleDrafts.single()
        assertEquals(CircleRole.GUARDIAN, draft.role)
        assertEquals(3.toUByte(), draft.noticeDays)
        awaitTag("circleQr")
    }

    @Test
    fun otherRolesHaveNoNotice() {
        openCircles()
        tap("circleAdd")
        tap("role:cosign")
        tap("preset:money")
        awaitTag("circleCreate")
        assertFalse(has("notice:3"))
        tap("circleCreate")
        awaitCore { core.circleDrafts.isNotEmpty() }
        assertEquals(null, core.circleDrafts.single().noticeDays)
    }

    @Test
    fun theGuardianSeesTheNoticeBeforeAccepting() {
        core.circleJoinView = CircleInviteView(
            "j1", "sam@example.com", CircleRole.GUARDIAN, "Purchases", true, 14_400uL, false, 3u, "730215", CircleJoinStage.COMPARE,
        )
        openCircles()
        tap("circleJoin")
        rule.onNodeWithTag("joinLinkField").performTextInput("reins-circle:127.0.0.1:8000/abc")
        tap("joinOpen")
        awaitTag("joinNotice")
        rule.onNodeWithTag("joinNotice").assertTextContains("Ends after 3 days' notice")
        assertTrue(has("joinAccept"))
    }

    @Test
    fun leavingShowsTheCountdown() {
        ward(
            TestData.wardView(
                decisions = listOf(GuardianDecisionView(1_699_990_000, "Buy headphones", false, "Not this month")),
            ),
        )
        openCircles()
        awaitTag("circlePerson:g1")
        // The ward can't pause its guardianship, and leaves from its screen, where the notice shows first.
        assertFalse(has("circlePause:g1"))
        assertFalse(has("circleEnd:g1"))
        tap("circleOpen:g1")
        awaitTag("guardianWard")
        rule.onNodeWithTag("guardianNotice").assertTextContains("Ends after 3 days' notice")
        rule.onNodeWithTag("guardianNote:0").assertTextContains("Not this month")
        tap("guardianLeave")
        awaitText("Leave Mom's guardianship?")
        assertTrue("nothing happens before the answer", core.circleEnds.isEmpty())
        tap("guardianLeaveConfirm")
        awaitCore { core.circleEnds.isNotEmpty() }
        assertEquals(listOf("g1"), core.circleEnds.toList())
        awaitTag("guardianEnding")
        rule.onNodeWithTag("guardianEnding").assertTextContains("Ends in 3 d")
        assertFalse(has("guardianLeave"))
        // Back on the list, the row counts down too.
        pressBack()
        awaitRowTag("circleEnding:g1")
        rule.onNodeWithTag("circleEnding:g1", useUnmergedTree = true).assertTextContains("Ends in 3 d")
    }

    @Test
    fun minorLeaveWaitsForConsent() {
        ward(TestData.wardView(minor = true))
        // This account also guards Sam, whose request to leave waits for it.
        core.circlePeopleList = core.circlePeopleList +
            TestData.circlePerson("g2", "sam@example.com", role = CircleRole.GUARDIAN, mine = false, scopeLine = "Purchases", needsConsent = true)
        core.guardianNoticeList = listOf(TestData.guardianNotice("gn1", "g2", needsConsent = true))
        openCircles()
        tap("circleOpen:g1")
        awaitTag("guardianMinor")
        tap("guardianLeave")
        awaitText("Mom must allow it.")
        tap("guardianLeaveConfirm")
        awaitTag("guardianWaiting")
        rule.onNodeWithTag("guardianWaiting").assertTextContains("Waiting for Mom")
        assertFalse(has("guardianEnding"))
        pressBack()
        awaitRowTag("circleEnding:g1")
        rule.onNodeWithTag("circleEnding:g1", useUnmergedTree = true).assertTextContains("Waiting for Mom")
        // The guardian's side: the request, and Allow.
        rule.onNodeWithTag("circleEnding:g2", useUnmergedTree = true).assertTextContains("Asked to leave")
        tap("circleOpen:g2")
        awaitTag("guardianConsent:gn1")
        assertTrue(showsText("Sam asked to leave. Allow?"))
        // Allow needs the owner check: a cancel consents to nothing.
        authResult = AuthResult.Cancelled
        val asked = prompts.get()
        tap("guardianAllow:gn1")
        awaitCore { prompts.get() > asked }
        rule.waitForIdle()
        assertTrue(core.guardianConsents.isEmpty())
        assertEquals("Let Sam leave", promptTexts.last().first)
        authResult = AuthResult.Success
        tap("guardianAllow:gn1")
        awaitCore { core.guardianConsents.isNotEmpty() }
        assertEquals(listOf("g2"), core.guardianConsents.toList())
        awaitGone("guardianConsent:gn1")
    }

    @Test
    fun unverifiedNoticesSayReportedByTheServer() {
        core.circlePeopleList = listOf(TestData.circlePerson("g2", "sam@example.com", role = CircleRole.GUARDIAN, mine = false))
        core.guardianNoticeList = listOf(
            TestData.guardianNotice("gn2", "g2", line = "Reset the account", verified = false, at = 1_700_000_050),
            TestData.guardianNotice("gn1", "g2", line = "Ended early", verified = true),
        )
        openCircles()
        tap("circleOpen:g2")
        awaitTag("guardianNotice:gn2")
        rule.onNodeWithTag("guardianNoticeLine:gn1").assertTextContains("Ended early")
        rule.onNodeWithTag("guardianUnverified:gn2").assertTextContains("Reported by the server")
        assertFalse("what this phone checked has no label", has("guardianUnverified:gn1"))
        // Nothing to allow here.
        assertFalse(has("guardianConsent:gn1"))
        tap("guardianProposeMinor")
        awaitCore { core.guardianMinorProposals.isNotEmpty() }
        assertEquals(listOf("g2"), core.guardianMinorProposals.toList())
        awaitTag("guardianMinorAsked")
    }

    @Test
    fun aGuardianNoticeIsOneNotification() {
        container.notifier.circleChanged(CircleEvent.GuardianNotice(TestData.guardianNotice("gn2", line = "Reset the account", verified = false)))
        val manager = context.getSystemService(NotificationManager::class.java)
        val posted = shadowOf(manager).activeNotifications.single().notification
        assertEquals("Sam: guardianship changed", posted.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals("Reset the account · Reported by the server", posted.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
        assertEquals(Notification.VISIBILITY_PRIVATE, posted.visibility)
    }

    @Test
    fun theWeeklySummaryIsTheWardsSwitch() {
        ward()
        openCircles()
        tap("circleOpen:g1")
        awaitTag("guardianDigest")
        assertTrue(showsText("Weekly summary for Mom"))
        tap("guardianDigest")
        awaitCore { core.guardianDigests.isNotEmpty() }
        assertEquals(listOf("g1" to true), core.guardianDigests.toList())
    }
}
