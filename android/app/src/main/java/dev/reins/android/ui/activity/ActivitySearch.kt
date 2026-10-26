package dev.reins.android.ui.activity

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import dev.reins.android.state.ActivitySearch
import dev.reins.android.state.ActivityWindow
import dev.reins.android.design.askString
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedLine
import dev.reins.core.ActivityDecider
import dev.reins.core.ActivityEntry
import dev.reins.core.ActivityField
import dev.reins.core.ActivityOutcome
import dev.reins.core.ActivityQuery
import dev.reins.core.ActivitySnippet
import dev.reins.core.ActivityStamp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

// Activity by day, and its search: what the list groups, counts, asks the core for and sets apart, kept apart from
// the drawing so it can be tested. The search itself runs in the core, on this phone; nothing here sends it anywhere.

// ---- days -------------------------------------------------------------------------------------------------------------

/** The local day (days since 1970-01-01 in [zone]) an entry logged at [epochSeconds] belongs to. */
fun localDay(epochSeconds: Long, zone: ZoneId = ZoneId.systemDefault()): Long =
    Instant.ofEpochSecond(epochSeconds).atZone(zone).toLocalDate().toEpochDay()

/** When local day [epochDay] starts in [zone], in Unix seconds (its first instant, also on a daylight-saving change). */
fun dayStart(epochDay: Long, zone: ZoneId = ZoneId.systemDefault()): Long =
    LocalDate.ofEpochDay(epochDay).atStartOfDay(zone).toEpochSecond()

/** A day of Activity as its header shows it: how many entries, how many were denied and how many failed. */
data class DayCount(val day: Long, val count: Int, val denied: Int = 0, val failed: Int = 0, val auto: Int = 0) {
    /** Carried out by you or a grant (not by Autopilot): the ✓ of the header. */
    val approved: Int get() = (count - denied - failed - auto).coerceAtLeast(0)
}

/** The stamps (newest first) by local day, newest day first. */
fun daysOf(stamps: List<ActivityStamp>, zone: ZoneId = ZoneId.systemDefault()): List<DayCount> =
    countDays(stamps.map { Triple(localDay(it.at, zone), it.outcome, it.decider) })

/** The entries (newest first) by local day, as [daysOf] counts stamps: until the core's stamps are read. */
fun daysOfEntries(entries: List<ActivityEntry>, zone: ZoneId = ZoneId.systemDefault()): List<DayCount> =
    countDays(entries.map { Triple(localDay(it.at, zone), outcomeKind(it.outcome), deciderKind(it.decidedBy)) })

/** Each day's count: all, denied, failed, and what Autopilot decided (counted once, under ⚡, whatever it decided). */
private fun countDays(items: List<Triple<Long, ActivityOutcome, ActivityDecider>>): List<DayCount> {
    val days = LinkedHashMap<Long, IntArray>()
    for ((day, outcome, decider) in items) {
        val n = days.getOrPut(day) { IntArray(4) }
        n[0]++
        if (decider == ActivityDecider.AUTOPILOT) {
            n[3]++
        } else {
            if (outcome == ActivityOutcome.DENIED) n[1]++
            if (outcome == ActivityOutcome.FAILED) n[2]++
        }
    }
    return days.entries.map { (day, n) -> DayCount(day, n[0], n[1], n[2], n[3]) }.sortedByDescending { it.day }
}

/** An entry's outcome as the core's filters read it. */
fun outcomeKind(outcome: String): ActivityOutcome = when (outcome) {
    "denied" -> ActivityOutcome.DENIED
    "error", "started", "unknown", "succeeded_withheld" -> ActivityOutcome.FAILED
    else -> ActivityOutcome.APPROVED
}

/** Who decided an entry, as the core's filters read it (automatic includes learned decisions and explicit permissions). */
fun deciderKind(decidedBy: String): ActivityDecider = when (decidedBy) {
    "" -> ActivityDecider.YOU
    "bypass" -> ActivityDecider.BYPASS
    "lockdown" -> ActivityDecider.LOCKDOWN
    else -> ActivityDecider.AUTOPILOT
}

/** A day's count beside its label: "14", "14 · 2 denied", "14 · 2 denied · 1 failed". */
fun daySummary(d: DayCount): String = buildString {
    append(d.count)
    if (d.denied > 0) append(" · ${d.denied} denied")
    if (d.failed > 0) append(" · ${d.failed} ${if (d.failed == 1) "needs" else "need"} attention")
}

/** What a day's header says it holds, for a screen reader: "14 entries, 2 denied". */
fun daySpoken(d: DayCount): String = buildString {
    append(if (d.count == 1) "1 entry" else "${d.count} entries")
    if (d.auto > 0) append(", ${d.auto} by Autopilot")
    if (d.denied > 0) append(", ${d.denied} denied")
    if (d.failed > 0) append(", ${d.failed} ${if (d.failed == 1) "needs" else "need"} attention")
}

/** "Today", "Yesterday", else the date ("Mon, 3 Mar"; "3 Mar 2025" in another year). */
fun dayTitle(epochDay: Long, today: Long, locale: Locale = Locale.getDefault()): String = when (today - epochDay) {
    0L -> "Today"
    1L -> "Yesterday"
    else -> {
        val date = LocalDate.ofEpochDay(epochDay)
        val pattern = if (date.year == LocalDate.ofEpochDay(today).year) "EEE, d MMM" else "d MMM yyyy"
        date.format(DateTimeFormatter.ofPattern(pattern, locale))
    }
}

/** A day starts open when it is Today, unless it was opened or closed by hand ([opened]). */
fun dayOpen(day: Long, today: Long, opened: Map<Long, Boolean>): Boolean = opened[day] ?: (day == today)

/** The query for one local day's entries: from its midnight up to the next one. */
fun dayQuery(epochDay: Long, zone: ZoneId = ZoneId.systemDefault()): ActivityQuery =
    ActivityQuery(since = dayStart(epochDay, zone), until = dayStart(epochDay + 1, zone))

// ---- the search -------------------------------------------------------------------------------------------------------

/** What the core is asked for: the words, the filters, and how far back "When" reaches from [nowSeconds]. */
fun ActivitySearch.query(nowSeconds: Long, zone: ZoneId = ZoneId.systemDefault()): ActivityQuery = ActivityQuery(
    text = text.trim(),
    outcomes = outcomes.sortedBy { it.ordinal },
    deciders = deciders.sortedBy { it.ordinal },
    services = services.sorted(),
    actions = actions.sorted(),
    connections = connections.sorted(),
    since = when (window) {
        ActivityWindow.Any -> null
        ActivityWindow.Today -> dayStart(localDay(nowSeconds, zone), zone)
        ActivityWindow.Week -> nowSeconds - 7 * DAY_SECONDS
        ActivityWindow.Month -> nowSeconds - 30 * DAY_SECONDS
    },
    until = null,
)

private const val DAY_SECONDS = 86_400L

/** [set] with [value] added, or taken out when it is there. */
fun <T> Set<T>.toggled(value: T): Set<T> = if (value in this) this - value else this + value

/** An outcome in the filter's words. */
fun outcomeName(o: ActivityOutcome): String = when (o) {
    ActivityOutcome.APPROVED -> "Approved"
    ActivityOutcome.DENIED -> "Denied"
    ActivityOutcome.FAILED -> "Needs attention"
}

/** Who decided, in the filter's words. */
fun deciderName(d: ActivityDecider): String = when (d) {
    ActivityDecider.YOU -> "You"
    ActivityDecider.AUTOPILOT -> "Automatic"
    ActivityDecider.BYPASS -> "Bypass"
    ActivityDecider.LOCKDOWN -> "Lockdown"
}

/** How far back, in the filter's words. */
fun windowName(w: ActivityWindow): String = when (w) {
    ActivityWindow.Any -> "Any time"
    ActivityWindow.Today -> "Today"
    ActivityWindow.Week -> "7 days"
    ActivityWindow.Month -> "30 days"
}

/** A kind of entry (`ActivityEntry.action`) in the words the kind lines use ("Search", "Access", "Connect"). */
fun kindName(action: String): String = when (action) {
    "search" -> "Search"
    "list" -> "List"
    "read" -> "Read"
    "headlines" -> "Headlines"
    "write" -> "Change"
    "send" -> "Send"
    "grant" -> "Access"
    "accounts" -> "Accounts"
    "pair" -> "Connect"
    "join" -> "New phone"
    "upload" -> "Upload"
    "request" -> "Request"
    "delete" -> "Delete"
    "push" -> "Push"
    "run" -> "Run"
    "pay" -> "Buy"
    else -> action.replaceFirstChar { it.uppercase() }
}

/** A chip's words: [none] when nothing is picked, else the first pick ("Denied"), with "+1" for each more. */
fun chipText(picked: List<String>, none: String): String = when (picked.size) {
    0 -> none
    1 -> picked[0]
    else -> "${picked[0]} +${picked.size - 1}"
}

// ---- setting apart what matched ---------------------------------------------------------------------------------------

/** A character of a word: a letter or a digit (as the core's index reads them). */
private fun wordChar(cp: Int): Boolean =
    Character.isAlphabetic(cp) || Character.isDigit(cp) ||
        Character.getType(cp).let { it == Character.LETTER_NUMBER.toInt() || it == Character.OTHER_NUMBER.toInt() }

/**
 * Where [text] matches [terms] (lower case), as ranges of [text]'s chars (end exclusive): each term at the start of a
 * word only (never inside one), matching as many characters as the term has, the longest term first; two matches
 * apart by one or two characters that are neither letters, digits nor spaces (`@ . / - _`) join into one. The same rule
 * as the core's snippets. [text] must be what is shown, cleaned ([dev.reins.android.ui.common.untrusted]) first, so a
 * hidden character can neither start a word nor move a match.
 */
fun matchRanges(text: String, terms: List<String>): List<IntRange> {
    val wanted = terms.filter { it.isNotEmpty() }.map { t -> t.lowercase().codePoints().toArray() }
    if (wanted.isEmpty() || text.isEmpty()) return emptyList()
    // Each character with where it starts, lower case.
    val starts = ArrayList<Int>(text.length)
    val chars = ArrayList<Int>(text.length)
    var i = 0
    while (i < text.length) {
        val cp = text.codePointAt(i)
        starts += i
        chars += Character.toLowerCase(cp)
        i += Character.charCount(cp)
    }
    val found = mutableListOf<IntArray>()
    var k = 0
    while (k < chars.size) {
        val startsWord = wordChar(chars[k]) && (k == 0 || !wordChar(chars[k - 1]))
        if (startsWord) {
            val n = wanted.filter { t -> t.size <= chars.size - k && t.indices.all { j -> t[j] == chars[k + j] } }.maxOfOrNull { it.size }
            if (n != null) {
                val end = if (k + n < chars.size) starts[k + n] else text.length
                found += intArrayOf(starts[k], end)
                k += n
                continue
            }
        }
        k++
    }
    val joined = mutableListOf<IntArray>()
    for (f in found) {
        val last = joined.lastOrNull()
        if (last != null) {
            val gap = text.substring(last[1], f[0])
            val cps = gap.codePoints().toArray()
            if (cps.size <= 2 && cps.all { !wordChar(it) && !Character.isWhitespace(it) }) {
                last[1] = f[1]
                continue
            }
        }
        joined += f
    }
    return joined.map { it[0] until it[1] }
}

/** [text] with what matches [terms] set in [style] (see [matchRanges]); itself when nothing matches. */
fun AnnotatedString.highlighted(terms: List<String>, style: SpanStyle): AnnotatedString {
    if (terms.isEmpty()) return this
    val ranges = matchRanges(text, terms)
    if (ranges.isEmpty()) return this
    val base = this
    return buildAnnotatedString {
        append(base)
        ranges.forEach { addStyle(style, it.first, it.last + 1) }
    }
}

/**
 * A row's title as shown: the sentence cleaned ([untrusted]), its target in italic, and where it matches [terms] set in
 * [highlight]. Matched on the cleaned text, the one shown: a hidden character can neither start a match nor move one.
 */
fun askTitle(text: String, target: String?, terms: List<String> = emptyList(), highlight: SpanStyle? = null): AnnotatedString {
    val base = askString(untrusted(text), target?.let(::untrusted))
    return if (highlight == null) base else base.highlighted(terms, highlight)
}

/** The tiny label before a snippet: which part of the entry it comes from. */
fun fieldLabel(f: ActivityField): String = when (f) {
    ActivityField.TITLE -> "TITLE"
    ActivityField.AGENT -> "AGENT"
    ActivityField.ADDRESS -> "ADDRESS"
    ActivityField.SUBJECT -> "SUBJECT"
    ActivityField.BODY -> "BODY"
    ActivityField.REPO -> "REPO"
    ActivityField.BRANCH -> "BRANCH"
    ActivityField.COMMIT -> "COMMIT"
    ActivityField.FILE -> "FILE"
    ActivityField.DIFF -> "DIFF"
    ActivityField.COMMAND -> "COMMAND"
    ActivityField.URL -> "URL"
    ActivityField.REQUEST -> "REQUEST"
}

/**
 * A snippet as one line: its field's label in [label], then its pieces, the matching ones in [hit] and the rest in
 * [rest]. Each piece is cleaned again on the phone (the core already did), keeping the spaces between pieces.
 */
fun snippetString(snippet: ActivitySnippet, label: SpanStyle, hit: SpanStyle, rest: SpanStyle): AnnotatedString = buildAnnotatedString {
    withStyle(label) { append(fieldLabel(snippet.field)) }
    append("  ")
    val pieces = snippet.spans.map { untrustedLine(it.text).replace('\n', ' ').replace('\t', ' ') to it.hit }
    pieces.forEachIndexed { i, (text, isHit) ->
        val shown = when (i) {
            0 -> text.trimStart()
            pieces.lastIndex -> text.trimEnd()
            else -> text
        }
        if (shown.isNotEmpty()) withStyle(if (isHit) hit else rest) { append(shown) }
    }
}
