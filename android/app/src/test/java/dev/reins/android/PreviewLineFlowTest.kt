package dev.reins.android

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.CellChoice
import dev.reins.core.FactChoice
import dev.reins.core.MatrixView
import dev.reins.core.ModelState
import dev.reins.core.PolicyChange
import dev.reins.core.Verdict
import dev.reins.core.WorldItem
import dev.reins.core.ZoneChoice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A change to Auto, previewed on this phone's own last 30 days before it's made (Decisions v2, 02 §4). */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PreviewLineFlowTest : FlowHarness() {
    private fun deep(tag: String) = rule.onNode(hasTestTag(tag), useUnmergedTree = true)

    private fun openMatrix() {
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u)
        core.autoSetupDone = false
        core.world = listOf(WorldItem(FactChoice.OWNER, "acme", "acme", "github", ZoneChoice.WORK, true, 6u))
        launch()
        tap("tabTrust")
        awaitTag("mode:MANUAL")
        tap("mode:AUTO")
        awaitTag("autoSetup")
        tap("autoSetupNext")
        awaitTag("matrix")
    }

    @Test
    fun loosenSheetShowsThePreview() {
        core.previewLine = "Last 30 days: 0 Auto → Ask, 7 Ask → Auto"
        openMatrix()
        // Work L1 is Ask: the tap makes it Auto, looser. The sheet shows what that would have done, nothing changes yet.
        tap("cell:WORK:1")
        awaitTag("loosenConfirm")
        assertTrue(showsText("Last 30 days: 0 Auto → Ask, 7 Ask → Auto"))
        assertEquals(listOf<PolicyChange>(PolicyChange.SetCell(ZoneChoice.WORK, 1u, CellChoice.AUTO)), core.previewCalls.toList())
        assertTrue(core.matrixCellCalls.isEmpty())
        assertEquals(0, prompts.get())
        // Continue: the owner's check, then the change.
        tap("loosenConfirm")
        awaitCore { core.matrixCellCalls.isNotEmpty() }
        assertEquals(Triple(ZoneChoice.WORK, 1.toUByte(), CellChoice.AUTO), core.matrixCellCalls.single())
        assertEquals(1, prompts.get())
        // Stricter needs no preview: Work L1 Auto → Ask first.
        tap("cell:WORK:1")
        awaitCore { core.matrixCellCalls.size == 2 }
        assertEquals(1, core.previewCalls.size)
    }

    @Test
    fun askMeNextTimeShowsItsPreview() {
        core.previewLine = "Last 30 days: 5 Auto → Ask, 0 Ask → Auto"
        val line = MatrixView(Verdict.APPROVE, false, ZoneChoice.MINE, 0u, "Auto · Mine · Undoable", false)
        core.activity = listOf(
            TestData.entry(7, action = "write", service = "github", op = "git_push", decidedBy = "autopilot",
                autopilot = TestData.note().copy(matrix = line)),
        )
        core.entryApprovals[7L] = TestData.gitPushView()
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        deep("previewLineText").performScrollTo().assertTextEquals("Last 30 days: 5 Auto → Ask, 0 Ask → Auto")
        assertEquals(listOf<PolicyChange>(PolicyChange.AskMeNextTime("req20")), core.previewCalls.toList())
    }
}
