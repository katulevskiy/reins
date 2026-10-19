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

// The river (Reins Signal, iOS-Activity 01, 02 and 04): what was decided, as paper rows. A kind tile, what it did,
// who and when, and how it ended; the row the last answer folded into glows in its kind colour for 4 s.

/** A day's label in mono capitals, with its count at the end on Today ("12 asked · 9 by Autopilot"). */
@Composable
fun DayLabel(label: String, count: String?, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 22.dp, bottom = 6.dp).semantics { heading() }, verticalAlignment = Alignment.CenterVertically) {
        LabelText(label, c.inkMeta, Modifier.weight(1f))
        count?.let { RText(it, RType.meta, c.inkMeta, maxLines = 1) }
    }
}

/**
 * A day of the river that opens and closes: its label in mono capitals, its count and what went wrong ("14 · 2
 * denied"), and a chevron that turns as it opens. A heading that says whether it is open.
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
            .padding(start = 24.dp, end = 20.dp, top = 22.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LabelText(label, if (open) c.inkMeta else c.ink2, Modifier.weight(1f))
        RText(daySummary(day), RType.meta, c.inkMeta, Modifier.testTag("daySummary:${day.day}"), maxLines = 1)
        GlyphIcon(Glyph.ChevronDown, c.inkFaint, size = 14.dp, modifier = Modifier.graphicsLayer { rotationZ = turn })
    }
}

/**
 * One decided entry: its kind tile, what it did (the row's words), who and when (and "under a grant", Autopilot's
 * share when filtering by it), and how it ended. [landed]: the fold just put it here; its tile is the solid field and
 * the row glows faintly, then both settle to paper.
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
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val hit = matchStyle()
    val reduce = LocalReduceMotion.current
    val sentence = remember(entry) { askSentence(entry) }
    val field = actionField(sentence.action)
    val ink = field.kindInk(c)
    val wash by animateColorAsState(if (landed) ink.copy(alpha = 0.07f) else Color.Transparent, Motion.fade(), label = "landedWash")
    // Landing: the tile grows out of the fold (from 1.6 times its size, 360 ms).
    val pop = remember { Animatable(1f) }
    LaunchedEffect(landed) {
        if (landed && !reduce) {
            pop.snapTo(1.6f)
            pop.animateTo(1f, Motion.fold(false))
        }
    }
    Row(
        modifier
            .fillMaxWidth()
            .background(wash)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 24.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier.weight(1f).heightIn(min = 56.dp).hairlineBelow(c.rule).padding(vertical = 9.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .graphicsLayer {
                        scaleX = pop.value
                        scaleY = pop.value
                    }
                    .testTag("tile:${entry.id}"),
            ) { IntegrationIcon(sentence.provider, sentence.action, size = 38.dp, ring = c.paper, field = field) }
            Column(Modifier.weight(1f)) {
                AskSentenceText(
                    sentence,
                    style = RType.serif(19f, 22f),
                    color = c.ink,
                    detailsColor = c.inkMeta,
                    maxLines = 2,
                    detailsMaxLines = 1,
                    tag = "entryTitle:${entry.id}",
                    detailsTag = "entryDetails:${entry.id}",
                    terms = terms,
                    highlight = hit.title,
                )
                val meta = buildAnnotatedString {
                    append(AnnotatedString(untrusted(entry.connectionLabel)).highlighted(terms, hit.text))
                    append(" · ")
                    append(rowTime(entry.at, nowSeconds))
                    if (entry.grantId != null && entry.decidedBy.isEmpty()) append(" · under a grant")
                    if (showShare) share(entry)?.let { append(" · $it") }
                    if (landed) withStyle(SpanStyle(color = c.violet)) { append(" · ${outcomeLine(entry)}") }
                }
                androidx.compose.material3.Text(meta, color = c.inkMeta, style = RType.meta, maxLines = 1, overflow = TextOverflow.Ellipsis)
                snippet?.let { SnippetLine(it, hit, Modifier.padding(top = 3.dp).testTag("snippet:${entry.id}")) }
                entry.autopilot?.risk?.takeIf { AutopilotText.riskWorthListing(it) }?.let { risk ->
                    dev.reins.android.ui.autopilot.RiskChip(risk, Modifier.padding(top = 4.dp).testTag("rowRisk:${entry.id}"), small = true)
                }
            }
            OutcomeEnd(outcomeOf(entry), Modifier.testTag("outcome:${entry.id}"))
        }
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

/** How an entry ended, at the end of its row: a word in its colour, or a glyph for an approval or a grant. */
@Composable
fun OutcomeEnd(outcome: Outcome, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    when (outcome) {
        Outcome.Approved -> GlyphIcon(Glyph.Check, c.success, modifier = modifier.semantics { this.contentDescription = "Approved" }, size = 18.dp)
        Outcome.Grant -> GlyphIcon(Glyph.Key, c.violet, modifier = modifier.semantics { this.contentDescription = "Grant" }, size = 18.dp)
        Outcome.Auto -> StatusWord("Auto", c.violet, modifier)
        Outcome.Overnight -> StatusWord(AutopilotText.OVERNIGHT, c.violet, modifier)
        Outcome.Denied, Outcome.AutoDenied, Outcome.OvernightDenied -> StatusWord("Denied", c.danger, modifier)
        Outcome.Bypass -> StatusWord("Bypass", c.danger, modifier)
        Outcome.Lockdown -> StatusWord("Lockdown", c.inkMeta, modifier)
        Outcome.Failed -> StatusWord("Failed", c.warning, modifier)
        Outcome.Started -> StatusWord("Awaiting outcome", c.warning, modifier.clearAndSetSemantics { contentDescription = "Attempted; awaiting outcome" })
        Outcome.Unknown -> StatusWord("Outcome unknown", c.warning, modifier.clearAndSetSemantics { contentDescription = "Outcome unknown; do not repeat" })
        Outcome.Withheld -> StatusWord("Response withheld", c.warning, modifier.clearAndSetSemantics { contentDescription = "Completed; response withheld; do not repeat" })
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
