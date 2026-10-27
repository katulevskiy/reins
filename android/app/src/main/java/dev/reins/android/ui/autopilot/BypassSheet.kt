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
import androidx.compose.foundation.layout.ColumnScope
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
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Info
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.reins.android.design.AskText
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.MenuItem
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalMenu
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.AutopilotText.BypassUnit
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.HelpText
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.HelpIcon
import dev.reins.android.design.hairlineBelow
import dev.reins.core.AutopilotMode

/** What the "Bypass for" sheet has picked. */
private sealed interface Longer {
    data class Preset(val minutes: UInt) : Longer
    data object Own : Longer
    data object UntilOff : Longer
}

/**
 * The modes turned on for a while after a hold and the owner's check, which share these sheets. Each has its own
 * colour, icon, words and test tags (Bypass keeps the tags it always had).
 */
enum class TimedKind(val mode: AutopilotMode, private val tag: String) {
    Bypass(AutopilotMode.BYPASS, "bypass"),
    Overnight(AutopilotMode.OVERNIGHT, "overnight"),
    ;

    private val cap get() = tag.replaceFirstChar(Char::uppercaseChar)

    /** "bypassSheet", "overnightLength:15", … */
    fun tag(part: String): String = tag + part

    /** "confirmBypass", "cancelOvernight", … */
    fun action(verb: String, suffix: String = ""): String = verb + cap + suffix

    val warningTag: String get() = if (this == Bypass) "untilOffWarning" else tag("UntilOffWarning")

    fun tint(c: RColors): Color = modeTint(mode, c)

    val icon: ImageVector get() = modeIcon(mode)

    fun hold(untilOff: Boolean): String = when (this) {
        Bypass -> if (untilOff) "Hold to bypass until off" else "Hold to start"
        Overnight -> if (untilOff) "Hold to start until off" else "Hold to start"
    }

    /** The second sheet's title: "Bypass for", "Overnight for". */
    val forTitle: String get() = "${AutopilotText.name(mode)} for"
}

/**
 * Turning a bypass on, from the Autopilot page, an AI's page or the quick menu: how long (15 min, 1 hour, 1 day, or
 * Custom, which opens "Bypass for": 4 hours, 1 week, a number of the user's own, or until turned off), then a hold.
 * [who] names the AI for a bypass of one; null is every AI. [onStart] gets the minutes, or null for until turned off.
 */
@Composable
fun BoxScope.BypassSheets(open: Boolean, who: String?, onStart: (UInt?) -> Unit, onClose: () -> Unit) {
    TimedSheets(
        TimedKind.Bypass,
        open,
        title = if (who == null) "Bypass" else "Bypass ${dev.reins.android.ui.common.untrustedName(who, 32)}",
        line = "Approved without asking. Sent means sent.",
        helpTopic = "what bypass still asks",
        helpText = AutopilotText.BYPASS_HELP,
        untilOffTitle = "Everything is approved without asking until you stop it.",
        stillAsks = AutopilotText.BYPASS_STILL_ASKS,
        onStart = onStart,
        onClose = onClose,
    )
}

/**
 * Turning Overnight on, like a bypass ([BypassSheets]: how long, a hold, then the owner's check), with how long what it
 * parks waits past the morning ([expiryHours], changed at once with [onExpiry]; no owner check: it only shortens or
 * lengthens a wait that ends "not approved"). Overnight never denies: what Auto would ask is parked for the morning.
 */
@Composable
fun BoxScope.OvernightSheets(
    open: Boolean,
    who: String?,
    expiryHours: UInt,
    onExpiry: (UInt) -> Unit,
    onStart: (UInt?) -> Unit,
    onClose: () -> Unit,
) {
    val c = LocalColors.current
    TimedSheets(
        TimedKind.Overnight,
        open,
        title = AutopilotText.overnightTitle(who),
        line = AutopilotText.OVERNIGHT_LINE,
        helpTopic = AutopilotText.OVERNIGHT,
        helpText = AutopilotText.OVERNIGHT_HELP,
        untilOffTitle = "Decides for you until you stop it.",
        stillAsks = AutopilotText.OVERNIGHT_STILL_ASKS,
        onStart = onStart,
        onClose = onClose,
    ) {
        RText(
            AutopilotText.EXPIRES_AFTER,
            RType.sans(13f, FontWeight.Medium),
            c.secondary,
            Modifier.padding(start = 22.dp, end = 22.dp, top = 16.dp, bottom = 8.dp),
            maxLines = 1,
        )
        // A value the owner set elsewhere (1 to 48 h) shows as its own choice until another is picked.
        val choices = (AutopilotText.expiryChoices + expiryHours).distinct().sorted()
        Segmented(
            choices,
            expiryHours,
            AutopilotText::expiryLabel,
            Modifier.padding(horizontal = 16.dp).testTag("overnightExpiry"),
            tag = { "overnightExpiry:$it" },
            onSelect = onExpiry,
        )
    }
}

/** The sheets of a [TimedKind]: how long and a hold, then "… for" with longer lengths and no end. */
@Composable
private fun BoxScope.TimedSheets(
    kind: TimedKind,
    open: Boolean,
    title: String,
    line: String,
    helpTopic: String,
    helpText: String,
    untilOffTitle: String,
    stillAsks: String,
    onStart: (UInt?) -> Unit,
    onClose: () -> Unit,
    /** Under the lengths, before the hold (Overnight's setting). */
    extra: @Composable ColumnScope.() -> Unit = {},
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val tint = kind.tint(c)
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

    BottomSheet(open, null, onClose = onClose, tag = kind.tag("Sheet")) {
        // Next (Autopilot 01): the mode's name and a round close, what still asks, the lengths and a hold.
        Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 16.dp, top = 6.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            RText(title, RType.sheetTitle, c.ink, Modifier.weight(1f).semantics(mergeDescendants = true) {}.testTag(kind.tag("Title")), maxLines = 2)
            dev.reins.android.design.RoundIconButton(dev.reins.android.design.Glyph.Close, "Close", Modifier.testTag(kind.action("cancel")), onClick = onClose)
        }
        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (kind == TimedKind.Bypass) {
                    // What still asks, as pills; the sentence is what a screen reader hears.
                    Row(
                        Modifier.weight(1f).semantics(mergeDescendants = true) { contentDescription = stillAsks }.testTag(kind.tag("SheetLine")),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RText("Still asks", RType.sans(13.5f, FontWeight.SemiBold), c.ink2, maxLines = 1)
                        dev.reins.android.design.NextPill("New AI", null, glyph = dev.reins.android.design.Glyph.Link)
                        dev.reins.android.design.NextPill("New phone", null, glyph = dev.reins.android.design.Glyph.Phone)
                    }
                } else {
                    RText(line, RType.sans(14.5f, lineHeight = 20f), c.ink2, Modifier.weight(1f).testTag(kind.tag("SheetLine")), maxLines = 2)
                }
                HelpIcon(help, helpTopic) { help = it }
            }
            HelpText(if (kind == TimedKind.Bypass) "$line $helpText" else helpText, help, Modifier.padding(bottom = 6.dp).testTag(kind.tag("Help")))
        }
        LengthSegments(
            kind,
            minutes = minutes,
            custom = custom,
            modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 16.dp),
            onPreset = { m ->
                minutes = m
                custom = false
            },
            onCustom = {
                feedback.play(Event.Selection)
                moreOpen = true
            },
        )
        extra()
        HoldButton(
            kind.hold(untilOff = minutes == null),
            tint,
            Modifier.padding(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 8.dp).fillMaxWidth().testTag(kind.action("confirm")),
            icon = null,
        ) { start(minutes) }
    }

    BottomSheet(open && moreOpen, kind.forTitle, onClose = {
        // Closing keeps the pick: it shows in the Custom segment of the first sheet.
        minutes = longerMinutes
        custom = true
        moreOpen = false
    }, tag = kind.tag("ForSheet")) {
        Column(Modifier.padding(horizontal = 24.dp)) {
            AutopilotText.bypassMoreMinutes.forEach { m ->
                LongerRow(Longer.Preset(m) == longer, kind.tag("For:$m"), { longer = Longer.Preset(m) }) {
                    RText(AutopilotText.lengthLabel(m), RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 1)
                }
            }
            LongerRow(longer == Longer.Own, kind.tag("For:own"), { longer = Longer.Own }) {
                RText("Custom", RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 1)
                Stepper(kind, count, unit.max) {
                    count = it
                    longer = Longer.Own
                }
                Spacer(Modifier.width(6.dp))
                UnitMenu(kind, unit) {
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
                .padding(start = 24.dp, end = 24.dp, top = 12.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(24.dp))
                .background(tint.copy(alpha = if (untilOff) 0.08f else 0.04f))
                .border(1.dp, tint.copy(alpha = 0.32f), RoundedCornerShape(24.dp)),
        ) {
            LongerRow(untilOff, kind.tag("For:untilOff"), { longer = Longer.UntilOff }, tint = tint) {
                Icon(Icons.Rounded.AllInclusive, null, Modifier.size(22.dp), tint = tint)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    RText("Until I turn it off", RType.sans(15.5f, FontWeight.SemiBold), tint, maxLines = 1)
                    RText("No end time", RType.sans(12.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
                }
            }
        }
        AnimatedVisibility(untilOff, enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
            Row(
                Modifier
                    .padding(start = 24.dp, end = 24.dp, top = 10.dp)
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .background(tint.copy(alpha = 0.10f))
                    .border(1.dp, tint.copy(alpha = 0.28f), RoundedCornerShape(16.dp))
                    .padding(horizontal = 14.dp, vertical = 12.dp)
                    .testTag(kind.warningTag),
            ) {
                Icon(if (kind == TimedKind.Bypass) Icons.Rounded.Warning else Icons.Rounded.Info, null, Modifier.padding(top = 1.dp).size(18.dp), tint = tint)
                Spacer(Modifier.width(10.dp))
                Column {
                    RText(untilOffTitle, RType.sans(13.5f, FontWeight.SemiBold, lineHeight = 19f), tint)
                    RText(stillAsks, RType.sans(13.5f, lineHeight = 19f), c.secondary)
                }
            }
        }
        HoldButton(
            kind.hold(untilOff),
            tint,
            Modifier.padding(start = 24.dp, end = 24.dp, top = 16.dp).fillMaxWidth().testTag(kind.action("confirm", "For")),
        ) { start(longerMinutes) }
    }
}

/** 15 min | 1 hour | 1 day | Custom ▾, the custom value shown in its segment once picked (in the mode's colour when it has no end). */
@Composable
private fun LengthSegments(kind: TimedKind, minutes: UInt?, custom: Boolean, modifier: Modifier, onPreset: (UInt) -> Unit, onCustom: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(modifier.fillMaxWidth().height(46.dp).clip(CircleShape).background(c.paperSunken).padding(4.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        AutopilotText.bypassMinutes.forEach { m ->
            val on = !custom && minutes == m
            Segment(on, Modifier.weight(1f).testTag(kind.tag("Length:$m")), onClick = {
                if (!on) {
                    onPreset(m)
                    feedback.play(Event.Selection)
                }
            }) { RText(AutopilotText.lengthLabel(m), RType.sans(14f, FontWeight.Medium), if (on) c.ink else c.ink2, maxLines = 1) }
        }
        val untilOff = custom && minutes == null
        val fg = when {
            untilOff -> kind.tint(c)
            custom -> c.text
            else -> c.secondary
        }
        Segment(custom, Modifier.weight(1.25f).testTag(kind.tag("Length:custom")), onClick = onCustom) {
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
    val bg by animateColorAsState(if (on) c.paperRaised else c.paperRaised.copy(alpha = 0f), label = "segment")
    Row(
        modifier
            .fillMaxHeight()
            .clip(CircleShape)
            .background(bg)
            .then(if (on) Modifier.border(1.dp, c.rule, CircleShape) else Modifier)
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
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp)) {
                if (!selected) feedback.play(Event.Selection)
                onClick()
            }
            .semantics(mergeDescendants = false) {
                this.selected = selected
                role = Role.RadioButton
            }
            .testTag(tag)
            .hairlineBelow(c.rule)
            .padding(start = 4.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        content()
        RadioDot(selected, tint)
    }
}

/** − 3 + : a number of the user's own. */
@Composable
private fun Stepper(kind: TimedKind, value: Int, max: Int, onChange: (Int) -> Unit) {
    val c = LocalColors.current
    Row(Modifier.height(36.dp).clip(RoundedCornerShape(11.dp)).background(c.paperSunken), verticalAlignment = Alignment.CenterVertically) {
        StepButton(Icons.Rounded.Remove, "Fewer", value > 1, kind.tag("Stepper-")) { onChange(value - 1) }
        RText("$value", RType.mono(15f, FontWeight.Medium), c.text, Modifier.width(30.dp).testTag(kind.tag("Count")), maxLines = 1, align = TextAlign.Center)
        StepButton(Icons.Rounded.Add, "More", value < max, kind.tag("Stepper+")) { onChange(value + 1) }
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
private fun UnitMenu(kind: TimedKind, unit: BypassUnit, onPick: (BypassUnit) -> Unit) {
    val c = LocalColors.current
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .height(36.dp)
                .clip(RoundedCornerShape(11.dp))
                .background(c.paperSunken)
                .pressable(shape = RoundedCornerShape(11.dp)) { open = true }
                .testTag(kind.tag("Unit"))
                .padding(start = 11.dp, end = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText(unit.label, RType.sans(14.5f, FontWeight.SemiBold), c.text, maxLines = 1)
            Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(16.dp), tint = c.secondary)
        }
        SignalMenu(
            expanded = open,
            onDismiss = { open = false },
            items = BypassUnit.entries.map { u ->
                MenuItem(u.label, selected = u == unit, tag = kind.tag("Unit:${u.name}")) {
                    open = false
                    onPick(u)
                }
            },
            width = 180.dp,
        )
    }
}
