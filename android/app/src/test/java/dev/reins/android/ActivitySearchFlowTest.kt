package dev.reins.android

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.activity.localDay
import dev.reins.core.EmailView
import java.util.TimeZone
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Activity by day (Today open, the rest closed until tapped) and its search, highlights and snippets. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ActivitySearchFlowTest : FlowHarness() {
    private val zone: TimeZone = TimeZone.getDefault()
    private val now = 1_700_000_100L // 22:15 UTC
    private val today = localDay(now, java.time.ZoneOffset.UTC)

    @Before
    fun utc() {
        TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
        val mail = TestData.info(null, emptyList(), EmailView(listOf("Ann <ann@corp.com>"), emptyList(), "Q3 numbers", "Please find the quarterly invoice attached."), null, null)
        core.activity = listOf(
            TestData.entry(10, "send", count = 1u, at = now - 600, info = mail),
            TestData.entry(9, "search", at = now - 1_200, info = TestData.info("invoice", emptyList(), null, null, null)),
            TestData.entry(5, "search", at = now - 86_400, info = TestData.info("invoice march", emptyList(), null, null, null), outcome = "denied"),
            TestData.entry(4, "read", at = now - 86_400 - 60),
            TestData.entry(2, "read", at = now - 3 * 86_400),
        )
    }

    @After
    fun restoreZone() {
        TimeZone.setDefault(zone)
    }

    @Test
    fun todayIsOpenAndTheDaysBeforeAreClosedWithTheirCounts() {
        launch()
        awaitTag("day:$today")
        awaitTag("entry:10")
        assertTrue(has("dayEntries:$today"))
        rule.onNodeWithTag("daySummary:${today - 1}", useUnmergedTree = true).assertTextContains("2 · 1 denied")
        rule.onNodeWithTag("daySummary:$today", useUnmergedTree = true).assertTextContains("2")
        assertTrue(has("day:${today - 3}"))
        assertFalse(has("entry:5"))
        assertFalse(has("dayEntries:${today - 1}"))
        // The header is a heading that says whether it is open.
        val header = rule.onNodeWithTag("day:${today - 1}").fetchSemanticsNode().config
        assertEquals("Collapsed", header.getOrNull(SemanticsProperties.StateDescription))
        assertTrue(header.contains(SemanticsProperties.Heading))
    }

    @Test
    fun aDayOpensClosesAndStaysSoAcrossTabs() {
        launch()
        awaitTag("day:${today - 1}")
        tap("day:${today - 1}")
        awaitTag("entry:5")
        assertTrue(has("entry:4"))
        assertTrue(has("dayEntries:${today - 1}"))
        // Its entries were read from the core, for that local day only.
        assertTrue(core.activityQueries.any { it.since == now - 86_400 - 22 * 3_600 - 15 * 60 && it.until == it.since!! + 86_400 })
        tap("day:${today - 1}")
        awaitGone("entry:5")
        tap("day:${today - 1}")
        awaitTag("entry:5")
        // Today closed by hand.
        tap("day:$today")
        awaitGone("entry:10")

        tap("tabTrust")
        awaitGone("activityList")
        tap("tabActivity")
        awaitTag("entry:5")
        assertFalse(has("entry:10"))
        assertEquals("Expanded", rule.onNodeWithTag("day:${today - 1}").fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription))
    }

    @Test
    fun aSearchShowsWhatMatchesSetApartWithASnippet() {
        launch()
        awaitTag("activitySearch")
        rule.onNodeWithTag("activitySearch").performTextInput("invoice")
        awaitTag("searchResults")
        awaitTag("entry:5")
        assertTrue(has("entry:9"))
        assertTrue(has("entry:10"))
        assertFalse(has("entry:4"))
        assertEquals("invoice", core.activityQueries.last().text)
        // Results run by day, yesterday's too, with no headers to open.
        assertFalse(has("day:${today - 1}"))
        assertTrue(showsText("3 found"))
        // The match in the title is set apart.
        val title = rule.onNodeWithTag("entryTitle:9", useUnmergedTree = true).fetchSemanticsNode().config[SemanticsProperties.Text].first()
        val hit = title.spanStyles.single { it.item.background != Color.Unspecified }
        assertEquals("invoice", title.text.substring(hit.start, hit.end).lowercase())
        // The email's body matched, not its row: one line says where.
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag("snippet:10"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag("snippet:10", useUnmergedTree = true).assertTextContains("BODY", substring = true)
        rule.onNodeWithTag("snippet:10", useUnmergedTree = true).assertTextContains("invoice", substring = true)
        assertTrue(rule.onAllNodes(hasTestTag("snippet:9"), useUnmergedTree = true).fetchSemanticsNodes().isEmpty())

        // A filter narrows it: denied only.
        tap("filters")
        tap("filter:outcome")
        tap("outcome:denied")
        awaitGone("entry:9")
        assertTrue(has("entry:5"))
        assertEquals(listOf(dev.reins.core.ActivityOutcome.DENIED), core.activityQueries.last().outcomes)

        // Nothing found.
        rule.onNodeWithTag("activitySearch").performTextInput("zzz")
        awaitTag("noResults")

        // Clearing both goes back to the days.
        tap("activitySearchClear")
        tap("filtersClear")
        awaitTag("day:$today")
        awaitTag("entry:10")
        assertFalse(has("searchResults"))
    }

    @Test
    fun theFiltersAloneSearchToo() {
        launch()
        awaitTag("entry:10")
        tap("filters")
        tap("filter:when")
        tap("when:today")
        awaitTag("searchResults")
        assertTrue(has("entry:10"))
        assertFalse(has("entry:5"))
        tap("filter:who")
        tap("who:autopilot")
        awaitTag("noAutomatic")
    }
}
