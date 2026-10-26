package dev.reins.android.ui.activity

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.runtime.Immutable
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.sp
import dev.reins.core.ActivitySnippet
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalReduceMotion
import dev.reins.android.design.MenuItem
import dev.reins.android.design.Motion
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.StatusWord
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.AskSentenceText
import dev.reins.android.ui.common.IntegrationIcon
import dev.reins.android.ui.common.actionField
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ActivityEntry
import dev.reins.core.ConnectionView
import dev.reins.core.ActivityDecider
import dev.reins.core.ActivityFacets
import dev.reins.core.ActivityOutcome
import dev.reins.android.state.ActivitySearch
import dev.reins.android.state.ActivityWindow
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import kotlin.math.roundToInt

// The river (Reins Next, Activity and "Activity with a wallpaper"): what was decided, by day, each day a pane (frosted
// over the wallpaper). A row is the integration's tile with what it did on its corner, who did what in one sentence,
// and on the right when and how it ended; the row the last answer folded into glows in its kind colour for 4 s.

/** A label over search results, as the top of its pane: the day and how many were found. */
@Composable
fun DayLabel(label: String, count: String?, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = 4.dp).semantics { heading() },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LabelText(label, c.ink2, Modifier.weight(1f))
        count?.let { RText(it, RType.section, c.ink2, maxLines = 1) }
    }
}

/**
 * A day of the river that opens and closes, as the top of its pane (the whole pane while closed): the day, then what
 * it holds (approved, decided by Autopilot, denied, needs attention, each with its glyph and colour) and a chevron
 * that turns as it opens. A heading that says whether it is open and what it holds.
 */
@Composable
fun DayHeader(day: DayCount, label: String, open: Boolean, modifier: Modifier = Modifier, onToggle: () -> Unit) {
    val c = LocalColors.current
    val reduce = LocalReduceMotion.current
    val turn by animateFloatAsState(if (open) 0f else -90f, if (reduce) Motion.reduced() else Motion.fold(false), label = "dayChevron")
    Row(
        modifier
            .fillMaxWidth()
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), label = if (open) "Collapse" else "Expand", onClick = onToggle)
            .semantics(mergeDescendants = true) {
                heading()
                contentDescription = "$label, ${daySpoken(day)}"
                stateDescription = if (open) "Expanded" else "Collapsed"
            }
            .testTag("day:${day.day}")
            .padding(start = 14.dp, end = 12.dp, top = if (open) 12.dp else 14.dp, bottom = if (open) 4.dp else 14.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LabelText(label, c.ink2, Modifier.weight(1f))
        DayCounts(day, Modifier.testTag("daySummary:${day.day}"))
        GlyphIcon(Glyph.ChevronDown, c.inkMeta, size = 16.dp, modifier = Modifier.graphicsLayer { rotationZ = turn })
    }
}

/** ✓ 7  ⚡ 2  ✕ 1  ! 1: what a day holds, each count in its colour with its glyph, tabular. Zeros are left out. */
@Composable
fun DayCounts(day: DayCount, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        @Composable
        fun count(n: Int, glyph: Glyph, tint: Color) {
            if (n <= 0) return
            Row(horizontalArrangement = Arrangement.spacedBy(3.dp), verticalAlignment = Alignment.CenterVertically) {
                GlyphIcon(glyph, tint, size = 13.dp)
                RText(n.toString(), RType.section.copy(fontFeatureSettings = "tnum"), tint, maxLines = 1)
            }
        }
        count(day.approved, Glyph.Check, c.success)
        count(day.auto, Glyph.AutopilotFilled, c.violet)
        count(day.denied, Glyph.Close, c.danger)
        count(day.failed, Glyph.CircleAlert, c.warning)
    }
}

/** What a row says: who did what, the AI's own name first ("My ChatGPT read **2 emails**"). */
internal fun rowSentence(label: String, text: String): String {
    val who = untrusted(label).trim()
    if (who.isEmpty()) return text
    // "Read 3 emails" becomes "read 3 emails"; a word in capitals ("SSH key") keeps them.
    val lower = if (text.length > 1 && text[0].isUpperCase() && text[1].isLowerCase()) text[0].lowercaseChar() + text.substring(1) else text
    return "$who $lower"
}

/** When, at the end of a row: "now", "5m", "2h" today; the time of day before that. */
fun compactTime(epochSeconds: Long, nowSeconds: Long, zone: java.util.TimeZone = java.util.TimeZone.getDefault()): String {
    val d = nowSeconds - epochSeconds
    val sameDay = java.time.Instant.ofEpochSecond(epochSeconds).atZone(zone.toZoneId()).toLocalDate() ==
        java.time.Instant.ofEpochSecond(nowSeconds).atZone(zone.toZoneId()).toLocalDate()
    return when {
        d in 0 until 60 -> "now"
        d in 60 until 3_600 -> "${d / 60}m"
        d in 3_600 until 86_400 && sameDay -> "${d / 3_600}h"
        else -> java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).apply { timeZone = zone }.format(java.util.Date(epochSeconds * 1000))
    }
}

/**
 * One decided entry, as a row of its day's pane: its integration tile (what it did on the corner), who did what (the
 * target in 600), the machine detail in mono, a risk line in amber, and on the right when and how it ended. A hairline
 * above it unless it is the first ([divider]). [landed]: the fold just put it here; its tile grows into place and the
 * row glows faintly in its kind colour, then settles.
 */
@Composable
fun RiverRow(
    entry: ActivityEntry,
    nowSeconds: Long,
    landed: Boolean,
    showShare: Boolean,
    modifier: Modifier = Modifier,
    /** Words searched (lower case): set apart where the title and who asked match them. */
    terms: List<String> = emptyList(),
    /** Where a search matched outside what the row shows: one more line under it. */
    snippet: ActivitySnippet? = null,
    divider: Boolean = true,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val hit = matchStyle()
    val reduce = LocalReduceMotion.current
    val sentence = remember(entry) { askSentence(entry) }
    val field = actionField(sentence.action)
    val ink = field.kindInk(c)
    val wash by animateColorAsState(if (landed) ink.copy(alpha = 0.08f) else Color.Transparent, Motion.fade(), label = "landedWash")
    // Landing: the tile grows out of the fold (from 1.6 times its size, 360 ms).
    val pop = remember { Animatable(1f) }
    LaunchedEffect(landed) {
        if (landed && !reduce) {
            pop.snapTo(1.6f)
            pop.animateTo(1f, Motion.fold(false))
        }
    }
    val words = remember(sentence, entry.connectionLabel) { sentence.copy(text = rowSentence(entry.connectionLabel, sentence.text)) }
    Row(
        modifier
            .fillMaxWidth()
            .background(wash)
            .then(if (divider) Modifier.ruleAbove(c) else Modifier)
            .pressable(highlight = c.paperSunken.copy(alpha = 0.6f), shape = RoundedCornerShape(0.dp), onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = 14.dp, vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .padding(end = 5.dp, bottom = 5.dp)
                .graphicsLayer {
                    scaleX = pop.value
                    scaleY = pop.value
                }
                .testTag("tile:${entry.id}"),
        ) { IntegrationIcon(sentence.provider, sentence.action, size = 36.dp, ring = c.paperRaised, field = field) }
        Column(Modifier.weight(1f)) {
            AskSentenceText(
                words,
                style = RType.rowSentence,
                color = c.ink,
                detailsColor = c.inkMeta,
                detailsStyle = RType.mono(12f),
                maxLines = 3,
                detailsMaxLines = 1,
                tag = "entryTitle:${entry.id}",
                detailsTag = "entryDetails:${entry.id}",
                terms = terms,
                highlight = hit.title,
            )
            snippet?.let { SnippetLine(it, hit, Modifier.padding(top = 3.dp).testTag("snippet:${entry.id}")) }
            entry.autopilot?.risk?.takeIf { AutopilotText.riskWorthListing(it) }?.let { risk ->
                RiskLine(AutopilotText.riskLine(risk), Modifier.padding(top = 3.dp).testTag("rowRisk:${entry.id}"))
            }
            if (landed) RText(outcomeLine(entry), RType.meta, c.violet, Modifier.padding(top = 2.dp), maxLines = 1)
        }
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            RText(compactTime(entry.at, nowSeconds), RType.sans(12.5f, FontWeight.Medium, tracking = 0.01f).copy(fontFeatureSettings = "tnum"), c.inkMeta, maxLines = 1)
            OutcomeEnd(outcomeOf(entry), Modifier.testTag("outcome:${entry.id}"), share = if (showShare) share(entry) else null)
        }
    }
}

/** A hairline above a row of a pane: from past the tile to 16 dp short of the right edge. */
fun Modifier.ruleAbove(c: dev.reins.android.design.RColors, start: androidx.compose.ui.unit.Dp = 62.dp): Modifier = drawBehind {
    val y = 0.5.dp.toPx()
    drawLine(c.rule, androidx.compose.ui.geometry.Offset(start.toPx(), y), androidx.compose.ui.geometry.Offset(size.width - 16.dp.toPx(), y), strokeWidth = 1.dp.toPx())
}

/** What made a row risky, in amber under it: the warning glyph and a few words ("Not in your contacts"). */
@Composable
fun RiskLine(text: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(5.dp), verticalAlignment = Alignment.CenterVertically) {
        GlyphIcon(Glyph.Warning, c.warning, size = 13.dp)
        RText(text, RType.sans(12.5f, FontWeight.Medium), c.warning, Modifier.testTag("riskLine"), maxLines = 2)
    }
}

/** How a match is set apart: a soft violet wash, and never by colour alone (an underline in the serif, weight elsewhere). */
@Immutable
data class MatchStyle(val title: SpanStyle, val text: SpanStyle)

@Composable
fun matchStyle(): MatchStyle {
    val c = LocalColors.current
    return remember(c) {
        val wash = c.violet.copy(alpha = if (c.dark) 0.24f else 0.14f)
        MatchStyle(
            title = SpanStyle(background = wash, textDecoration = TextDecoration.Underline),
            text = SpanStyle(color = c.ink, fontWeight = FontWeight.SemiBold, background = wash),
        )
    }
}

/** Where a search matched outside the row: the field in tiny mono capitals, then the words, the matches set apart. */
@Composable
private fun SnippetLine(snippet: ActivitySnippet, hit: MatchStyle, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val text = remember(snippet, hit, c) {
        snippetString(
            snippet,
            label = RType.label.copy(fontSize = 9.5.sp, color = c.inkMeta).toSpanStyle(),
            hit = hit.text,
            rest = SpanStyle(color = c.inkMeta),
        )
    }
    BasicText(text, modifier, style = RType.sans(13f, lineHeight = 18f).copy(color = c.inkMeta), maxLines = 2, overflow = TextOverflow.Ellipsis)
}

/** Autopilot's share for its decision: "97%", or "91% deny". */
private fun share(e: ActivityEntry): String? {
    val note = e.autopilot ?: return null
    val denied = e.outcome == "denied"
    val p = (if (denied) note.pDeny else note.pApprove).takeIf { it > 0f } ?: return null
    return "${(p * 100).roundToInt()}%" + if (denied) " deny" else ""
}

/** How an entry ended, at the end of its row: a tick or a cross when you decided, else a small tag saying who did. */
@Composable
fun OutcomeEnd(outcome: Outcome, modifier: Modifier = Modifier, share: String? = null) {
    val c = LocalColors.current
    when (outcome) {
        Outcome.Approved -> GlyphIcon(Glyph.Check, c.success, modifier = modifier.semantics { this.contentDescription = "Approved" }, size = 18.dp)
        Outcome.Denied -> GlyphIcon(Glyph.Close, c.danger, modifier = modifier.semantics { this.contentDescription = "Denied" }, size = 18.dp)
        Outcome.Permission -> OutcomeTag("Permission", Glyph.Shield, c.violet, modifier, spoken = "Permission")
        Outcome.Grant -> OutcomeTag("Grant", Glyph.Key, c.violet, modifier, spoken = "Grant")
        Outcome.Auto -> OutcomeTag(listOfNotNull("Auto", share).joinToString(" "), Glyph.AutopilotFilled, c.violet, modifier, spoken = "Approved by Autopilot")
        Outcome.AutoDenied -> OutcomeTag(listOfNotNull("Auto", share).joinToString(" "), Glyph.AutopilotFilled, c.danger, modifier, spoken = "Denied by Autopilot")
        Outcome.Overnight -> OutcomeTag(AutopilotText.OVERNIGHT, Glyph.Moon, c.violet, modifier)
        Outcome.OvernightDenied -> OutcomeTag(AutopilotText.OVERNIGHT, Glyph.Moon, c.danger, modifier, spoken = "Denied overnight")
        Outcome.Bypass -> OutcomeTag("Bypass", Glyph.Bypass, c.danger, modifier)
        Outcome.Lockdown -> OutcomeTag("Locked", Glyph.Block, c.warning, modifier, spoken = "Denied in lockdown")
        Outcome.Failed -> OutcomeTag("Failed", Glyph.CircleAlert, c.warning, modifier)
        Outcome.Started -> OutcomeTag("Awaiting outcome", Glyph.CircleAlert, c.warning, modifier, spoken = "Attempted; awaiting outcome")
        Outcome.Unknown -> OutcomeTag("Outcome unknown", Glyph.CircleAlert, c.warning, modifier, spoken = "Outcome unknown; do not repeat")
        Outcome.Withheld -> OutcomeTag("Response withheld", Glyph.CircleAlert, c.warning, modifier, spoken = "Completed; response withheld; do not repeat")
    }
}

/** A small outcome tag (`.otag`): 20 dp, the glyph and the word in [tint] on its 12 % wash. */
@Composable
fun OutcomeTag(text: String, glyph: Glyph?, tint: Color, modifier: Modifier = Modifier, spoken: String? = null) {
    Row(
        modifier
            .heightIn(min = 20.dp)
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.12f))
            .then(if (spoken != null) Modifier.clearAndSetSemantics { contentDescription = spoken } else Modifier)
            .padding(horizontal = 7.dp),
        horizontalArrangement = Arrangement.spacedBy(3.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        glyph?.let { GlyphIcon(it, tint, size = 11.dp) }
        RText(text, RType.tag, tint, maxLines = 1)
    }
}

/**
 * The filters, as chips under the header, each a menu: Outcome, Who, Kind (integrations and kinds), AI and When. A chip
 * with a pick is ink and says it; the round clear button drops them all.
 */
@Composable
fun FilterRow(
    search: ActivitySearch,
    facets: ActivityFacets?,
    connections: List<ConnectionView>,
    modifier: Modifier = Modifier,
    onChange: (ActivitySearch) -> Unit,
) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 24.dp).testTag("filterRow"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (search.filtered) {
            Box(
                Modifier
                    .size(34.dp)
                    .clip(CircleShape)
                    .background(c.paperSunken)
                    .pressable(shape = CircleShape, label = "Clear filters") {
                        onChange(search.copy(outcomes = emptySet(), deciders = emptySet(), services = emptySet(), actions = emptySet(), connections = emptySet(), window = ActivityWindow.Any))
                    }
                    .semantics { contentDescription = "Clear filters" }
                    .testTag("filtersClear"),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.Close, c.ink, size = 16.dp) }
        }
        MenuChip(
            chipText(ActivityOutcome.entries.filter { it in search.outcomes }.map(::outcomeName), "Outcome"),
            active = search.outcomes.isNotEmpty(),
            tag = "filter:outcome",
            name = "Outcome",
            items = ActivityOutcome.entries.map { o ->
                MenuItem(outcomeName(o), selected = o in search.outcomes, tag = "outcome:${o.name.lowercase()}") { onChange(search.copy(outcomes = search.outcomes.toggled(o))) }
            },
        )
        MenuChip(
            chipText(ActivityDecider.entries.filter { it in search.deciders }.map(::deciderName), "Who"),
            active = search.deciders.isNotEmpty(),
            tag = "filter:who",
            name = "Who decided",
            items = ActivityDecider.entries.map { d ->
                MenuItem(deciderName(d), selected = d in search.deciders, tag = "who:${d.name.lowercase()}") { onChange(search.copy(deciders = search.deciders.toggled(d))) }
            },
        )
        val services = (facets?.services.orEmpty() + search.services).distinct()
        val actions = (facets?.actions.orEmpty() + search.actions).distinct()
        MenuChip(
            chipText(services.filter { it in search.services }.map(::serviceLabel) + actions.filter { it in search.actions }.map(::kindName), "Kind"),
            active = search.services.isNotEmpty() || search.actions.isNotEmpty(),
            tag = "filter:kind",
            name = "Kind",
            items = services.map { sv ->
                MenuItem(serviceLabel(sv), selected = sv in search.services, tag = "service:$sv") { onChange(search.copy(services = search.services.toggled(sv))) }
            } + actions.mapIndexed { i, a ->
                MenuItem(kindName(a), glyph = kindGlyph(a), selected = a in search.actions, divider = i == 0 && services.isNotEmpty(), tag = "action:$a") {
                    onChange(search.copy(actions = search.actions.toggled(a)))
                }
            },
        )
        // Who asked: the connections with entries, else (until they are read) the ones connected now.
        val ais = facets?.connections?.map { it.id to it.label } ?: connections.map { it.id to it.label }
        MenuChip(
            chipText(ais.filter { it.first in search.connections }.map { untrusted(it.second) }, "Any AI"),
            active = search.connections.isNotEmpty(),
            tag = "filter:ai",
            name = "Which AI",
            items = listOf(MenuItem("Any AI", selected = search.connections.isEmpty(), tag = "ai:any") { onChange(search.copy(connections = emptySet())) }) +
                ais.map { (id, label) -> MenuItem(untrusted(label), selected = id in search.connections, tag = "ai:$id") { onChange(search.copy(connections = search.connections.toggled(id))) } },
        )
        MenuChip(
            if (search.window == ActivityWindow.Any) "When" else windowName(search.window),
            active = search.window != ActivityWindow.Any,
            tag = "filter:when",
            name = "When",
            items = listOf(ActivityWindow.Today, ActivityWindow.Week, ActivityWindow.Month, ActivityWindow.Any).map { w ->
                MenuItem(windowName(w), selected = w == search.window, tag = "when:${w.name.lowercase()}") { onChange(search.copy(window = w)) }
            },
        )
    }
}

/** A filter chip that opens its menu: raised with a hairline, ink when something is picked. */
@Composable
private fun MenuChip(text: String, active: Boolean, tag: String, name: String, items: List<MenuItem>) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var menu by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .heightIn(min = 34.dp)
                .clip(CircleShape)
                .background(if (active) c.inverse else c.paperRaised)
                .then(if (active) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
                .pressable(shape = CircleShape, label = name) {
                    menu = true
                    feedback.play(Event.Open)
                }
                .semantics {
                    selected = active
                    if (active) stateDescription = text
                }
                .testTag(tag)
                .padding(horizontal = 13.dp, vertical = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val fg = if (active) c.onInverse else c.ink
            RText(text, RType.sans(14f, FontWeight.Medium), fg, maxLines = 1)
            GlyphIcon(Glyph.ChevronDown, fg, size = 14.dp)
        }
        SignalMenu(expanded = menu, onDismiss = { menu = false }, items = items, alignEnd = false, width = 240.dp)
    }
}
