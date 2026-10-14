package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.AllInclusive
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Remove
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.AutopilotText.BypassUnit
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.HelpText
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.HelpIcon
import dev.reins.android.ui.settings.settingsCard

/** What the "Bypass for" sheet has picked. */
private sealed interface Longer {
    data class Preset(val minutes: UInt) : Longer
    data object Own : Longer
    data object UntilOff : Longer
}

/**
 * Turning a bypass on, from the Autopilot page, an AI's page or the quick menu: how long (15 min, 1 hour, 1 day, or
 * Custom, which opens "Bypass for": 4 hours, 1 week, a number of the user's own, or until turned off), then a hold.
 * [who] names the AI for a bypass of one; null is every AI. [onStart] gets the minutes, or null for until turned off.
 */
@Composable
fun BoxScope.BypassSheets(open: Boolean, who: String?, onStart: (UInt?) -> Unit, onClose: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    // The length picked in the first sheet; [custom] when it came from "Bypass for" (then shown in its segment).
    var minutes by remember { mutableStateOf<UInt?>(60u) }
    var custom by remember { mutableStateOf(false) }
    var moreOpen by remember { mutableStateOf(false) }
    var help by remember { mutableStateOf(false) }
    var longer by remember { mutableStateOf<Longer>(Longer.Own) }
    var count by remember { mutableIntStateOf(3) }
    var unit by remember { mutableStateOf(BypassUnit.Days) }
    LaunchedEffect(open) {
        if (open) {
            minutes = 60u
            custom = false
            moreOpen = false
            help = false
        }
    }
    val longerMinutes: UInt? = when (val l = longer) {
        is Longer.Preset -> l.minutes
        Longer.Own -> AutopilotText.customMinutes(count, unit)
        Longer.UntilOff -> null
    }
    fun start(m: UInt?) {
        moreOpen = false
        onClose()
        onStart(m)
    }

    BottomSheet(open, null, onClose = onClose, tag = "bypassSheet") {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.size(56.dp).clip(RoundedCornerShape(18.dp)).background(c.danger.copy(alpha = 0.16f)), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.Bolt, null, Modifier.size(30.dp), tint = c.danger)
            }
            RText(
                if (who == null) "Bypass every AI?" else "Bypass ${untrusted(who)}?",
                RType.sans(22f, FontWeight.SemiBold),
                c.text,
                Modifier.padding(top = 14.dp).testTag("bypassTitle"),
                maxLines = 2,
                align = TextAlign.Center,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                RText("Approved without asking. Sent means sent.", RType.sans(14f), c.secondary, maxLines = 2)
                HelpIcon(help, "what bypass still asks") { help = it }
            }
            HelpText(AutopilotText.BYPASS_HELP, help, Modifier.padding(bottom = 6.dp))
        }
        LengthSegments(
            minutes = minutes,
            custom = custom,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp),
            onPreset = { m ->
                minutes = m
                custom = false
            },
            onCustom = {
                feedback.play(Event.Selection)
                moreOpen = true
            },
        )
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 18.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CapsuleButton("Cancel", Modifier.weight(1f).testTag("cancelBypass"), style = ButtonStyle.Secondary, onClick = onClose)
            HoldButton(
                if (minutes == null) "Hold to bypass until off" else "Hold to bypass",
                c.danger,
                Modifier.weight(if (minutes == null) 2.4f else 1.35f).testTag("confirmBypass"),
                // The longer label takes the icon's room.
                icon = if (minutes == null) null else Icons.Rounded.Fingerprint,
                height = 50.dp,
            ) { start(minutes) }
        }
        RText("Hold for 2 seconds", RType.sans(12f), c.tertiary, Modifier.fillMaxWidth().padding(top = 8.dp), align = TextAlign.Center)
    }

    BottomSheet(open && moreOpen, "Bypass for", onClose = {
        // Closing keeps the pick: it shows in the Custom segment of the first sheet.
        minutes = longerMinutes
        custom = true
        moreOpen = false
    }, tag = "bypassForSheet") {
        Column(Modifier.padding(horizontal = 16.dp).settingsCard(c)) {
            AutopilotText.bypassMoreMinutes.forEach { m ->
                LongerRow(Longer.Preset(m) == longer, "bypassFor:$m", { longer = Longer.Preset(m) }) {
                    RText(AutopilotText.lengthLabel(m), RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 1)
                }
            }
            LongerRow(longer == Longer.Own, "bypassFor:own", { longer = Longer.Own }) {
                RText("Custom", RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 1)
                Stepper(count, unit.max) {
                    count = it
                    longer = Longer.Own
                }
                Spacer(Modifier.width(6.dp))
                UnitMenu(unit) {
                    unit = it
                    count = count.coerceAtMost(it.max)
                    longer = Longer.Own
                }
                Spacer(Modifier.width(4.dp))
            }
        }
        val untilOff = longer == Longer.UntilOff
        Column(
            Modifier
                .padding(start = 16.dp, end = 16.dp, top = 12.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(24.dp))
                .background(c.danger.copy(alpha = if (untilOff) 0.08f else 0.04f))
                .border(1.dp, c.danger.copy(alpha = 0.32f), RoundedCornerShape(24.dp)),
        ) {
            LongerRow(untilOff, "bypassFor:untilOff", { longer = Longer.UntilOff }, tint = c.danger) {
                Icon(Icons.Rounded.AllInclusive, null, Modifier.size(22.dp), tint = c.danger)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    RText("Until I turn it off", RType.sans(15.5f, FontWeight.SemiBold), c.danger, maxLines = 1)
                    RText("No end time", RType.sans(12.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
                }
            }
        }
        AnimatedVisibility(untilOff, enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
            Row(
                Modifier
                    .padding(start = 16.dp, end = 16.dp, top = 10.dp)
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .background(c.danger.copy(alpha = 0.10f))
                    .border(1.dp, c.danger.copy(alpha = 0.28f), RoundedCornerShape(16.dp))
                    .padding(horizontal = 14.dp, vertical = 12.dp)
                    .testTag("untilOffWarning"),
            ) {
                Icon(Icons.Rounded.Warning, null, Modifier.padding(top = 1.dp).size(18.dp), tint = c.danger)
                Spacer(Modifier.width(10.dp))
                Column {
                    RText("Everything is approved without asking until you stop it.", RType.sans(13.5f, FontWeight.SemiBold, lineHeight = 19f), c.danger)
                    RText("Passwords, deletions and new connections still ask.", RType.sans(13.5f, lineHeight = 19f), c.secondary)
                }
            }
        }
        HoldButton(
            if (untilOff) "Hold to bypass until off" else "Hold to bypass",
            c.danger,
            Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp).fillMaxWidth().testTag("confirmBypassFor"),
            height = 50.dp,
        ) { start(longerMinutes) }
    }
}

/** 15 min | 1 hour | 1 day | Custom ▾, the custom value shown in its segment once picked (red when it has no end). */
@Composable
private fun LengthSegments(minutes: UInt?, custom: Boolean, modifier: Modifier, onPreset: (UInt) -> Unit, onCustom: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(modifier.fillMaxWidth().height(44.dp).clip(CircleShape).background(c.controlFill).padding(4.dp)) {
        AutopilotText.bypassMinutes.forEach { m ->
            val on = !custom && minutes == m
            Segment(on, Modifier.weight(1f).testTag("bypassLength:$m"), onClick = {
                if (!on) {
                    onPreset(m)
                    feedback.play(Event.Selection)
                }
            }) { RText(AutopilotText.lengthLabel(m), RType.sans(14f, if (on) FontWeight.SemiBold else FontWeight.Medium), if (on) c.text else c.secondary, maxLines = 1) }
        }
        val untilOff = custom && minutes == null
        val fg = when {
            untilOff -> c.danger
            custom -> c.text
            else -> c.secondary
        }
        Segment(custom, Modifier.weight(1.25f).testTag("bypassLength:custom"), onClick = onCustom) {
            if (untilOff) {
                Icon(Icons.Rounded.AllInclusive, null, Modifier.size(15.dp), tint = fg)
                Spacer(Modifier.width(3.dp))
            }
            RText(if (custom) AutopilotText.lengthLabel(minutes) else "Custom", RType.sans(14f, if (custom) FontWeight.SemiBold else FontWeight.Medium), fg, maxLines = 1)
            Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(16.dp), tint = fg)
        }
    }
}

@Composable
private fun Segment(on: Boolean, modifier: Modifier, onClick: () -> Unit, content: @Composable () -> Unit) {
    val c = LocalColors.current
    val bg by animateColorAsState(if (on) (if (c.dark) Color.White.copy(alpha = 0.12f) else Color.White) else Color.Transparent, label = "segment")
    Row(
        modifier
            .fillMaxHeight()
            .clip(CircleShape)
            .background(bg)
            .pressable(shape = CircleShape, dim = 0.7f, onClick = onClick)
            .semantics {
                selected = on
                role = Role.Tab
            },
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) { content() }
}

/** A row of "Bypass for" with its radio at the end. */
@Composable
private fun LongerRow(
    selected: Boolean,
    tag: String,
    onClick: () -> Unit,
    tint: Color = LocalColors.current.accent,
    content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        Modifier
            .fillMaxWidth()
            .height(60.dp)
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) {
                if (!selected) feedback.play(Event.Selection)
                onClick()
            }
            .semantics(mergeDescendants = false) {
                this.selected = selected
                role = Role.RadioButton
            }
            .testTag(tag)
            .padding(start = 18.dp, end = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        content()
        RadioDot(selected, tint)
    }
}

/** − 3 + : a number of the user's own. */
@Composable
private fun Stepper(value: Int, max: Int, onChange: (Int) -> Unit) {
    val c = LocalColors.current
    Row(Modifier.height(36.dp).clip(RoundedCornerShape(11.dp)).background(c.controlFill), verticalAlignment = Alignment.CenterVertically) {
        StepButton(Icons.Rounded.Remove, "Fewer", value > 1, "bypassStepper-") { onChange(value - 1) }
        RText("$value", RType.mono(15f, FontWeight.Medium), c.text, Modifier.width(30.dp).testTag("bypassCount"), maxLines = 1, align = TextAlign.Center)
        StepButton(Icons.Rounded.Add, "More", value < max, "bypassStepper+") { onChange(value + 1) }
    }
}

@Composable
private fun StepButton(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, enabled: Boolean, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .size(36.dp)
            .pressable(enabled = enabled, shape = RoundedCornerShape(11.dp), label = label, onClick = onClick)
            .semantics { contentDescription = label }
            .testTag(tag),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, Modifier.size(18.dp), tint = if (enabled) c.secondary else c.tertiary.copy(alpha = 0.5f)) }
}

/** minutes / hours / days / weeks. */
@Composable
private fun UnitMenu(unit: BypassUnit, onPick: (BypassUnit) -> Unit) {
    val c = LocalColors.current
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .height(36.dp)
                .clip(RoundedCornerShape(11.dp))
                .background(c.controlFill)
                .pressable(shape = RoundedCornerShape(11.dp)) { open = true }
                .testTag("bypassUnit")
                .padding(start = 11.dp, end = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText(unit.label, RType.sans(14.5f, FontWeight.SemiBold), c.text, maxLines = 1)
            Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(16.dp), tint = c.secondary)
        }
        DropdownMenu(open, onDismissRequest = { open = false }) {
            BypassUnit.entries.forEach { u ->
                DropdownMenuItem(
                    text = { Text(u.label) },
                    modifier = Modifier.testTag("bypassUnit:${u.name}"),
                    onClick = {
                        open = false
                        onPick(u)
                    },
                )
            }
        }
    }
}
