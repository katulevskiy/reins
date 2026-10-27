package dev.reins.android

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Foreground
import dev.reins.core.AutoQuestion
import dev.reins.core.FactChoice
import dev.reins.core.MatrixView
import dev.reins.core.Verdict
import dev.reins.core.ZoneChoice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** "Auto needs information" cards with the waiting items, and the matrix's line and "Ask me next time" on entries. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class QuestionCardsFlowTest : FlowHarness() {
    private fun question(kind: FactChoice, key: String, service: String, at: Long) =
        AutoQuestion("$service:$key", kind, key, service, "Claude Code", "req-$key", at, ZoneChoice.OUTSIDE)

    private val octo = question(FactChoice.OWNER, "octo", "github", 1_700_000_050)
    private val bob = question(FactChoice.PERSON, "bob@acme.io", "email", 1_700_000_040)
    private val eve = question(FactChoice.PERSON, "eve@evil.test", "email", 1_700_000_030)

    private fun deep(tag: String) = rule.onNode(hasTestTag(tag), useUnmergedTree = true)

    @Test
    fun questionCardAnswers() {
        core.autoQuestionList = listOf(octo, bob, eve)
        launch()
        awaitTag("question:github:octo")
        deep("questionTitle:github:octo").assertTextEquals("Who is octo on GitHub?")
        deep("questionTitle:email:bob@acme.io").assertTextEquals("Who is bob@acme.io?")
        deep("questionWho:github:octo").assertTextEquals("Claude Code")
        // Outside needs no owner check.
        tap("answer:email:bob@acme.io:OUTSIDE")
        awaitCore { core.answeredQuestions.isNotEmpty() }
        assertEquals("email:bob@acme.io" to ZoneChoice.OUTSIDE, core.answeredQuestions.single())
        assertEquals(0, prompts.get())
        awaitGone("question:email:bob@acme.io")
        // Anything else loosens: refused, the card comes back and nothing is recorded.
        authResult = AuthResult.Cancelled
        tap("answer:github:octo:WORK")
        awaitCore { prompts.get() == 1 }
        awaitTag("question:github:octo")
        assertEquals(1, core.answeredQuestions.size)
        authResult = AuthResult.Success
        tap("answer:github:octo:WORK")
        awaitCore { core.answeredQuestions.size == 2 }
        assertEquals("github:octo" to ZoneChoice.WORK, core.answeredQuestions.last())
        assertEquals("Treat octo as Work", core.autoOwnerReasons.last())
        awaitGone("question:github:octo")
        // A dismiss: it stays Outside and isn't asked again.
        tap("dismissQuestion:email:eve@evil.test")
        awaitCore { core.dismissedQuestions.isNotEmpty() }
        assertEquals(listOf("email:eve@evil.test"), core.dismissedQuestions.toList())
        awaitGone("questionCards")
    }

    @Test
    fun questionCardsNeverNotify() {
        Foreground.focused = false
        core.autoQuestionList = listOf(octo, bob)
        launch()
        awaitTag("question:github:octo")
        // New questions arrive with the next read of the waiting items, still without a notification.
        core.autoQuestionList = listOf(eve, octo, bob)
        container.refreshAfterAnswer()
        awaitTag("question:email:eve@evil.test")
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        assertEquals(0, shadowOf(manager).allNotifications.size)
    }

    @Test
    fun anAutoEntryShowsTheMatrixLineAndAsksNextTime() {
        val line = MatrixView(Verdict.APPROVE, false, ZoneChoice.MINE, 0u, "Auto · Mine · Undoable", false)
        core.activity = listOf(
            TestData.entry(7, action = "write", service = "github", op = "git_push", decidedBy = "autopilot",
                autopilot = TestData.note().copy(matrix = line)),
            TestData.entry(8, action = "write", service = "github", op = "git_push",
                autopilot = TestData.note(dev.reins.core.AutopilotMode.MANUAL).copy(matrix = line.copy(shadow = true))),
        )
        core.entryApprovals[7L] = TestData.gitPushView()
        core.entryApprovals[8L] = TestData.gitPushView().copy(requestId = "req21")
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        deep("matrixLineText").performScrollTo().assertTextEquals("Auto · Mine · Undoable")
        tap("askMeNextTime")
        awaitCore { core.askedNextTime.isNotEmpty() }
        assertEquals(listOf("req20"), core.askedNextTime.toList())
        awaitTag("askedNextTime")
        assertEquals("stricter only: no owner check", 0, prompts.get())
        tap("back")
        tap("entry:8")
        awaitTag("entryApproval")
        deep("matrixLineText").performScrollTo().assertTextEquals("Would be: Auto · Mine · Undoable")
        assertFalse("only what Auto decided", has("askMeNextTime"))
        assertTrue(core.askedNextTime.size == 1)
    }
}
