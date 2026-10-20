package dev.reins.android.ui.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.List
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.FilterList
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.People
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material.icons.rounded.ViewHeadline
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ActionKind
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.AskSentenceText
import dev.reins.android.ui.common.IntegrationIcon
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ActivityEntry

/** A day's header ("Today"), with the filter button at the end of the first one. */
@Composable
fun DayHeader(label: String, modifier: Modifier = Modifier, filterOpen: Boolean? = null, onFilter: () -> Unit = {}) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(start = 22.dp, end = 16.dp, top = 14.dp, bottom = 4.dp).defaultMinSize(minHeight = 40.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(label, RType.sans(14f, FontWeight.Medium), c.accent, Modifier.weight(1f).semantics { heading() }, maxLines = 1)
        if (filterOpen != null) {
            Box(
                Modifier
                    .size(40.dp)
                    .clip(CircleShape)
                    .background(if (filterOpen) c.accent.copy(alpha = 0.16f) else Color.Transparent)
                    .pressable(shape = CircleShape, label = "Filter", onClick = onFilter)
                    .semantics { contentDescription = "Filter" }
                    .testTag("filters"),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Rounded.FilterList, null, Modifier.size(20.dp), tint = c.accent) }
        }
    }
}

/** The filter chips over the history: the shared [dev.reins.android.design.SelectChip], one chosen. */
@Composable
fun FilterChips(selected: HistoryFilter, modifier: Modifier = Modifier, onSelect: (HistoryFilter) -> Unit) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        HistoryFilter.entries.forEach { f ->
            val on = f == selected
            dev.reins.android.design.SelectChip(
                f.label,
                on,
                Modifier.testTag("filter:${f.name.lowercase()}"),
                leading = if (f == HistoryFilter.Autopilot) {
                    { Icon(Icons.Rounded.Shield, null, Modifier.size(15.dp), tint = if (on) c.accent else c.secondary) }
                } else {
                    null
                },
            ) { if (!on) onSelect(f) }
        }
    }
}

/** The background of a history row: rows of a day form one card, with round ends. */
fun rowShape(first: Boolean, last: Boolean): Shape = RoundedCornerShape(
    topStart = if (first) 24.dp else 0.dp,
    topEnd = if (first) 24.dp else 0.dp,
    bottomStart = if (last) 24.dp else 0.dp,
    bottomEnd = if (last) 24.dp else 0.dp,
)

/** The plate of a day's group: raised paper in both themes (Signal `paper-raised`). */
fun groupFill(c: RColors): Color = c.paperRaised

/**
 * Who asked: "Claude Code · M2A" (the agent, then the computer or app it asked through) when the entry names an agent
 * other than the connection, else the connection's name.
 */
internal fun askerLine(entry: ActivityEntry): String {
    val label = untrusted(entry.connectionLabel)
    val agent = entry.request?.agent?.let(::untrusted)?.takeIf { it.isNotBlank() && !it.equals(label, ignoreCase = true) }
    return if (agent != null) "$agent · $label" else label
}

/** One entry of the history: what it did, who and when, and how it ended. */
@Composable
fun HistoryRow(entry: ActivityEntry, nowSeconds: Long, first: Boolean, last: Boolean, modifier: Modifier = Modifier, inset: Dp = 16.dp, onClick: () -> Unit) {
    val c = LocalColors.current
    val shape = rowShape(first, last)
    Row(
        modifier
            .padding(horizontal = inset)
            .fillMaxWidth()
            .clip(shape)
            .background(groupFill(c))
            .pressable(highlight = c.controlFill, shape = shape, onClick = onClick)
            .padding(start = 16.dp, end = 16.dp, top = if (first) 14.dp else 10.dp, bottom = if (last) 14.dp else 10.dp),
        verticalAlignment = Alignment.Top,
    ) {
        // What it touched, with what it did on the icon's corner (ASK-TEXT.md), and the request in words.
        val sentence = remember(entry) { askSentence(entry) }
        IntegrationIcon(sentence.provider, sentence.action, size = 40.dp, ring = groupFill(c))
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f).padding(top = 1.dp)) {
            AskSentenceText(sentence, style = RType.serif(18f, 21f), color = c.text, detailsColor = c.tertiary, maxLines = 2, detailsMaxLines = 1, tag = "entryTitle:${entry.id}")
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                ConnectionIcon(entry.connectionId, entry.connectionLabel, size = 15.dp)
                Spacer(Modifier.width(5.dp))
                RText("${askerLine(entry)} · ${rowTime(entry.at, nowSeconds)}", RType.sans(12.5f), c.tertiary, Modifier.testTag("entryWho:${entry.id}"), maxLines = 1)
            }
            // The risk only when it says something (above low, or held); the entry's page always shows it.
            entry.autopilot?.risk?.takeIf { dev.reins.android.autopilot.AutopilotText.riskWorthListing(it) }?.let { risk ->
                dev.reins.android.ui.autopilot.RiskChip(risk, Modifier.padding(top = 5.dp).testTag("rowRisk:${entry.id}"), small = true)
            }
        }
        Spacer(Modifier.width(10.dp))
        OutcomeMark(outcomeOf(entry), Modifier.padding(top = 2.dp).testTag("outcome:${entry.id}"))
    }
}

@Composable
fun OutcomeMark(outcome: Outcome, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    when (outcome) {
        Outcome.Approved -> Icon(Icons.Rounded.CheckCircle, "Approved", modifier.size(19.dp), tint = c.success)
        Outcome.Denied -> Icon(Icons.Rounded.Cancel, "Denied", modifier.size(19.dp), tint = c.danger)
        Outcome.Failed -> Icon(Icons.Rounded.Warning, "Failed", modifier.size(18.dp), tint = c.warning)
        Outcome.Started -> Marker(Icons.Rounded.Warning, "Awaiting outcome", c.warning, modifier.clearAndSetSemantics { contentDescription = "Attempted; awaiting outcome" })
        Outcome.Unknown -> Marker(Icons.Rounded.Warning, "Outcome unknown", c.warning, modifier.clearAndSetSemantics { contentDescription = "Outcome unknown; do not repeat" })
        Outcome.Withheld -> Marker(Icons.Rounded.Warning, "Response withheld", c.warning, modifier.clearAndSetSemantics { contentDescription = "Completed; response withheld; do not repeat" })
        Outcome.Auto -> Marker(Icons.Rounded.Bolt, "Auto", c.accent, modifier)
        Outcome.AutoDenied -> Marker(Icons.Rounded.Bolt, "Auto", c.danger, modifier)
        Outcome.Overnight -> Marker(dev.reins.android.ui.autopilot.OvernightIcon, dev.reins.android.autopilot.AutopilotText.OVERNIGHT, c.night, modifier)
        Outcome.OvernightDenied -> Marker(dev.reins.android.ui.autopilot.OvernightIcon, dev.reins.android.autopilot.AutopilotText.OVERNIGHT, c.danger, modifier)
        Outcome.Permission -> Marker(Icons.Rounded.Key, "Permission", c.accent, modifier)
        Outcome.Grant -> Marker(Icons.Rounded.Key, "Grant", c.accent, modifier)
        Outcome.Bypass -> Marker(Icons.Rounded.Bolt, "Bypass", c.danger, modifier)
        Outcome.Lockdown -> Marker(Icons.Rounded.Lock, "Lockdown", c.secondary, modifier)
    }
}

@Composable
private fun Marker(icon: ImageVector, label: String, tint: Color, modifier: Modifier) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(15.dp), tint = tint)
        Spacer(Modifier.width(3.dp))
        RText(label, RType.sans(12.5f, FontWeight.SemiBold), tint, maxLines = 1)
    }
}
