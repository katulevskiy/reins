package dev.reins.android

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.AutopilotNote
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** What the built apps showed: readable Activity rows, an entry that shows what was asked, clear choices, the badge. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PolishFlowTest : FlowHarness() {
    private val push = TestData.label("terminal", "run", "Run", "git push --force origin main", code = true)

    private fun pushEntry(autopilot: AutopilotNote? = null, decidedBy: String = "") = TestData.entry(
        7,
        action = "write",
        outcome = "denied",
        service = "desktop",
        op = "ask",
        opTitle = "Ask you on your phone",
        label = push,
        request = TestData.asked(
            "git push --force origin main \\\n  --no-verify",
            purpose = "Force-push the rebased branch so the PR updates",
            agent = "Claude Code",
            cwd = "~/dev/acme-web",
            why = "Rewriting git history",
            askedAt = 1_700_000_007 - 9,
        ),
        autopilot = autopilot,
        decidedBy = decidedBy,
    )

    /** A tag inside a row that merges its children (a tile in a tappable row). */
    private fun hasDeep(tag: String): Boolean {
        settle()
        return rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    @Test
    fun aRowShowsWhatWasTouchedWhatItDidAndTheRequestItself() {
        core.activity = listOf(
            pushEntry(),
            TestData.entry(8, action = "read", label = TestData.label("file", "read", "Read", "~/dev/api/.env", code = true)),
            TestData.entry(9, action = "send", label = TestData.label("gmail", "send", "Send an email to", "ada@acme.com")),
        )
        launch()
        awaitTag("entry:7")
        rule.onNodeWithTag("entryTitle:7", useUnmergedTree = true).assertTextEquals("Run git push --force origin main")
        rule.onNodeWithTag("entryTitle:8", useUnmergedTree = true).assertTextEquals("Read ~/dev/api/.env")
        rule.onNodeWithTag("entryTitle:9", useUnmergedTree = true).assertTextEquals("Send an email to ada@acme.com")
        assertTrue("a kind tile each", hasDeep("tile:7"))
        assertTrue(hasDeep("tile:8"))
        assertTrue(hasDeep("tile:9"))
        assertFalse("never the tool's generic title", showsText("Ask you on your phone", substring = true))
    }

    @Test
    fun aWaitingRequestReadsAsTheRequest() {
        core.pending = listOf(
            TestData.pending("req7", action = "write", service = "desktop", op = "ask", opTitle = "Ask you on your phone").copy(label = push),
        )
        launch()
        awaitTag("pending:req7")
        rule.onNodeWithTag("sentence:req7", useUnmergedTree = true).assertTextEquals("Run git push --force origin main")
        assertTrue("what it touches and what it does", showsText("TERMINAL · RUN"))
    }

    @Test
    fun anOlderEntryShowsWhatItAskedInFull() {
        core.activity = listOf(pushEntry())
        launch()
        tap("entry:7")
        awaitTag("entryHero")
        rule.onNodeWithTag("detailTitle", useUnmergedTree = true).assertTextEquals("Run git push --force origin main")
        rule.onNodeWithTag("askedWho").assertTextContains("Claude Code", substring = true)
        rule.onNodeWithTag("askedIn").assertTextContains("~/dev/acme-web", substring = true)
        rule.onNodeWithTag("askedWhy").assertTextContains("Rewriting git history", substring = true)
        rule.onNodeWithTag("askedWhen").assertTextContains("answered in 9 s", substring = true)
        rule.onNodeWithTag("fullRequestText", useUnmergedTree = true).assertTextEquals("git push --force origin main \\\n  --no-verify")
        rule.onNodeWithTag("fullRequestPurpose", useUnmergedTree = true)
            .assertTextEquals("# Claude Code: “Force-push the rebased branch so the PR updates”")
        tap("copyFullRequest")
        val clip = context.getSystemService(android.content.ClipboardManager::class.java).primaryClip
        assertEquals(
            "git push --force origin main \\\n  --no-verify\n\nForce-push the rebased branch so the PR updates",
            clip?.getItemAt(0)?.text?.toString(),
        )
    }

    @Test
    fun anEntryShowsItsApprovalAgainReadOnly() {
        core.activity = listOf(pushEntry(TestData.note(suggested = dev.reins.core.Verdict.DENY, pApprove = 0.09f), decidedBy = "autopilot"))
        core.entryApprovals[7L] = TestData.designPushView().copy(waitUntil = null, quick = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextContains("Denied by Autopilot", substring = true)
        // The approval's own facts and pages: the commits, then a commit.
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
        pressBack()
        pressBack()
        awaitTag("hero")
        // No decision dock: "This was wrong" is where it was, then what was asked.
        assertFalse(has("dock"))
        assertFalse(has("approve"))
        awaitTag("thisWasWrong")
        assertTrue(has("fullRequest"))
        rule.onNodeWithTag("thisWasWrong").assertIsDisplayed()
    }

    @Test
    fun aRecordedSearchShowsWhatWasSharedTickedAndCannotBeChanged() {
        val view = TestData.designMailView(null).copy(quick = null)
        core.activity = listOf(
            TestData.entry(
                5,
                info = TestData.info(null, listOf(dev.reins.core.ActivityMessage("m1", "", "Stripe", "Your invoice from Acme Inc.", 1)), null, null, null),
                label = TestData.label("gmail", "search", "Search Gmail for", "from:stripe.com"),
            ),
        )
        core.entryApprovals[5L] = view
        launch()
        tap("entry:5")
        awaitTag("entryApproval")
        awaitTag("message:m1")
        assertFalse("nothing to pick again", has("selectAll") || has("clearAll"))
        tap("check:m2")
        settle()
        assertEquals("a read-only page decides nothing", 0, core.approvals.size)
    }

    @Test
    fun theChosenChipIsTintedWithACheckAndTheOthersAreNot() {
        core.connections = listOf(TestData.connection("c1", "Claude"), TestData.connection("c2", "ChatGPT"))
        launch()
        tap("tabTrust")
        tap("newGrant")
        awaitTag("conn:c1")
        tap("conn:c2")
        rule.onNodeWithTag("conn:c2").assertIsSelected()
        rule.onNodeWithTag("conn:c1").assertIsNotSelected()
        tap("scope:all")
        rule.onNodeWithTag("scope:all").assertIsSelected()
        // All mail takes the place of the address box.
        awaitTag("allMailNote")
        assertFalse(has("parties"))
    }
}
