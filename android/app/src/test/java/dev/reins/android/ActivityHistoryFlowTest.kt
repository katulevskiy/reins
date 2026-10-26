package dev.reins.android

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.text.TextLayoutResult
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A history record has a scrolling paper header; active requests retain the prominent decision field. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ActivityHistoryFlowTest : FlowHarness() {
    private fun deep(tag: String) = rule.onNodeWithTag(tag, useUnmergedTree = true)

    @Test
    fun aLongRecordedCommandScrollsWithItsFactsAndCanStillBeCopiedInFull() {
        val command = "JAVA_HOME=/opt/jdk ANDROID_HOME=/opt/android \\\n" +
            "./gradlew :app:assembleDebug --no-daemon --console=plain \\\n" +
            (1..12).joinToString(" \\\n") { "-Pbuild.option$it=development-value-$it" }
        val label = TestData.label("terminal", "run", "Run", command, code = true)
        core.activity = listOf(TestData.entry(7, action = "write", service = "desktop", op = "ask", label = label,
            request = TestData.asked(command, agent = "Claude Code", cwd = "/home/dana/dev/reins")))
        core.entryApprovals[7L] = TestData.askView(detail = "Command:\n$command\n\nIn: /home/dana/dev/reins", topic = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        rule.onNodeWithTag("entryIcon").assertIsDisplayed()
        deep("detailTitle").assertTextEquals("Ran a command in reins")
        deep("detailKind").assertTextContains("Terminal · Run", substring = true)
        deep("entryAutopilotTitle").assertTextEquals("Allowed by you")
        assertFalse("a record has no active field", has("hero"))
        assertFalse("a record has no countdown", has("waitLine") || has("lateBanner"))
        deep("fullRequestText").performScrollTo().assertTextEquals(command)
        rule.onNodeWithTag("entryIcon").assertIsNotDisplayed()
        tap("copyFullRequest")
        val clip = context.getSystemService(android.content.ClipboardManager::class.java).primaryClip
        assertEquals(command, clip?.getItemAt(0)?.text?.toString())
        tap("back")
        awaitTag("entry:7")
        assertTrue(core.approvals.isEmpty() && core.denials.isEmpty())
    }

    @Test
    fun aRecordedDenialKeepsTheRiskExplanationWarningsAndActualDecisionActor() {
        val why = "This force push replaces published history and may remove reviewed commits from the shared branch. ".repeat(3).trim()
        core.activity = listOf(TestData.entry(7, action = "write", outcome = "denied", service = "github", op = "git_push",
            label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins"), decidedBy = "autopilot",
            autopilot = TestData.note(pApprove = 0.04f, suggested = dev.reins.core.Verdict.DENY, risk = TestData.risk(why = why))))
        core.entryApprovals[7L] = TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)))
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        deep("detailTitle").assertTextContains("Asked to force-push", substring = true)
        deep("entryAutopilotTitle").assertTextContains("Denied by Autopilot", substring = true)
        deep("riskLine").performScrollTo().assertTextContains(why, substring = true)
        val layouts = mutableListOf<TextLayoutResult>()
        deep("riskLine").performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        assertFalse("the complete risk explanation wraps", layouts.single().hasVisualOverflow)
        assertTrue(has("forceWarning:0"))
        tap("whyAutopilot")
        awaitTag("whyDetail")
        assertTrue(showsText("Like 4 times you approved", substring = true))
        assertFalse(has("approve") || has("deny") || has("dock"))
        assertTrue(has("thisWasWrong"))
    }

    @Test
    fun anOlderRecordUsesTheSameHeaderAndKeepsItsUnabridgedSummary() {
        val summary = "A retained summary from an older request. ".repeat(12).trim()
        core.activity = listOf(TestData.entry(7, action = "search", outcome = "denied").copy(detail = summary))
        launch()
        tap("entry:7")
        awaitTag("entryHero")
        assertTrue(has("entryBar"))
        assertFalse(has("entryApproval") || has("hero"))
        deep("detailSummary").assertTextEquals(summary)
        val layouts = mutableListOf<TextLayoutResult>()
        deep("detailSummary").performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        assertFalse(layouts.single().hasVisualOverflow)
        deep("entryAutopilotTitle").assertTextEquals("Denied by you")
    }

    @Test
    fun aRecordedRequestStillLinksToTheGrantThatCoveredIt() {
        core.grants = listOf(TestData.grant("g1"))
        core.activity = listOf(TestData.entry(7, action = "write", service = "github", op = "git_push", grantId = "g1"))
        core.entryApprovals[7L] = TestData.designPushView()
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        tap("openGrantFromEntry")
        awaitTag("grantTitle")
    }

    @Test
    fun aPendingApprovalKeepsItsDecisionFieldAndControls() {
        openRequest(TestData.designPushView())
        awaitTag("hero")
        assertTrue(has("approve") && has("deny"))
        assertFalse(has("entryBar") || has("entryHero") || has("entryOutcome"))
    }
}
