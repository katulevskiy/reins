package dev.reins.android

import androidx.compose.foundation.layout.Box
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.countText
import dev.reins.android.ui.main.ReinsTabBar
import dev.reins.android.ui.nav.Tab
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The count on the Activity tab: whole for one to three digits, inside the bar, never clipped by the tab's pill. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class NavBadgeTest {
    @get:Rule val rule = createComposeRule()

    @Test
    fun countsUpToNinetyNineThenNinetyNinePlus() {
        assertEquals(listOf("1", "12", "99", "99+"), listOf(1, 12, 99, 120).map(::countText))
    }

    @Test
    fun theBadgeIsWholeForOneTwoAndThreeDigits() {
        var count by androidx.compose.runtime.mutableIntStateOf(1)
        rule.setContent {
            CompositionLocalProvider(LocalColors provides RColors.Dark) {
                Box { ReinsTabBar(Tab.Trust, activityCount = count, bypassOn = false, onSelect = {}) }
            }
        }
        for ((n, shown) in listOf(1 to "1", 12 to "12", 120 to "99+")) {
            count = n
            rule.waitForIdle()
            val badge = rule.onNodeWithTag("tabActivity:count", useUnmergedTree = true)
            badge.assertIsDisplayed()
            val b = badge.fetchSemanticsNode()
            val label = rule.onNodeWithText(shown, useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
            assertTrue("the number is whole inside its badge: $label in ${b.boundsInRoot}", b.boundsInRoot.contains(label.center) && label.width <= b.boundsInRoot.width)
            val tab = rule.onNodeWithTag("tabActivity").fetchSemanticsNode().boundsInRoot
            val r = b.boundsInRoot
            assertTrue("inside the tab: $r in $tab", r.left >= tab.left && r.right <= tab.right)
            assertTrue("at least as wide as tall: $r", r.width >= r.height - 0.5f)
            assertTrue("above the bar's edge by at most its ring: $r vs $tab", r.top >= tab.top - 9f * rule.density.density)
        }
    }
}
