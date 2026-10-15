package dev.reins.android

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A push in Activity: its commits, a commit's files and a file's diff; and an entry whose approval is not here. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PushInspectFlowTest : FlowHarness() {
    private val label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins")

    private val full = """
        git push katulevskiy/reins
        create branch signal/desktop-release: 3 commits
          1a2b3c4 Signal on the desktop
          5d6e7f8 Release notes
          9a0b1c2 Bump the version

        github_git_push
        repo: katulevskiy/reins
    """.trimIndent()

    private fun pushEntry() = TestData.entry(
        7, action = "write", outcome = "released", service = "github", op = "git_push", opTitle = "Push to a branch",
        label = label, request = TestData.asked(full, agent = "git", askedAt = 1_700_000_007 - 3), decidedBy = "bypass",
    )

    @Test
    fun anEntryWithItsApprovalOpensCommitsFilesAndTheDiff() {
        core.activity = listOf(pushEntry())
        core.entryApprovals[7L] = TestData.designPushView().copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        rule.onNodeWithTag("gitRef:0", useUnmergedTree = true).assertTextEquals("feature/passkeys")
        settle()
        assertTrue(rule.onAllNodes(androidx.compose.ui.test.hasTestTag("gitTotals:0"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        assertFalse("not a second page of what was asked", has("keptPush"))
    }

    @Test
    fun anEntryWithoutItsApprovalShowsTheCommitsItKept() {
        core.activity = listOf(pushEntry())
        launch()
        tap("entry:7")
        awaitTag("entryAutopilot")
        rule.onNodeWithTag("detailTitle", useUnmergedTree = true).assertTextEquals("Pushed 3 commits to katulevskiy/reins")
        rule.onNodeWithTag("detailDetails", useUnmergedTree = true).assertTextEquals("signal/desktop-release · new branch")
        awaitTag("keptPush")
        rule.onNodeWithTag("keptRef:0", useUnmergedTree = true).assertTextEquals("signal/desktop-release")
        rule.onNodeWithTag("keptChip:0", useUnmergedTree = true).assertTextEquals("New branch")
        rule.onNodeWithTag("keptCommitsTitle:0", useUnmergedTree = true).assertTextEquals("3 commits")
        assertTrue(has("keptCommit:0:0"))
        assertTrue(has("keptCommit:0:2"))
        assertTrue("the request in full stays", has("fullRequest"))
        // When is said once, in "What it asked".
        settle()
        assertEquals(1, rule.onAllNodesWithText("When", useUnmergedTree = true).fetchSemanticsNodes().size)
    }

    @Test
    fun anOlderEntryWithoutAPushBlockShowsNothingMadeUp() {
        core.activity = listOf(pushEntry().copy(request = TestData.asked("github_git_push\nrepo: katulevskiy/reins")))
        launch()
        tap("entry:7")
        awaitTag("entryAutopilot")
        assertFalse(has("keptPush"))
        assertFalse(has("detailDetails"))
        assertTrue(has("fullRequest"))
    }

    @Test
    fun aFetchRowReadsAsOneVerb() {
        core.activity = listOf(
            TestData.entry(8, action = "read", outcome = "released", service = "github", op = "git_fetch", label = TestData.label("github", "read", "Clone and fetch", "katulevskiy/reins")),
        )
        launch()
        awaitTag("entry:8")
        rule.onNodeWithTag("entryTitle:8", useUnmergedTree = true).assertTextEquals("Fetched katulevskiy/reins")
    }
}
