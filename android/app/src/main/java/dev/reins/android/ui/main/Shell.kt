package dev.reins.android.ui.main

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.ChipMode
import dev.reins.android.design.DockedTabBar
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.MenuItem
import dev.reins.android.design.MenuTint
import dev.reins.android.design.ModeChip
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.TabItem
import dev.reins.android.design.rememberNowState
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.nav.Tab
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings

// The shell of the four tabs (Reins Signal, And-Core): the docked tab bar, the serif header every root screen shares,
// and the mode chip with its menu, which replaces the old floating switcher.

/** The glyph of each tab. */
val Tab.glyph: Glyph
    get() = when (this) {
        Tab.Activity -> Glyph.Activity
        Tab.Trust -> Glyph.Shield
        Tab.Connections -> Glyph.Link
        Tab.Settings -> Glyph.Settings
    }

/**
 * The docked tab bar: the count of unseen activity on Activity, and the breathing bypass dot on Trust while any bypass
 * runs ([bypassOn]).
 */
@Composable
fun ReinsTabBar(selected: Tab, activityCount: Int, bypassOn: Boolean, onSelect: (Tab) -> Unit, modifier: Modifier = Modifier) {
    val tabs = Tab.entries.map { tab ->
        TabItem(
            tab.label,
            tab.glyph,
            tag = tab.tag,
            badge = if (tab == Tab.Activity) activityCount else 0,
            dot = tab == Tab.Trust && bypassOn,
        )
    }
    DockedTabBar(tabs, selected.ordinal, onSelect = { onSelect(Tab.entries[it]) }, modifier = modifier)
}

/** Whether a bypass runs anywhere: for every AI, or for one of them. */
fun bypassRunning(settings: AutopilotSettings?): Boolean =
    settings != null && (settings.mode == AutopilotMode.BYPASS || settings.connections.any { it.mode == AutopilotMode.BYPASS })

/** The status bar's height, or the boards' 36 dp where the system reports none (the screenshot renderer). */
val HeaderInsets: WindowInsets @Composable get() = WindowInsets.statusBars.union(WindowInsets(top = 36.dp))

/**
 * A root screen's header: the serif title (44) with its actions at the end, and an optional mood line under it in the
 * italic serif ("One thing waits for you."). Below the status bar unless [underField] (a field already took the top).
 */
@Composable
fun RootHeader(
    title: String,
    modifier: Modifier = Modifier,
    mood: AnnotatedString? = null,
    underField: Boolean = false,
    actions: @Composable RowScope.() -> Unit = {},
) {
    val c = LocalColors.current
    Column(
        modifier
            .fillMaxWidth()
            .then(if (underField) Modifier.padding(top = 22.dp) else Modifier.windowInsetsPadding(HeaderInsets).padding(top = 4.dp))
            .padding(horizontal = 24.dp),
    ) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            RText(title, RType.title, c.ink, Modifier.weight(1f).semantics { heading() }, maxLines = 1)
            actions()
        }
        if (mood != null) {
            BasicText(mood, Modifier.padding(top = 4.dp).testTag("mood"), style = MoodStyle.copy(color = c.ink2))
        }
    }
}

/** The mood line: the italic serif at 22 / 28. */
val MoodStyle get() = RType.serif(22f, 28f, italic = true)

/** A mood line whose [lead] (a number word: "One", "Two") stands upright and the rest leans: "*One* thing waits". */
fun mood(lead: String?, rest: String): AnnotatedString = buildAnnotatedString {
    if (lead != null) withStyle(SpanStyle(fontStyle = FontStyle.Normal)) { append(lead) }
    append(rest)
}

/** "One" … "Twelve", then digits. */
fun numberWord(n: Int, capital: Boolean = false): String {
    val w = listOf("no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve").getOrNull(n) ?: n.toString()
    return if (capital) w.replaceFirstChar { it.uppercase() } else w
}

/** What the mode chip shows for [s]: the mode in force (a bypass or an Overnight as such while it runs). */
fun chipMode(s: AutopilotSettings?): ChipMode = when (s?.mode) {
    AutopilotMode.AUTO -> ChipMode.Auto
    AutopilotMode.OVERNIGHT -> ChipMode.Overnight
    AutopilotMode.BYPASS -> ChipMode.Bypass
    AutopilotMode.LOCKDOWN -> ChipMode.Lockdown
    else -> ChipMode.Manual
}

/** The end of the timed mode in force, if one runs. */
private fun timedUntil(s: AutopilotSettings?): Long? = when (s?.mode) {
    AutopilotMode.BYPASS -> s.bypassUntil
    AutopilotMode.OVERNIGHT -> s.overnightUntil
    else -> null
}

/**
 * The mode chip in a root header, and the menu it opens (it grows from the chip): Manual, Auto and Lockdown switch at
 * once; Overnight and Bypass open their hold sheets ([onTimed]); a running one reads "Tap to stop"; the last row opens
 * Autopilot on Trust. While a bypass or an Overnight runs the chip counts down; its clock lives here only.
 */
@Composable
fun HeaderModeChip(
    settings: AutopilotSettings?,
    modelReady: Boolean,
    onSet: (AutopilotMode) -> Unit,
    onTimed: (AutopilotMode) -> Unit,
    onStop: (AutopilotMode) -> Unit,
    onSettings: () -> Unit,
) {
    val feedback = LocalFeedback.current
    var open by remember { mutableStateOf(false) }
    val mode = settings?.mode ?: AutopilotMode.MANUAL
    val chip = chipMode(settings)
    val until = timedUntil(settings)
    val clock = if (until != null && !AutopilotText.isUntilOff(until)) {
        val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
        AutopilotText.clock(until, now)
    } else {
        null
    }
    Box {
        Box(Modifier.semantics(mergeDescendants = true) { contentDescription = "Autopilot: ${AutopilotText.name(mode)}" }.testTag("modePill")) {
            ModeChip(chip, timer = clock, chevron = true) {
                open = true
                feedback.play(Event.Open)
            }
        }
        fun timedRow(m: AutopilotMode, glyph: Glyph, tint: MenuTint, line: String) = if (mode == m) {
            MenuItem(
                if (clock != null) "${AutopilotText.name(m)} · $clock" else AutopilotText.name(m),
                "Tap to stop",
                glyph,
                tint,
                selected = true,
                destructive = m == AutopilotMode.BYPASS,
                tag = "quick:${m.name}",
            ) { onStop(m) }
        } else {
            MenuItem("${AutopilotText.name(m)}…", line, glyph, tint, destructive = m == AutopilotMode.BYPASS, chevron = true, tag = "quick:${m.name}") { onTimed(m) }
        }
        val needs = { m: AutopilotMode -> if (!modelReady && AutopilotText.needsModel(m)) "Needs the model" else null }
        SignalMenu(
            expanded = open,
            onDismiss = { open = false },
            modifier = Modifier.testTag("autopilotQuick"),
            width = 300.dp,
            items = listOf(
                MenuItem("Manual", "You decide, with a hint", Glyph.Person, MenuTint.Plain, selected = mode == AutopilotMode.MANUAL, tag = "quick:MANUAL") { if (mode != AutopilotMode.MANUAL) onSet(AutopilotMode.MANUAL) },
                MenuItem("Auto", needs(AutopilotMode.AUTO) ?: "Decides the easy ones", Glyph.Autopilot, if (mode == AutopilotMode.AUTO) MenuTint.Violet else MenuTint.Plain, selected = mode == AutopilotMode.AUTO, tag = "quick:AUTO") { if (mode != AutopilotMode.AUTO) onSet(AutopilotMode.AUTO) },
                MenuItem("Lockdown", "Deny everything at once", Glyph.Lock, MenuTint.Warn, selected = mode == AutopilotMode.LOCKDOWN, tag = "quick:LOCKDOWN") { if (mode != AutopilotMode.LOCKDOWN) onSet(AutopilotMode.LOCKDOWN) },
                timedRow(AutopilotMode.OVERNIGHT, Glyph.Moon, MenuTint.Violet, needs(AutopilotMode.OVERNIGHT) ?: "Auto while you're away").copy(divider = true),
                timedRow(AutopilotMode.BYPASS, Glyph.Autopilot, MenuTint.Danger, "Approve everything for a while"),
                MenuItem("Autopilot settings", chevron = true, divider = true, tag = "quickMore") { onSettings() },
            ),
        )
    }
}
