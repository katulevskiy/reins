package dev.reins.android.ui.trust

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalReduceMotion
import dev.reins.android.design.Motion
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.SectionHeader
import dev.reins.android.ui.activity.RiverRow
import dev.reins.android.ui.activity.ruleAbove
import dev.reins.core.ActivityEntry
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings

/**
 * The list under a running mode's field folds to its latest three, so a long bypass does not push Grants and the
 * profiles out of reach: "Show all 97" opens the rest in place, "Show less" folds it back.
 */
object RunningListFold {
    const val FOLDED = 3

    /** How many rows show. */
    fun shown(total: Int, expanded: Boolean): Int = if (expanded) total else minOf(total, FOLDED)

    /** How many the fold hides (the number on "Show all"); null when it hides none. */
    fun hidden(total: Int): Int? = if (total > FOLDED) total - FOLDED else null

    /** "Show all 97". */
    fun showAll(total: Int): String? = hidden(total)?.let { "Show all $it" }
}

/**
 * What a running bypass, Overnight or lockdown decided, newest first: since it started for a bypass or an Overnight;
 * the latest ones for a lockdown, whose start the core does not keep. Null when none runs.
 */
fun runningEntries(entries: List<ActivityEntry>, s: AutopilotSettings?): List<ActivityEntry>? {
    val (decidedBy, since) = when {
        s == null -> return null
        s.mode == AutopilotMode.BYPASS && s.bypassUntil != null -> "bypass" to s.bypassSince
        s.mode == AutopilotMode.OVERNIGHT && s.overnightUntil != null -> "overnight" to s.overnightSince
        s.mode == AutopilotMode.LOCKDOWN -> "lockdown" to null
        else -> return null
    }
    return entries
        .filter { it.decidedBy == decidedBy && (since == null || it.at >= since) }
        .sortedWith(compareByDescending<ActivityEntry> { it.at }.thenByDescending { it.id })
}

/**
 * Trust's mood line while a bypass or an Overnight runs: what it did today, counted from its list ([listed], from
 * [runningEntries]): "Bypass approved 100 today.", "Overnight decided 12 today.". Null otherwise (a lockdown keeps
 * Autopilot's line).
 */
fun runningMood(s: AutopilotSettings, listed: List<ActivityEntry>?, nowSeconds: Long): String? {
    val (who, did) = when {
        listed == null -> return null
        s.mode == AutopilotMode.BYPASS -> "Bypass" to "approved"
        s.mode == AutopilotMode.OVERNIGHT -> dev.reins.android.autopilot.AutopilotText.OVERNIGHT to "decided"
        else -> return null
    }
    val today = dev.reins.android.ui.activity.dayOf(nowSeconds)
    val n = listed.count { dev.reins.android.ui.activity.dayOf(it.at) == today }
    return if (n == 0) "$who $did nothing yet today." else "$who $did $n today."
}

/** "During bypass", "During Overnight", "Denied" (Autopilot 02-04). */
fun runningListTitle(s: AutopilotSettings): String = when (s.mode) {
    AutopilotMode.BYPASS -> "During bypass"
    AutopilotMode.OVERNIGHT -> "During ${dev.reins.android.autopilot.AutopilotText.OVERNIGHT}"
    else -> "Denied"
}

/**
 * The running mode's list (Autopilot 02-04): a header with the mode's mark and its count in the mode's colour, then
 * its latest three as Activity's rows in a pane, the rest opening in place.
 */
@Composable
fun RunningList(
    s: AutopilotSettings,
    entries: List<ActivityEntry>,
    nowSeconds: Long,
    expanded: Boolean,
    onExpanded: (Boolean) -> Unit,
    onOpen: (Long) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (entries.isEmpty()) return
    val c = LocalColors.current
    val reduce = LocalReduceMotion.current
    val folded = RunningListFold.shown(entries.size, expanded = false)
    val mode = when (s.mode) {
        AutopilotMode.BYPASS -> dev.reins.android.design.ChipMode.Bypass
        AutopilotMode.OVERNIGHT -> dev.reins.android.design.ChipMode.Overnight
        else -> dev.reins.android.design.ChipMode.Lockdown
    }
    val tint = when (s.mode) {
        AutopilotMode.BYPASS -> c.danger
        AutopilotMode.OVERNIGHT -> c.kindNight
        else -> c.warning
    }
    Column(modifier.fillMaxWidth().testTag("runningList")) {
        SectionHeader(runningListTitle(s), glyph = mode.glyph, count = "${entries.size}", countInk = tint)
        val row = @Composable { i: Int, entry: ActivityEntry ->
            RiverRow(entry, nowSeconds, landed = false, showShare = false, Modifier.testTag("runningEntry:${entry.id}"), divider = i > 0) { onOpen(entry.id) }
        }
        PaneGroup {
            entries.take(folded).forEachIndexed { i, entry -> row(i, entry) }
            AnimatedVisibility(
                visible = expanded && entries.size > folded,
                enter = if (reduce) fadeIn(Motion.reduced()) else expandVertically(Motion.fold(false)) + fadeIn(Motion.fold(false)),
                exit = if (reduce) fadeOut(Motion.reduced()) else shrinkVertically(Motion.fold(false)) + fadeOut(Motion.fold(false)),
            ) {
                Column { entries.drop(folded).forEachIndexed { i, entry -> row(folded + i, entry) } }
            }
            RunningListFold.showAll(entries.size)?.let { showAll ->
                val text = if (expanded) "Show less" else showAll
                Row(
                    Modifier
                        .fillMaxWidth()
                        .ruleAbove(c, 14.dp)
                        .heightIn(min = 48.dp)
                        .pressable(label = text) { onExpanded(!expanded) }
                        .testTag(if (expanded) "runningShowLess" else "runningShowAll")
                        .padding(horizontal = 14.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RText(text, RType.sans(14.5f, androidx.compose.ui.text.font.FontWeight.Medium), c.ink2, maxLines = 1)
                    GlyphIcon(if (expanded) Glyph.ChevronUp else Glyph.ChevronDown, c.inkFaint, size = 13.dp)
                }
            }
        }
    }
}
