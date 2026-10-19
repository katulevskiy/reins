package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.DiffLineKind
import dev.reins.core.DiffLineView
import dev.reins.core.GitCommitView
import dev.reins.core.GitFileView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A push in Activity: the approval as it was shown, read-only (commits, a commit's files, a file's diff), told from the
 * entry's own outcome; and an entry whose approval is not on this phone, which shows its full request as text only.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PushInspectFlowTest : FlowHarness() {
    private val label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins")

    /** Text shaped like a push: on an entry without its approval it stays text, never commits or branches. */
    private val full = """
        git push katulevskiy/reins
        create branch signal/desktop-release: 3 commits
          1a2b3c4 Signal on the desktop
          5d6e7f8 Allowed by you
          9a0b1c2 Bump the version

        github_git_push
        repo: katulevskiy/reins
    """.trimIndent()

    private fun pushEntry(outcome: String = "released", decidedBy: String = "bypass") = TestData.entry(
        7, action = "write", outcome = outcome, service = "github", op = "git_push", opTitle = "Push to a branch",
        label = label, request = TestData.asked(full, agent = "git", askedAt = 1_700_000_007 - 3), decidedBy = decidedBy,
    )

    private fun deep(tag: String) = rule.onNodeWithTag(tag, useUnmergedTree = true)

    private fun hasDeep(tag: String): Boolean {
        settle()
        return rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    @Test
    fun anEntryWithItsApprovalOpensCommitsFilesAndTheDiffReadOnly() {
        core.activity = listOf(pushEntry())
        core.entryApprovals[7L] = TestData.designPushView().copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        deep("detailTitle").assertTextContains("Pushed 8 commits to", substring = true)
        deep("entryAutopilotTitle").assertTextEquals("Allowed during a bypass")
        deep("gitRef:0").assertTextEquals("feature/passkeys")
        assertTrue(hasDeep("gitTotals:0"))
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        assertFalse("history drilldowns have no waiting field", has("hero"))
        assertTrue("native Back stays available", has("entryBar"))
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        // Read-only: no decision anywhere, and nothing was decided by looking.
        assertFalse(has("dock"))
        assertFalse(has("approve"))
        assertFalse(has("deny"))
        assertTrue(core.approvals.isEmpty())
        assertTrue(core.denials.isEmpty())
    }

    @Test
    fun aDeniedEntryReadsAsAskedWhateverItsApprovalShows() {
        core.activity = listOf(pushEntry(outcome = "denied", decidedBy = ""))
        core.entryApprovals[7L] = TestData.designPushView().copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        deep("detailTitle").assertTextContains("Asked to push 8 commits to", substring = true)
        deep("entryAutopilotTitle").assertTextEquals("Denied by you")
        assertFalse(showsText("Pushed", substring = true))
    }

    @Test
    fun aBypassThatFailedNeverReadsAsAllowed() {
        core.activity = listOf(pushEntry(outcome = "error"))
        launch()
        tap("entry:7")
        awaitTag("entryAutopilot")
        deep("entryAutopilotTitle").assertTextEquals("Failed")
        deep("detailTitle").assertTextEquals("Asked to push 3 commits to katulevskiy/reins")
    }

    @Test
    fun whatTheApprovalShowsIsCleanedAndHiddenCharactersAreFlagged() {
        val bidi = "‮"
        val zw = "​"
        val file = GitFileView(
            "src/${bidi}evil.ts", "modified", 2u, 1u, false,
            listOf(
                DiffLineView(DiffLineKind.HUNK, null, null, "@@ -1,2 +1,3 @@$zw"),
                DiffLineView(DiffLineKind.DELETE, 1u, null, "  return ${bidi}deny()"),
                DiffLineView(DiffLineKind.ADD, null, 1u, "  return${zw} allow()\u0007"),
            ),
            false, true,
        )
        val commit = GitCommitView("1a2b3c4", "Fix ${bidi}login", "Ada${zw} <ada@example.com>", 1_700_000_000, 1u, listOf(file), 2u, 1u, true)
        val ref = TestData.gitRef("feat$zw/x$bidi", change = "create", commitCount = 3u, commits = listOf(commit), filesChanged = 1u, files = listOf(file), additions = 2u, deletions = 1u)
        core.activity = listOf(pushEntry())
        core.entryApprovals[7L] = TestData.gitPushView(TestData.gitPush(ref, notes = listOf("Note$bidi with${zw} marks"))).copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        deep("gitRef:0").assertTextEquals("feat/x")
        deep("gitNote:0").assertTextEquals("Note with marks")
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        deep("gitCommitsRef:0").assertTextEquals("feat/x")
        deep("commitRowSubject").assertTextEquals("Fix login")
        assertTrue("the Hidden warning stays", hasDeep("commitHidden"))
        // Commits gained but not listed say so.
        deep("gitCommitsNotListed:0").assertTextEquals("2 more not listed")
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
        assertTrue(hasDeep("hiddenChars"))
        deep("commitFile:0:path").assertTextEquals("src/evil.ts")
        assertTrue(hasDeep("commitFile:0:hidden"))
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        settle()
        val texts = rule.onAllNodes(androidx.compose.ui.test.hasTestTag("commitFile:0:diff"), useUnmergedTree = true).fetchSemanticsNodes()
            .flatMap { node -> node.children.flatMap { child -> textsOf(child) } }
        val shown = texts.joinToString("\n")
        assertTrue(shown, "−   return deny()" in shown)
        assertTrue(shown, "+   return allow() " in shown)
        listOf(bidi, zw, "\u0007").forEach { assertFalse("no hidden or control character reaches the diff", it in shown) }
    }

    private fun textsOf(node: androidx.compose.ui.semantics.SemanticsNode): List<String> =
        (node.config.getOrElseNullable(androidx.compose.ui.semantics.SemanticsProperties.Text) { null }?.map { it.text }.orEmpty()) +
            node.children.flatMap { textsOf(it) }

    @Test
    fun anEntryWithoutItsApprovalShowsItsRequestAsTextOnly() {
        core.activity = listOf(pushEntry(decidedBy = ""))
        launch()
        tap("entry:7")
        awaitTag("entryHero")
        deep("detailTitle").assertTextEquals("Pushed 3 commits to katulevskiy/reins")
        assertFalse("no branch line read out of the text", hasDeep("detailDetails"))
        assertFalse("no structured push made from the text", has("gitPush"))
        assertFalse(has("entryApproval"))
        assertTrue("the request in full stays", has("fullRequest"))
        deep("fullRequestText").assertTextContains("5d6e7f8 Allowed by you", substring = true)
        deep("entryAutopilotTitle").assertTextEquals("Allowed by you")
        // When is said once, in "What it asked".
        settle()
        assertEquals(1, rule.onAllNodesWithText("When", useUnmergedTree = true).fetchSemanticsNodes().size)
    }

    @Test
    fun anotherEntrysApprovalIsNeverShown() {
        core.activity = listOf(pushEntry(decidedBy = ""), pushEntry().copy(id = 8))
        core.entryApprovals[8L] = TestData.designPushView().copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryHero")
        assertFalse(has("entryApproval"))
        assertFalse(has("gitPush"))
    }

    @Test
    fun aFetchRowReadsAsOneVerb() {
        core.activity = listOf(
            TestData.entry(8, action = "read", outcome = "released", service = "github", op = "git_fetch", label = TestData.label("github", "read", "Clone and fetch", "katulevskiy/reins")),
        )
        launch()
        awaitTag("entry:8")
        deep("entryTitle:8").assertTextEquals("Fetched katulevskiy/reins")
    }
}
