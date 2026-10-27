package dev.reins.android

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.activity.localDay
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

/**
 * The app reads only the newest entries at once (`AppContainer.ACTIVITY_LIMIT`); a day of Bypass logs hundreds. An entry
 * from an older day, beyond those, still opens: its page and its emails are read from the core by its id.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ActivityOlderEntriesFlowTest : FlowHarness() {
    private val zone: TimeZone = TimeZone.getDefault()
    private val now = 1_700_000_100L // 22:15 UTC
    private val today = localDay(now, java.time.ZoneOffset.UTC)
    private val old = today - 3

    @Before
    fun seed() {
        TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
        val limit = 300 // AppContainer.ACTIVITY_LIMIT
        // Today: more entries than the history read at once holds, newest first; three days ago: 20 more.
        val recent = (limit + 20 downTo 21).map { id -> TestData.entry(id.toLong(), "send", at = now - (limit + 20 - id)) }
        val older = (20 downTo 1).map { id ->
            val info = if (id == 5) TestData.info(null, TestData.messages("Your March statement is ready"), null, null, null) else TestData.info(null, emptyList(), null, null, null)
            TestData.entry(id.toLong(), "read", count = 1u, at = now - 3 * 86_400 - (20 - id), info = info)
        }
        core.activity = recent + older
        core.emails["m0"] = dev.reins.core.EmailContent(
            "m0", "Bank <alerts@bank.com>", listOf("me@gmail.com"), emptyList(), "Your March statement is ready", 1_700_000_000,
            "Your statement is attached.",
        )
    }

    @After
    fun restoreZone() {
        TimeZone.setDefault(zone)
    }

    private fun openOlderDay() {
        // Everything seen, so the river opens at its top rather than at the oldest unseen entry.
        container.state.setSeenActivityId(Long.MAX_VALUE)
        launch()
        tap("day:$today") // closed, so the older day is on screen
        tap("day:$old")
        // Its entries come from the core: scrolled to once there.
        rule.waitUntil(10_000) {
            runCatching { rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:5")) }.isSuccess
        }
        awaitTag("entry:5")
    }

    @Test
    fun anEntryFromAnOlderDayBeyondTheNewestOpensItsDetail() {
        openOlderDay()
        assertTrue("the history read at once stops before it", container.state.activity.value.none { it.id == 5L })
        tap("entry:5")
        awaitTag("entryHero")
        assertFalse(has("entryGone"))
        rule.onNodeWithTag("detailSummary", useUnmergedTree = true).assertTextEquals("detail of 5")
        assertTrue(core.entryLookups.contains(5L))
    }

    @Test
    fun anEmailOfAnOlderEntryOpensInFull() {
        openOlderDay()
        tap("entry:5")
        tap("emailsShared")
        tap("detailMessage:0")
        awaitTag("emailBody")
        assertFalse(has("emailGone"))
        assertEquals(listOf("me@gmail.com" to "m0"), core.openedEmails.toList())
    }

    @Test
    fun anEntryTheCoreNoLongerHasIsNotFound() {
        openOlderDay()
        core.activity = core.activity.filterNot { it.id == 5L }
        tap("entry:5")
        awaitTag("entryGone")
        assertFalse(has("entryHero"))
    }
}
