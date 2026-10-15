package dev.reins.android

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.ActivityPush
import dev.reins.core.DiffLineKind
import dev.reins.core.DiffLineView
import dev.reins.core.GitCommitView
import dev.reins.core.GitFileView
import dev.reins.core.GitPushView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A push decided on another device: its commits and files came with the entry, its changes line by line did not. The
 * row and the sentence read the synced push; a commit's changes load from GitHub through this phone's own connection.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class OtherDevicePushFlowTest : FlowHarness() {
    private val label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins")

    /** The design's commit as GitHub returns it: its files with their changes. */
    private val full = TestData.designCommit()

    /** The same commit as synced with the entry: files and their counts, no changes line by line. */
    private val synced = full.copy(files = full.files.map { it.copy(diff = emptyList()) })

    private fun push(commit: GitCommitView = synced): GitPushView {
        val others = TestData.designPushView().git!!.refs[0].commits.subList(1, 3)
        val ref = TestData.gitRef("signal/desktop-release", change = "create", commitCount = 3u, commits = listOf(commit) + others, filesChanged = 5u, files = commit.files, additions = 199u, deletions = 87u)
        return GitPushView("katulevskiy/reins", 0u, emptyList(), listOf(ref), null)
    }

    private fun setUpEntry(git: GitPushView = push(), diffsNotKept: Boolean = true) {
        core.activity = listOf(
            TestData.entry(
                7, action = "write", outcome = "released", service = "github", op = "git_push", opTitle = "Push with git", label = label,
                request = TestData.asked("git push katulevskiy/reins", agent = "git").copy(git = git),
            ).copy(account = "katulevskiy"),
        )
        core.entryApprovals[7L] = TestData.gitPushView(git).copy(service = "github", waitUntil = null, quick = null)
        core.entryPushes[7L] = ActivityPush(git, !diffsNotKept, diffsNotKept, false)
    }

    private fun deep(tag: String) = rule.onNodeWithTag(tag, useUnmergedTree = true)

    private fun hasDeep(tag: String): Boolean {
        settle()
        return rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    private fun openCommit() {
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
    }

    @Test
    fun theRowReadsTheSyncedPush() {
        setUpEntry()
        launch()
        awaitTag("entry:7")
        deep("entryTitle:7").assertTextEquals("Pushed 3 commits to katulevskiy/reins")
        deep("entryDetails:7").assertTextEquals("signal/desktop-release · new branch")
    }

    @Test
    fun aCommitWhoseDiffWasNotKeptLoadsItFromGitHub() {
        setUpEntry()
        core.commitChanges[7L to "9f3c2a1"] = full
        openCommit()
        deep("loadChangesText").assertTextEquals("Diff not kept on this phone")
        assertFalse("nothing to open yet", hasDeep("commitFile:0:diff"))
        tap("loadChangesAction")
        awaitTag("commitFile:0")
        settle()
        assertFalse("the line goes once the changes are here", has("loadChanges"))
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        assertEquals(listOf(7L to "9f3c2a1"), core.commitChangeCalls.toList())
        assertTrue("loading decides nothing", core.approvals.isEmpty() && core.denials.isEmpty())
    }

    @Test
    fun itSpinsWhileLoadingAndSaysWhenItCouldNot() {
        setUpEntry()
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        core.commitChangeGate = gate
        openCommit()
        tap("loadChangesAction")
        rule.waitUntil(5_000) {
            settle()
            rule.onAllNodes(androidx.compose.ui.test.hasText("Loading from GitHub"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        assertFalse("no second load while one runs", has("loadChangesAction"))
        gate.complete(Unit)
        awaitTag("retryChanges")
        deep("loadChangesText").assertTextEquals("Couldn't load")
        assertTrue("the core's words behind the (?)", has("help:loadChanges"))
        core.commitChangeGate = null
        core.commitChanges[7L to "9f3c2a1"] = full
        tap("retryChanges")
        rule.waitUntil(5_000) { !has("loadChanges") }
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
    }

    @Test
    fun aLoadedCommitIsCleanedAndFlagged() {
        setUpEntry()
        val bidi = "‮"
        val file = GitFileView(
            "src/${bidi}login.ts", "modified", 1u, 1u, false,
            listOf(DiffLineView(DiffLineKind.DELETE, 1u, null, "a${bidi}b"), DiffLineView(DiffLineKind.ADD, null, 1u, "c​d")),
            false, true,
        )
        core.commitChanges[7L to "9f3c2a1"] = full.copy(subject = "Log in$bidi", files = listOf(file), hiddenChars = true)
        openCommit()
        tap("loadChangesAction")
        awaitTag("commitFile:0")
        deep("commitSubject").assertTextEquals("Log in")
        assertTrue(hasDeep("hiddenChars"))
        deep("commitFile:0:path").assertTextEquals("src/login.ts")
        assertTrue(hasDeep("commitFile:0:hidden"))
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        assertTrue(hasText("− ab"))
        assertTrue(hasText("+ cd"))
    }

    private fun hasText(text: String): Boolean {
        settle()
        return rule.onAllNodes(androidx.compose.ui.test.hasText(text), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    @Test
    fun aPushWithItsChangesHereLoadsNothing() {
        setUpEntry(push(full), diffsNotKept = false)
        openCommit()
        assertFalse(has("loadChanges"))
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        assertTrue(core.commitChangeCalls.isEmpty())
    }

    @Test
    fun theWaitingSheetNeverOffersToLoad() {
        val view = TestData.gitPushView(push()).copy(service = "github", waitUntil = 1_700_000_100 + 40)
        core.pending = listOf(TestData.pending(view.requestId, "write", 1u, waitUntil = view.waitUntil, label = view.connectionLabel, service = "github", op = view.op, opTitle = view.opTitle))
        core.approval = view
        launch(
            android.content.Intent(context, MainActivity::class.java)
                .setAction(dev.reins.android.platform.AppNotifier.ACTION_OPEN_ITEM)
                .putExtra(dev.reins.android.platform.AppNotifier.EXTRA_KIND, "request")
                .putExtra(dev.reins.android.platform.AppNotifier.EXTRA_ID, view.requestId),
        )
        awaitTag("gitPush")
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
        assertFalse(has("loadChanges"))
    }
}
