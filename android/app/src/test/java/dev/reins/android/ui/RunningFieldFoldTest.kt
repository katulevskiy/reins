package dev.reins.android.ui

import dev.reins.android.ui.autopilot.FoldGeometry
import dev.reins.android.ui.autopilot.RunningFold
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The running field on Trust folds into a compact bar with the scroll: its progress and where its parts go. */
class RunningFieldFoldTest {
    // px at 1x: a 390 wide screen, a 36 status bar.
    private val sizes = FoldGeometry.Sizes(
        width = 390, top = 44, side = 24, gap = 12, labelHeight = 16, labelNaturalWidth = 180, clockWidth = 220, clockHeight = 60, heroHeight = 100,
        buttonCompactWidth = 120, openButton = 54, compactButton = 44, openBottom = 24, compactBottom = 12,
    )

    @Test
    fun theFoldFollowsTheScrollAndCrossFadesUnderReduceMotion() {
        assertEquals(0f, RunningFold.progress(0, 200, reduceMotion = false))
        assertEquals("continuous, not a snap", 0.25f, RunningFold.progress(50, 200, reduceMotion = false))
        assertEquals(1f, RunningFold.progress(900, 200, reduceMotion = false))
        assertEquals("open until measured", 0f, RunningFold.progress(100, 0, reduceMotion = false))
        assertEquals(0f, RunningFold.progress(99, 200, reduceMotion = true))
        assertEquals("Reduce Motion: one state or the other", 1f, RunningFold.progress(100, 200, reduceMotion = true))
        assertEquals(1f, RunningFold.heroAlpha(0f))
        assertEquals(0f, RunningFold.heroAlpha(0.45f))
        assertEquals("trustField:open", RunningFold.stateTag(0f))
        assertEquals("trustField:folding", RunningFold.stateTag(0.3f))
        assertEquals("trustField:compact", RunningFold.stateTag(1f))
    }

    @Test
    fun openTheButtonRunsAcrossUnderTheSentence() {
        val g = FoldGeometry.at(sizes, 0f)
        assertEquals(24, g.buttonX)
        assertEquals(342, g.buttonWidth)
        assertEquals(54, g.buttonHeight)
        assertEquals(1f, g.clockScale)
        assertTrue(g.heroY >= g.clockY + 60)
        assertTrue(g.buttonY >= g.heroY + 100)
        assertEquals(g.openHeight, g.surfaceHeight)
        assertEquals(g.buttonY + 54 + 24, g.openHeight)
    }

    @Test
    fun compactTheButtonSitsRightOfTheTimer() {
        val g = FoldGeometry.at(sizes, 1f)
        assertEquals("at the right edge", 390 - 24, g.buttonX + g.buttonWidth)
        assertEquals("its own width", 120, g.buttonWidth)
        assertEquals("still a full touch target", 44, g.buttonHeight)
        assertEquals("the label line stays", 44, g.labelY)
        assertEquals(FoldGeometry.CLOCK_SCALE, g.clockScale, 0.001f)
        val drawn = 60 * g.clockScale
        assertEquals("level with the timer", (g.buttonY + 22).toFloat(), g.clockY + drawn / 2, 1f)
        assertTrue("side by side", 24 + 220 * g.clockScale < g.buttonX)
        assertEquals(g.compactHeight, g.surfaceHeight)
        assertTrue(g.compactHeight < g.openHeight / 2)
        assertEquals("the room it keeps does not move", FoldGeometry.at(sizes, 0f).openHeight, g.openHeight)
    }

    @Test
    fun halfwayEverythingIsHalfwayThere() {
        val open = FoldGeometry.at(sizes, 0f)
        val compact = FoldGeometry.at(sizes, 1f)
        val mid = FoldGeometry.at(sizes, 0.5f)
        assertEquals((open.surfaceHeight + compact.surfaceHeight) / 2f, mid.surfaceHeight.toFloat(), 1f)
        assertEquals((open.buttonY + compact.buttonY) / 2f, mid.buttonY.toFloat(), 1f)
        assertEquals((open.buttonWidth + compact.buttonWidth) / 2f, mid.buttonWidth.toFloat(), 1f)
        assertEquals((1f + FoldGeometry.CLOCK_SCALE) / 2f, mid.clockScale, 0.001f)
    }

    @Test
    fun aLockdownFoldsItsButtonOntoTheLabelLine() {
        val g = FoldGeometry.at(sizes.copy(clockWidth = null, clockHeight = null), 1f)
        assertEquals(g.labelY + 8, g.buttonY + 22)
        assertTrue("the label gives the button room", 24 + g.labelWidth <= g.buttonX)
        assertEquals(44 + 44 + 12, g.compactHeight)
    }

    @Test
    fun aLabelTooLongToShareItsLineKeepsItAndTheButtonGoesUnder() {
        val g = FoldGeometry.at(sizes.copy(clockWidth = null, clockHeight = null, labelNaturalWidth = 230, buttonCompactWidth = 170), 1f)
        assertEquals("never cut", 342, g.labelWidth)
        assertEquals("at the right edge", 390 - 24, g.buttonX + g.buttonWidth)
        assertTrue("under the label", g.buttonY >= g.labelY + 16)
        assertEquals(44, g.buttonHeight)
    }

    @Test
    fun aLongTimerShrinksToFitBesideTheButton() {
        val g = FoldGeometry.at(sizes.copy(clockWidth = 600), 1f)
        assertTrue(24 + 600 * g.clockScale <= g.buttonX - 12 + 1)
    }
}
