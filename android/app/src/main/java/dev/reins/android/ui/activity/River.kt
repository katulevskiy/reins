package dev.reins.android.ui.activity

import androidx.compose.animation.animateColorAsState
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
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
import dev.reins.android.design.SignalChip
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.StatusWord
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.LabelText as RequestLabelText
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ActivityEntry
import dev.reins.core.ConnectionView
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
 * One decided entry: its kind tile, what it did (the row's words), who and when (and "under a grant", Autopilot's
 * share when filtering by it), and how it ended. [landed]: the fold just put it here; its tile is the solid field and
 * the row glows faintly, then both settle to paper.
 */
@Composable
fun RiverRow(entry: ActivityEntry, nowSeconds: Long, landed: Boolean, showShare: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val reduce = LocalReduceMotion.current
    val field = entryField(entry)
    val ink = field.kindInk(c)
    val wash by animateColorAsState(if (landed) ink.copy(alpha = 0.07f) else Color.Transparent, Motion.fade(), label = "landedWash")
    val tileFill by animateColorAsState(if (landed) field.fill() else field.tile(c), Motion.fade(), label = "landedTile")
    val glyphInk by animateColorAsState(if (landed) Color.White else ink, Motion.fade(), label = "landedGlyph")
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
                    .size(38.dp)
                    .graphicsLayer {
                        scaleX = pop.value
                        scaleY = pop.value
                    }
                    .clip(RoundedCornerShape(11.dp))
                    .background(tileFill)
                    .testTag("tile:${entry.id}"),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(entryGlyph(entry), glyphInk, size = 20.dp) }
            Column(Modifier.weight(1f)) {
                val label = entry.label
                if (label != null) {
                    RequestLabelText(label, Modifier.testTag("entryTitle:${entry.id}"), size = 15.5f, lineHeight = 20f, maxLines = 2, color = c.ink)
                } else {
                    RText(historySentence(entry), RType.row, c.ink, Modifier.testTag("entryTitle:${entry.id}"), maxLines = 2)
                }
                val meta = buildAnnotatedString {
                    append(untrusted(entry.connectionLabel))
                    append(" · ")
                    append(rowTime(entry.at, nowSeconds))
                    if (entry.grantId != null && entry.decidedBy.isEmpty()) append(" · under a grant")
                    if (showShare) share(entry)?.let { append(" · $it") }
                    if (landed) withStyle(SpanStyle(color = c.violet)) { append(" · ${outcomeLine(entry)}") }
                }
                androidx.compose.material3.Text(meta, color = c.inkMeta, style = RType.meta, maxLines = 1, overflow = TextOverflow.Ellipsis)
                entry.autopilot?.risk?.takeIf { AutopilotText.riskWorthListing(it) }?.let { risk ->
                    dev.reins.android.ui.autopilot.RiskChip(risk, Modifier.padding(top = 4.dp).testTag("rowRisk:${entry.id}"), small = true)
                }
            }
            OutcomeEnd(outcomeOf(entry), Modifier.testTag("outcome:${entry.id}"))
        }
    }
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
    }
}

/**
 * The filters, as chips under the header: All, Autopilot, Denied, Grants, and "Any AI", a menu of who asked.
 */
@Composable
fun FilterRow(
    selected: HistoryFilter,
    ai: String?,
    connections: List<ConnectionView>,
    modifier: Modifier = Modifier,
    onSelect: (HistoryFilter) -> Unit,
    onAi: (String?) -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var menu by remember { mutableStateOf(false) }
    Row(
        modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 24.dp).testTag("filterRow"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        HistoryFilter.entries.forEach { f ->
            SignalChip(f.label, f == selected, Modifier.testTag("filter:${f.name.lowercase()}")) { if (f != selected) onSelect(f) }
        }
        Box {
            val who = connections.firstOrNull { it.id == ai }?.let { untrusted(it.label) }
            Row(
                Modifier
                    .heightIn(min = 34.dp)
                    .clip(CircleShape)
                    .background(if (ai != null) c.inverse else c.paperRaised)
                    .then(if (ai != null) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
                    .pressable(shape = CircleShape, label = "Any AI") {
                        menu = true
                        feedback.play(Event.Open)
                    }
                    .testTag("filter:ai")
                    .padding(horizontal = 13.dp, vertical = 7.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val fg = if (ai != null) c.onInverse else c.ink
                RText(who ?: "Any AI", RType.sans(14f, FontWeight.Medium), fg, maxLines = 1)
                GlyphIcon(Glyph.ChevronDown, fg, size = 14.dp)
            }
            SignalMenu(
                expanded = menu,
                onDismiss = { menu = false },
                alignEnd = false,
                width = 240.dp,
                items = listOf(MenuItem("Any AI", selected = ai == null, tag = "ai:any") { onAi(null) }) +
                    connections.map { conn -> MenuItem(untrusted(conn.label), selected = ai == conn.id, tag = "ai:${conn.id}") { onAi(conn.id) } },
            )
        }
    }
}
