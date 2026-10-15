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
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material.icons.rounded.ViewHeadline
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
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
import dev.reins.android.ui.common.LabelText
import dev.reins.android.ui.common.ProviderTile
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ActivityEntry

/** The icon of what an operation does, on the history's tiles. */
fun kindIcon(action: String): ImageVector = when (ActionKind.of(action)) {
    ActionKind.Search -> Icons.Rounded.Search
    ActionKind.List -> Icons.AutoMirrored.Rounded.List
    ActionKind.Read -> Icons.Rounded.Visibility
    ActionKind.Headlines -> Icons.Rounded.ViewHeadline
    ActionKind.Write -> Icons.Rounded.Edit
    ActionKind.Send -> Icons.AutoMirrored.Rounded.Send
    ActionKind.Grant -> Icons.Rounded.Key
    ActionKind.Accounts -> Icons.Rounded.People
    ActionKind.Pair -> Icons.Rounded.Computer
    ActionKind.Join -> Icons.Rounded.PhoneAndroid
    ActionKind.Upload -> Icons.Rounded.UploadFile
    ActionKind.Other -> Icons.Rounded.Key
}

/** What an operation does, as a round tile in its colour. */
@Composable
fun KindTile(action: String, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    val c = LocalColors.current
    val tint = toneColor(toneOf(action), c)
    Box(modifier.size(size).clip(CircleShape).background(tint.copy(alpha = 0.15f)), contentAlignment = Alignment.Center) {
        Icon(kindIcon(action), null, Modifier.size(size * 0.5f), tint = tint)
    }
}

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
                    { Icon(Icons.Rounded.Bolt, null, Modifier.size(15.dp), tint = if (on) c.accent else c.secondary) }
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
        verticalAlignment = if (entry.label != null) Alignment.Top else Alignment.CenterVertically,
    ) {
        val label = entry.label
        // What it touched, with what it did on the tile's corner; older test data without a label keeps the kind tile.
        if (label != null) ProviderTile(label, ring = groupFill(c)) else KindTile(entry.action)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f).padding(top = if (label != null) 1.dp else 0.dp)) {
            if (label != null) {
                LabelText(label, Modifier.testTag("entryTitle:${entry.id}"), size = 15f, lineHeight = 19.5f, maxLines = 2)
            } else {
                RText(historySentence(entry), RType.sans(15f, FontWeight.Medium), c.text, Modifier.testTag("entryTitle:${entry.id}"), maxLines = 1)
            }
            Row(Modifier.padding(top = if (label != null) 4.dp else 3.dp), verticalAlignment = Alignment.CenterVertically) {
                ConnectionIcon(entry.connectionId, entry.connectionLabel, size = 15.dp)
                Spacer(Modifier.width(5.dp))
                RText("${untrusted(entry.connectionLabel)} · ${rowTime(entry.at, nowSeconds)}", RType.sans(12.5f), c.tertiary, maxLines = 1)
            }
            // The risk only when it says something (above low, or held); the entry's page always shows it.
            entry.autopilot?.risk?.takeIf { dev.reins.android.autopilot.AutopilotText.riskWorthListing(it) }?.let { risk ->
                dev.reins.android.ui.autopilot.RiskChip(risk, Modifier.padding(top = 5.dp).testTag("rowRisk:${entry.id}"), small = true)
            }
        }
        Spacer(Modifier.width(10.dp))
        OutcomeMark(outcomeOf(entry), Modifier.padding(top = if (label != null) 2.dp else 0.dp).testTag("outcome:${entry.id}"))
    }
}

@Composable
fun OutcomeMark(outcome: Outcome, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    when (outcome) {
        Outcome.Approved -> Icon(Icons.Rounded.CheckCircle, "Approved", modifier.size(19.dp), tint = c.success)
        Outcome.Denied -> Icon(Icons.Rounded.Cancel, "Denied", modifier.size(19.dp), tint = c.danger)
        Outcome.Failed -> Icon(Icons.Rounded.Warning, "Failed", modifier.size(18.dp), tint = c.warning)
        Outcome.Auto -> Marker(Icons.Rounded.Bolt, "Auto", c.accent, modifier)
        Outcome.AutoDenied -> Marker(Icons.Rounded.Bolt, "Auto", c.danger, modifier)
        Outcome.Overnight -> Marker(dev.reins.android.ui.autopilot.OvernightIcon, dev.reins.android.autopilot.AutopilotText.OVERNIGHT, c.night, modifier)
        Outcome.OvernightDenied -> Marker(dev.reins.android.ui.autopilot.OvernightIcon, dev.reins.android.autopilot.AutopilotText.OVERNIGHT, c.danger, modifier)
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
