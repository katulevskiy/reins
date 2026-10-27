package dev.reins.android.ui.main

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import dev.reins.android.design.field
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.RowTile
import dev.reins.android.design.PaneRow
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.NextSheet
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
 * A root screen's header (`.ptitle`): the display title (`--s 40`) with its actions at the end, 8 apart. Below the
 * status bar unless [underField] (a field already took the top). Next has no mood line: [mood] is not drawn.
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
            .then(if (underField) Modifier.padding(top = 18.dp) else Modifier.windowInsetsPadding(HeaderInsets).padding(top = 8.dp))
            .padding(horizontal = 20.dp),
    ) {
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            RText(title, RType.title, c.ink, Modifier.weight(1f).semantics { heading() }, maxLines = 1)
            actions()
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
 * The mode chip in a root header (Next: same colour, glyph and name in every header). It opens the one mode sheet
 * ([ModeSheet], drawn over the whole app by the root). While a bypass or an Overnight runs the chip counts down; its
 * clock lives here only.
 */
@Composable
fun HeaderModeChip(settings: AutopilotSettings?, onOpen: () -> Unit) {
    val feedback = LocalFeedback.current
    val mode = settings?.mode ?: AutopilotMode.MANUAL
    val clock = runningClock(settings)
    Box(Modifier.semantics(mergeDescendants = true) { contentDescription = "Autopilot: ${AutopilotText.name(mode)}" }.testTag("modePill")) {
        ModeChip(chipMode(settings), timer = clock, chevron = true) {
            onOpen()
            feedback.play(Event.Open)
        }
    }
}

/** The countdown of the timed mode in force ("47:12", "23:39" from an hour on), ticking each second; null when none runs or it never ends. */
@Composable
private fun runningClock(settings: AutopilotSettings?): String? {
    val until = timedUntil(settings) ?: return null
    if (AutopilotText.isUntilOff(until)) return null
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    return AutopilotText.chipClock(until, now)
}

/**
 * The one mode sheet (Next, Activity 03 and ModePick): Manual, Auto, Overnight, Lockdown and Bypass, each on its
 * field-coloured tile with what it does; Manual, Auto and Lockdown switch at once, Overnight and Bypass open their hold
 * sheets ([onTimed]); a running one shows its clock and stops on tap. Under it, Autopilot on Trust.
 */
@Composable
fun ModeSheet(
    open: Boolean,
    settings: AutopilotSettings?,
    modelReady: Boolean,
    onSet: (AutopilotMode) -> Unit,
    onTimed: (AutopilotMode) -> Unit,
    onStop: (AutopilotMode) -> Unit,
    onSettings: () -> Unit,
    onClose: () -> Unit,
) {
    val c = LocalColors.current
    val mode = settings?.mode ?: AutopilotMode.MANUAL
    val clock = if (open) runningClock(settings) else null
    val needs = { m: AutopilotMode -> if (!modelReady && AutopilotText.needsModel(m)) "Needs the model" else null }
    NextSheet(open, onClose, title = "Mode", paper = true, tag = "autopilotQuick") {
        PaneGroup {
            @Composable
            fun row(m: AutopilotMode, chip: ChipMode, line: String, first: Boolean = false, timed: Boolean = false) {
                val on = mode == m
                val running = on && timed
                PaneRow(
                    title = if (running && clock != null) "${AutopilotText.name(m)} · $clock" else AutopilotText.name(m),
                    subtitle = if (running) "Tap to stop" else needs(m) ?: line,
                    leading = {
                        val field = chip.field()
                        RowTile(chip.glyph ?: Glyph.Person, fill = field?.fill(), tint = if (field == null) c.ink2 else null)
                    },
                    titleColor = if (m == AutopilotMode.BYPASS) c.danger else null,
                    trailing = when {
                        on && !timed -> ({ GlyphIcon(Glyph.Check, c.violet, size = 20.dp) })
                        else -> null
                    },
                    chevron = timed && !on,
                    first = first,
                    tag = "quick:${m.name}",
                ) {
                    when {
                        running -> onStop(m)
                        timed -> onTimed(m)
                        !on -> onSet(m)
                    }
                    onClose()
                }
            }
            row(AutopilotMode.MANUAL, ChipMode.Manual, "You decide", first = true)
            row(AutopilotMode.AUTO, ChipMode.Auto, "Easy ones decided")
            row(AutopilotMode.OVERNIGHT, ChipMode.Overnight, "Risky ones wait", timed = true)
            row(AutopilotMode.LOCKDOWN, ChipMode.Lockdown, "Everything denied")
            row(AutopilotMode.BYPASS, ChipMode.Bypass, "Everything allowed", timed = true)
        }
        PaneGroup(Modifier.padding(top = 12.dp)) {
            PaneRow(
                "Autopilot",
                leading = { RowTile(Glyph.AutopilotFilled, tint = c.violet) },
                chevron = true,
                first = true,
                tag = "quickMore",
            ) {
                onClose()
                onSettings()
            }
        }
    }
}
