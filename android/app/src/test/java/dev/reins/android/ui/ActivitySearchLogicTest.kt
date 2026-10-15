package dev.reins.android.ui

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import dev.reins.android.state.ActivitySearch
import dev.reins.android.state.ActivityWindow
import dev.reins.android.ui.activity.DayCount
import dev.reins.android.ui.activity.askTitle
import dev.reins.android.ui.activity.chipText
import dev.reins.android.ui.activity.dayOpen
import dev.reins.android.ui.activity.dayQuery
import dev.reins.android.ui.activity.dayStart
import dev.reins.android.ui.activity.daySummary
import dev.reins.android.ui.activity.dayTitle
import dev.reins.android.ui.activity.daysOf
import dev.reins.android.ui.activity.deciderKind
import dev.reins.android.ui.activity.localDay
import dev.reins.android.ui.activity.matchRanges
import dev.reins.android.ui.activity.outcomeKind
import dev.reins.android.ui.activity.query
import dev.reins.android.ui.activity.snippetString
import dev.reins.core.ActivityDecider
import dev.reins.core.ActivityField
import dev.reins.core.ActivityHighlight
import dev.reins.core.ActivityOutcome
import dev.reins.core.ActivitySnippet
import dev.reins.core.ActivityStamp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Activity by day and its search: grouping, counts, the query, and what a title sets apart. */
class ActivitySearchLogicTest {
    private val newYork = ZoneId.of("America/New_York")
    private val kolkata = ZoneId.of("Asia/Kolkata")

    private fun at(iso: String) = Instant.parse(iso).epochSecond

    private fun day(date: String) = LocalDate.parse(date).toEpochDay()

    private fun stamp(id: Long, at: Long, outcome: ActivityOutcome = ActivityOutcome.APPROVED) =
        ActivityStamp(id, at, outcome, ActivityDecider.YOU)

    // ---- days -----------------------------------------------------------------------------------------------------

    @Test
    fun aDayIsTheLocalCalendarDayAcrossMidnight() {
        // 23:59:59 and 00:00:00 local, a second apart, fall on two days.
        assertEquals(day("2026-03-05"), localDay(at("2026-03-06T04:59:59Z"), newYork))
        assertEquals(day("2026-03-06"), localDay(at("2026-03-06T05:00:00Z"), newYork))
        assertEquals(day("2026-03-08"), localDay(at("2026-03-08T18:29:59Z"), kolkata))
        assertEquals(day("2026-03-09"), localDay(at("2026-03-08T18:30:00Z"), kolkata))
        // The same instant is another day in another zone.
        assertEquals(day("2026-03-08"), localDay(at("2026-03-09T03:00:00Z"), newYork))
        assertEquals(day("2026-03-09"), localDay(at("2026-03-09T03:00:00Z"), kolkata))
    }

    @Test
    fun theDaylightSavingDayIsTwentyThreeHoursAndGroupsRight() {
        // New York springs forward at 02:00 on 2026-03-08: that day starts at 05:00Z (EST) and the next at 04:00Z (EDT).
        val dst = day("2026-03-08")
        assertEquals(at("2026-03-08T05:00:00Z"), dayStart(dst, newYork))
        assertEquals(at("2026-03-09T04:00:00Z"), dayStart(dst + 1, newYork))
        assertEquals(23 * 3_600L, dayStart(dst + 1, newYork) - dayStart(dst, newYork))
        val q = dayQuery(dst, newYork)
        assertEquals(at("2026-03-08T05:00:00Z"), q.since)
        assertEquals(at("2026-03-09T04:00:00Z"), q.until)
        // 23:59:59 EDT on the 8th is still the 8th; midnight is the 9th; 23:59:59 EST on the 7th is the 7th.
        assertEquals(dst, localDay(at("2026-03-09T03:59:59Z"), newYork))
        assertEquals(dst + 1, localDay(at("2026-03-09T04:00:00Z"), newYork))
        assertEquals(dst - 1, localDay(at("2026-03-08T04:59:59Z"), newYork))
        assertEquals(dst, localDay(at("2026-03-08T07:30:00Z"), newYork)) // 03:30 EDT, after the skipped hour
        val days = daysOf(
            listOf(
                stamp(5, at("2026-03-09T04:00:00Z")),
                stamp(4, at("2026-03-09T03:59:59Z"), ActivityOutcome.DENIED),
                stamp(3, at("2026-03-08T07:30:00Z")),
                stamp(2, at("2026-03-08T05:00:00Z"), ActivityOutcome.FAILED),
                stamp(1, at("2026-03-08T04:59:59Z")),
            ),
            newYork,
        )
        assertEquals(
            listOf(DayCount(dst + 1, 1), DayCount(dst, 3, denied = 1, failed = 1), DayCount(dst - 1, 1)),
            days,
        )
    }

    @Test
    fun daysCountInKolkata() {
        val days = daysOf(
            listOf(
                stamp(3, at("2026-03-08T18:30:00Z"), ActivityOutcome.DENIED),
                stamp(2, at("2026-03-08T18:29:59Z"), ActivityOutcome.DENIED),
                stamp(1, at("2026-03-08T00:00:00Z")),
            ),
            kolkata,
        )
        assertEquals(listOf(DayCount(day("2026-03-09"), 1, denied = 1), DayCount(day("2026-03-08"), 2, denied = 1)), days)
    }

    @Test
    fun aDaySaysItsCountAndWhatWentWrong() {
        assertEquals("14", daySummary(DayCount(1, 14)))
        assertEquals("14 · 2 denied", daySummary(DayCount(1, 14, denied = 2)))
        assertEquals("14 · 2 denied · 1 failed", daySummary(DayCount(1, 14, denied = 2, failed = 1)))
        assertEquals("3 · 1 failed", daySummary(DayCount(1, 3, failed = 1)))
    }

    @Test
    fun todayIsOpenAndTheRestClosedUntilTapped() {
        val today = day("2026-10-15")
        assertEquals("Today", dayTitle(today, today))
        assertEquals("Yesterday", dayTitle(today - 1, today))
        assertEquals("Mon, 12 Oct", dayTitle(day("2026-10-12"), today, Locale.UK))
        assertEquals("30 Dec 2025", dayTitle(day("2025-12-30"), today, Locale.UK))
        assertTrue(dayOpen(today, today, emptyMap()))
        assertFalse(dayOpen(today - 1, today, emptyMap()))
        assertFalse(dayOpen(today, today, mapOf(today to false)))
        assertTrue(dayOpen(today - 1, today, mapOf(today - 1 to true)))
    }

    @Test
    fun outcomesAndDecidersReadAsTheCoreReadsThem() {
        assertEquals(ActivityOutcome.DENIED, outcomeKind("denied"))
        assertEquals(ActivityOutcome.FAILED, outcomeKind("error"))
        assertEquals(ActivityOutcome.APPROVED, outcomeKind("granted"))
        assertEquals(ActivityDecider.YOU, deciderKind(""))
        assertEquals(ActivityDecider.AUTOPILOT, deciderKind("overnight"))
        assertEquals(ActivityDecider.BYPASS, deciderKind("bypass"))
        assertEquals(ActivityDecider.LOCKDOWN, deciderKind("lockdown"))
    }

    // ---- the query --------------------------------------------------------------------------------------------------

    @Test
    fun theQueryCarriesTheWordsAndEveryFilter() {
        val now = at("2026-03-08T15:00:00Z") // 11:00 EDT
        val s = ActivitySearch(
            text = "  invoice march ",
            outcomes = setOf(ActivityOutcome.FAILED, ActivityOutcome.DENIED),
            deciders = setOf(ActivityDecider.AUTOPILOT),
            services = setOf("github", "gmail"),
            actions = setOf("send", "read"),
            connections = setOf("c2", "c1"),
            window = ActivityWindow.Today,
        )
        val q = s.query(now, newYork)
        assertEquals("invoice march", q.text)
        assertEquals(listOf(ActivityOutcome.DENIED, ActivityOutcome.FAILED), q.outcomes)
        assertEquals(listOf(ActivityDecider.AUTOPILOT), q.deciders)
        assertEquals(listOf("github", "gmail"), q.services)
        assertEquals(listOf("read", "send"), q.actions)
        assertEquals(listOf("c1", "c2"), q.connections)
        // Today starts at local midnight (EST, before the clocks moved).
        assertEquals(at("2026-03-08T05:00:00Z"), q.since)
        assertNull(q.until)
        assertEquals(now - 7 * 86_400, s.copy(window = ActivityWindow.Week).query(now, newYork).since)
        assertEquals(now - 30 * 86_400, s.copy(window = ActivityWindow.Month).query(now, newYork).since)
        assertNull(s.copy(window = ActivityWindow.Any).query(now, newYork).since)
    }

    @Test
    fun anEmptySearchIsBrowsing() {
        assertFalse(ActivitySearch().searching)
        assertFalse(ActivitySearch(text = "   ").searching)
        assertTrue(ActivitySearch(text = "x").searching)
        assertTrue(ActivitySearch(window = ActivityWindow.Week).searching)
        assertTrue(ActivitySearch(window = ActivityWindow.Week).filtered)
        assertFalse(ActivitySearch(text = "x").filtered)
        val q = ActivitySearch().query(0)
        assertEquals("", q.text)
        assertTrue(q.outcomes.isEmpty() && q.deciders.isEmpty() && q.services.isEmpty() && q.actions.isEmpty() && q.connections.isEmpty())
        assertNull(q.since)
    }

    @Test
    fun aChipSaysItsFirstPickAndHowManyMore() {
        assertEquals("Who", chipText(emptyList(), "Who"))
        assertEquals("Denied", chipText(listOf("Denied"), "Outcome"))
        assertEquals("Denied +1", chipText(listOf("Denied", "Failed"), "Outcome"))
    }

    // ---- what a title sets apart ------------------------------------------------------------------------------------

    private fun matched(text: String, vararg terms: String) = matchRanges(text, terms.toList()).map { text.substring(it.first, it.last + 1) }

    @Test
    fun onlyTheStartOfAWordMatches() {
        assertEquals(listOf("inv"), matched("Searched mail for invoice", "inv"))
        assertEquals(emptyList<String>(), matched("Reinvoiced", "inv"))
        assertEquals(listOf("Inv", "inv"), matched("Invoice and re-invoice", "inv"))
        // Case does not matter; the longest term that fits wins.
        assertEquals(listOf("INVOICE"), matched("INVOICE", "in", "invoice"))
        assertEquals(emptyList<String>(), matched("Read", "readme"))
    }

    @Test
    fun matchesApartByPunctuationJoin() {
        assertEquals(listOf("alice@example.com"), matched("Sent an email to alice@example.com", "alice", "example", "com"))
        assertEquals(listOf("src/app"), matched("Read src/app/main.rs", "src", "app"))
        assertEquals(listOf("acme/web"), matched("Pushed 3 commits to acme/web", "acme", "web"))
        assertEquals(listOf("v1.2"), matched("Tag v1.2", "v1", "2"))
        // A space, or three characters between, keeps them apart.
        assertEquals(listOf("alice", "example"), matched("alice example", "alice", "example"))
        assertEquals(listOf("a", "b"), matched("a --- b", "a", "b"))
    }

    @Test
    fun hiddenCharactersInATitleCannotMakeOrMoveAHighlight() {
        val hit = SpanStyle(fontWeight = FontWeight.SemiBold)
        fun highlights(s: AnnotatedString) = s.spanStyles.filter { it.item == hit }.map { s.text.substring(it.start, it.end) to it.start }

        // A zero-width space would start a word inside "Payinvoice": cleaned first, there is no word start to match.
        assertEquals(listOf("inv"), matched("Pay​invoice", "inv"))
        val zw = askTitle("Pay​invoice", null, listOf("inv"), hit)
        assertEquals("Payinvoice", zw.text)
        assertTrue(highlights(zw).isEmpty())
        // A right-to-left override cannot shift the match: it covers exactly the word shown.
        val rlo = askTitle("Read ‮invoice", null, listOf("invoice"), hit)
        assertEquals("Read invoice", rlo.text)
        assertEquals(listOf("invoice" to 5), highlights(rlo))
        // A soft hyphen inside a word: the word is shown whole, and matched whole.
        val shy = askTitle("Searched for in­voice", null, listOf("invoice"), hit)
        assertEquals("Searched for invoice", shy.text)
        assertEquals(listOf("invoice" to 13), highlights(shy))
    }

    @Test
    fun theTitleKeepsItsItalicTargetUnderTheHighlight() {
        val hit = SpanStyle(fontWeight = FontWeight.SemiBold)
        val s = askTitle("Searched mail for invoice 42", "invoice 42", listOf("inv", "mail"), hit)
        assertEquals("Searched mail for invoice 42", s.text)
        val italic = s.spanStyles.single { it.item.fontStyle == FontStyle.Italic }
        assertEquals("invoice 42", s.text.substring(italic.start, italic.end))
        assertEquals(listOf("mail", "inv"), s.spanStyles.filter { it.item == hit }.map { s.text.substring(it.start, it.end) })
        // No words, no highlight.
        assertTrue(askTitle("Searched mail", null, emptyList(), hit).spanStyles.isEmpty())
    }

    @Test
    fun aSnippetIsItsFieldThenItsPiecesWithTheMatchesApart() {
        val label = SpanStyle(fontStyle = FontStyle.Italic)
        val hit = SpanStyle(fontWeight = FontWeight.SemiBold)
        val rest = SpanStyle()
        val snippet = ActivitySnippet(
            ActivityField.FILE,
            listOf(ActivityHighlight(" changed ", false), ActivityHighlight("src/app", true), ActivityHighlight("/main.rs‮ ", false)),
        )
        val s = snippetString(snippet, label, hit, rest)
        assertEquals("FILE  changed src/app/main.rs", s.text)
        assertEquals("src/app", s.spanStyles.single { it.item == hit }.let { s.text.substring(it.start, it.end) })
        assertEquals("FILE", s.spanStyles.single { it.item == label }.let { s.text.substring(it.start, it.end) })
    }
}
