package dev.reins.android

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.AttentionLevel
import dev.reins.core.MessageView
import dev.reins.core.ReasonCode
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Items worth a closer look, on the approval sheet and in Activity: flagged commits first, each opening its own commit;
 * the summary on the folded row; one chip on a code's row; and ticks as before (only a code starts unticked).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class AttentionFlowTest : FlowHarness() {
    private fun hasDeep(tag: String): Boolean {
        settle()
        return rule.onAllNodes(hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    private fun top(tag: String) = rule.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot.top

    private fun spoken(tag: String) =
        rule.onNodeWithTag(tag).fetchSemanticsNode().config.getOrNull(SemanticsProperties.ContentDescription).orEmpty().joinToString(" ")

    private fun checkFlaggedCommits() {
        assertTrue(hasDeep("gitAttention:0"))
        assertTrue(showsText("2 of 8 to look at"))
        tap("gitCommits:0")
        awaitTag("gitCommit:0:5")
        // Close (the CI change, 6th in the push), then Look (the deleted file, 3rd), then the rest in order.
        assertTrue(top("gitCommit:0:5") < top("gitCommit:0:2"))
        assertTrue(top("gitCommit:0:2") < top("gitCommit:0:0"))
        assertTrue(top("gitCommit:0:0") < top("gitCommit:0:1"))
        // TalkBack reads how closely and why.
        assertTrue(spoken("gitCommit:0:5"), "Look closer: Changes CI" in spoken("gitCommit:0:5"))
        assertTrue(spoken("gitCommit:0:2"), "Worth a look: Deletes a file" in spoken("gitCommit:0:2"))
        // The first row on screen opens the commit it shows, by its place in the push.
        tap("gitCommit:0:5")
        awaitTag("commitSubject")
        rule.onNodeWithTag("commitSubject", useUnmergedTree = true).assertTextEquals("Run the release on every tag")
        assertTrue(hasDeep("commitFile:0:attention"))
    }

    @Test
    fun flaggedCommitsComeFirstAndOpenTheirOwnCommit() {
        openRequest(TestData.attentionPushView())
        awaitTag("gitCommits:0")
        checkFlaggedCommits()
    }

    @Test
    fun anActivityPushShowsTheSameMarks() {
        core.activity = listOf(
            TestData.entry(
                7, action = "write", outcome = "released", service = "github", op = "git_push", opTitle = "Push to a branch",
                label = TestData.label("github", "push", "Push 8 commits to", "octo/app"),
            ),
        )
        core.entryApprovals[7L] = TestData.attentionPushView().copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        checkFlaggedCommits()
    }

    @Test
    fun aForcePushSaysItRewritesHistoryWithNothingFlagged() {
        openRequest(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)), noStanding = true))
        awaitTag("gitCommits:0")
        assertTrue(hasDeep("gitRewrites:0"))
        assertFalse(hasDeep("gitAttention:0"))
    }

    @Test
    fun flaggedEmailsStayTickedAndACodeHasOneChip() {
        val code = MessageView(
            "code", "Acme <no-reply@acme.example>", "Your sign-in code", 1_700_000_000, "Your code is 548219", false, true,
            TestData.attention(AttentionLevel.CLOSE, ReasonCode.SIGN_IN_CODE to "Looks like a sign-in code"),
        )
        val view = TestData.attentionSearchView().let { it.copy(messages = it.messages + code, count = 13u) }
        openRequest(view)
        awaitTag("check:n11")
        // A section line, in sentence case.
        assertTrue(showsText("13 found · 3 to look at"))
        // A bank or a reset email starts ticked, with its chip; a code does not.
        rule.onNodeWithTag("check:n11").assertIsOn()
        rule.onNodeWithTag("check:n10").assertIsOn()
        rule.onNodeWithTag("check:code").assertIsOff()
        assertTrue(hasDeep("attention:n10"))
        assertTrue(hasDeep("attention:n11"))
        // One chip on the code's row, with the word it always had.
        assertTrue(hasDeep("sensitive:code"))
        assertFalse(hasDeep("attention:code"))
        assertTrue(spoken("check:code"), "Look closer: Code" in spoken("check:code"))
        assertFalse(spoken("check:code"), "sign-in" in spoken("check:code"))
        // Flagged first: the code and the reset (Close), the invoice (Look), then the rest.
        assertTrue(top("check:code") < top("check:n11"))
        assertTrue(top("check:n10") < top("check:n11"))
        assertTrue(top("check:n11") < top("check:n0"))
    }
}
