package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.ui.unit.sp
import dev.reins.android.design.AskText
import dev.reins.android.design.RoundIconButton
import dev.reins.android.feedback.play
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.AllInclusive
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Science
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.DontMemoize
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.Tag
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowMillis
import dev.reins.android.design.rememberNowState
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.IconPlate
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsPage
import dev.reins.android.ui.settings.SettingsRow
import dev.reins.android.ui.settings.settingsCard
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.ProfileView
import kotlin.math.roundToInt

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.semantics.text
import dev.reins.android.design.ButtonSize
import androidx.compose.ui.draw.drawBehind
import dev.reins.android.design.Field
import dev.reins.android.design.FieldHeaderShape
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.HairlineRow
import dev.reins.android.design.LabelText
import dev.reins.android.design.Pill
import dev.reins.android.design.SignalButton
import dev.reins.android.design.askString
import dev.reins.android.design.hairlineBelow

// Autopilot's part of Trust (Reins Signal, iOS-Trust / And-Core 03): the mode tiles with Lock down, Overnight and
// Bypass, this week's numbers, the profiles, the on-device model and Try it. Each is a section Trust composes; the
// sheets they open live in [AutopilotSheets]. The view model and every lambda are the Autopilot tab's, unchanged.

/** Which of Autopilot's sheets and dialogs are open; Trust keeps one for its sections. */
@androidx.compose.runtime.Stable
class AutopilotSheetsState {
    var bypass by mutableStateOf(false)
    var overnight by mutableStateOf(false)
    var model by mutableStateOf(false)
    var deleteModel by mutableStateOf(false)
    var newProfile by mutableStateOf(false)

    /** A sheet covers the page: the tab bar steps aside. */
    val covered: Boolean get() = bypass || overnight || model
}

/** The global bypass in force, else the global Overnight; null when neither is. */
internal data class Running(val kind: TimedKind, val until: Long, val since: Long?)

internal fun running(s: AutopilotSettings): Running? = when {
    s.mode == AutopilotMode.BYPASS && s.bypassUntil != null -> Running(TimedKind.Bypass, s.bypassUntil!!, s.bypassSince)
    s.mode == AutopilotMode.OVERNIGHT && s.overnightUntil != null -> Running(TimedKind.Overnight, s.overnightUntil!!, s.overnightSince)
    else -> null
}

/**
 * The field a running bypass, Overnight or lockdown puts at the top of Trust (iOS-Trust 03): bypass red, Overnight
 * violet, Lockdown the warn field (amber), as on the mode chip and the menu.
 */
fun trustField(s: AutopilotSettings?): Field? = when {
    s == null -> null
    running(s)?.kind == TimedKind.Overnight -> Field.Night
    running(s) != null -> Field.Delete
    s.mode == AutopilotMode.LOCKDOWN -> Field.Warn
    else -> null
}

/**
 * A running bypass, Overnight or lockdown as a field pinned at the top of Trust (iOS-Autopilot 02, 03): the time left
 * as the hero ("47:12 *left*", or the infinity sign without an end), what it is doing in the serif, what still asks,
 * and the one way out, full width. As the page scrolls ([scrolled], px) it folds into a compact bar ([FoldingField]):
 * the label line and the timer stay, the sentence fades, the button moves to the timer's right. Labels and tags are
 * the same in both states.
 */
@Composable
fun TrustRunningField(
    s: AutopilotSettings,
    fold: RunningFold,
    scrolled: () -> Int,
    onStop: (TimedKind) -> Unit,
    onEndLockdown: () -> Unit,
    /** How many it decided while it runs: the count on the label row (✓ bypass, pause Overnight, ✕ lockdown). */
    decided: Int? = null,
) {
    val c = LocalColors.current
    val field = trustField(s) ?: return
    val run = running(s)
    FoldingField(
        fold,
        scrolled,
        surface = { Box(Modifier.fieldSurface(field, FieldHeaderShape)) },
        label = {
            // Next (Autopilot 02-04): the mode's glyph and name, and on the right how many it decided.
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GlyphIcon(
                    when (run?.kind) {
                        TimedKind.Bypass -> Glyph.Bypass
                        TimedKind.Overnight -> Glyph.Moon
                        null -> Glyph.Block
                    },
                    c.onField,
                    size = 16.dp,
                )
                RText(AutopilotText.name(s.mode), RType.sans(15f, FontWeight.SemiBold), c.onField, Modifier.weight(1f).testTag("runningName"), maxLines = 1)
                if (decided != null && decided > 0) {
                    GlyphIcon(
                        when (run?.kind) {
                            TimedKind.Bypass -> Glyph.Check
                            TimedKind.Overnight -> Glyph.Pause
                            null -> Glyph.Close
                        },
                        c.onField,
                        size = 16.dp,
                    )
                    RText(decided.toString(), RType.sans(15f, FontWeight.SemiBold), c.onField, Modifier.testTag("runningCount"), maxLines = 1)
                }
                if (run != null && AutopilotText.isUntilOff(run.until)) FieldClock(run.kind, run.until)
            }
        },
        clock = if (run != null && !AutopilotText.isUntilOff(run.until)) ({ HeroClock(run.kind, run.until) }) else null,
        hero = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                if (run == null) {
                    androidx.compose.foundation.text.BasicText(
                        askString("Everything that would ask is denied", "is denied"),
                        Modifier.testTag("heroMode"),
                        style = RType.display(32f).copy(color = c.onField),
                    )
                } else {
                    Box(Modifier.testTag("heroMode")) { RunningLine(run.kind, run.until, run.since, s, c.onField2) }
                }
            }
        },
        button = {
            if (run != null) {
                SignalButton(
                    "Stop now",
                    Modifier.testTag(run.kind.action("stop")),
                    style = dev.reins.android.design.ButtonStyle.OnFieldWhite,
                ) { onStop(run.kind) }
            } else {
                SignalButton(
                    "End lockdown",
                    Modifier.testTag("endLockdown"),
                    style = dev.reins.android.design.ButtonStyle.OnFieldWhite,
                    glyph = Glyph.LockOpen,
                    onClick = onEndLockdown,
                )
            }
        },
    )
}

/** The time left as the field's hero: "47:12" in display type at its largest, on its own clock. */
@Composable
private fun HeroClock(kind: TimedKind, until: Long) {
    val c = LocalColors.current
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    RText(AutopilotText.clock(until, now), RType.display(96f).copy(fontFeatureSettings = "tnum"), c.onField, Modifier.testTag(kind.tag("Clock")), maxLines = 1, ltr = true)
}

/** A timed mode without an end, on its field's label row: the infinity sign. */
@Composable
private fun FieldClock(kind: TimedKind, until: Long) {
    val c = LocalColors.current
    GlyphIcon(Glyph.Infinity, c.onField, modifier = Modifier.semantics { contentDescription = "${AutopilotText.name(kind.mode)} until you stop it" }.testTag(kind.tag("UntilOff")), size = 20.dp)
}

/**
 * What still asks while it runs ("New connections and new phones still ask."), or, without an end, "On until you stop
 * it · 2 days so far" (its own clock, so only it recomposes).
 */
@Composable
private fun RunningLine(kind: TimedKind, until: Long, since: Long?, s: AutopilotSettings, color: Color) {
    val now = rememberNowMillis(60_000) / 1000
    if (kind == TimedKind.Bypass && !AutopilotText.isUntilOff(until)) {
        // Next (Autopilot 02): "Still asks" and what still does, as pills; the sentence is what is read out.
        Row(
            Modifier.semantics(mergeDescendants = true) { contentDescription = AutopilotText.BYPASS_STILL_ASKS }.testTag(kind.tag("Line")),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText("Still asks", RType.sans(14f, FontWeight.Medium), color, maxLines = 1)
            dev.reins.android.ui.approval.FieldVerdict("New AI", glyph = Glyph.Link, maxLines = 1)
            dev.reins.android.ui.approval.FieldVerdict("New phone", glyph = Glyph.Phone, maxLines = 1)
        }
        return
    }
    val text = when {
        AutopilotText.isUntilOff(until) -> AutopilotText.runningLine(kind.mode, until, since, now)
        kind == TimedKind.Bypass -> AutopilotText.BYPASS_STILL_ASKS
        else -> AutopilotText.overnightStillAsks(s.overnightWhenRisky)
    }
    RText(text, RType.sans(14.5f), color, Modifier.testTag(kind.tag("Line")), maxLines = 2)
}

/**
 * The mode tiles: Manual and Auto side by side (the one in force ringed), then Lock down and Bypass, and Overnight.
 * While a bypass or an Overnight runs, the field above has them; a lockdown keeps the tiles to pick what follows it.
 */
@Composable
fun AutopilotModes(
    s: AutopilotSettings,
    modelReady: Boolean,
    onPick: (AutopilotMode) -> Unit,
    onBypass: () -> Unit,
    onOvernight: () -> Unit,
    onLockdown: () -> Unit,
    modifier: Modifier = Modifier,
) {
    if (running(s) != null) return
    val lockdown = s.mode == AutopilotMode.LOCKDOWN
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(Modifier.height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(AutopilotMode.MANUAL, AutopilotMode.AUTO).forEach { mode ->
                val needs = !modelReady && AutopilotText.needsModel(mode)
                ModeTile(
                    mode,
                    selected = s.mode == mode,
                    dimmed = needs,
                    line = if (needs) "Needs the model" else AutopilotText.line(mode).let { if (mode == AutopilotMode.MANUAL) "You decide, with a hint" else it },
                    modifier = Modifier.weight(1f).fillMaxHeight(),
                ) { onPick(mode) }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SignalButton(
                if (lockdown) "Locked down" else "Lock down",
                Modifier.weight(1f).testTag("mode:LOCKDOWN"),
                style = dev.reins.android.design.ButtonStyle.Soft,
                size = ButtonSize.Small,
                glyph = Glyph.Lock,
                enabled = !lockdown,
                onClick = onLockdown,
            )
            SignalButton(
                "Bypass…",
                Modifier.weight(1f).testTag("mode:BYPASS"),
                style = dev.reins.android.design.ButtonStyle.DangerWash,
                size = ButtonSize.Small,
                glyph = Glyph.Autopilot,
                onClick = onBypass,
            )
        }
        val c = LocalColors.current
        Row(
            Modifier
                .fillMaxWidth()
                .height(40.dp)
                .clip(CircleShape)
                .background(c.violet.copy(alpha = 0.12f))
                .pressable(shape = CircleShape, label = AutopilotText.OVERNIGHT, onClick = onOvernight)
                .testTag("mode:OVERNIGHT"),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlyphIcon(Glyph.Moon, c.violet, size = 18.dp)
            RText("${AutopilotText.OVERNIGHT}…", RType.sans(14.5f, FontWeight.SemiBold), c.violet, maxLines = 1)
        }
    }
}

/** Manual or Auto as a tile: its glyph, its name in the serif, what it does; the one in force has a 2 dp ring. */
@Composable
private fun ModeTile(mode: AutopilotMode, selected: Boolean, dimmed: Boolean, line: String, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(22.dp)
    val accent = if (mode == AutopilotMode.AUTO) c.violet else c.ink
    val edge by animateColorAsState(if (selected) accent else c.rule, label = "modeEdge")
    Column(
        modifier
            .graphicsLayer { alpha = if (dimmed) 0.5f else 1f }
            .clip(shape)
            .background(c.paperRaised)
            .border(if (selected) 2.dp else 1.dp, edge, shape)
            .pressable(shape = shape, onClick = onClick)
            .semantics(mergeDescendants = true) {
                this.selected = selected
                role = Role.RadioButton
            }
            .testTag("mode:${mode.name}")
            .padding(14.dp),
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            GlyphIcon(if (mode == AutopilotMode.AUTO) Glyph.Autopilot else Glyph.Person, if (mode == AutopilotMode.AUTO) c.violet else c.ink, size = 20.dp)
            Spacer(Modifier.weight(1f))
            if (selected && mode == AutopilotMode.AUTO) GlyphIcon(Glyph.Check, c.violet, size = 18.dp)
        }
        RText(AutopilotText.name(mode), RType.serif(26f, 30f), c.ink, Modifier.padding(top = 8.dp), maxLines = 1)
        RText(line, RType.meta, c.inkMeta, maxLines = 3)
    }
}

/** One number of a strip, with what it counts. */
internal data class Stat(val value: String, val label: String, val color: Color? = null, val suffix: String? = null, val tag: String? = null)

/** Three numbers side by side between two hairlines, in the serif; they roll when they change. */
@Composable
internal fun StatTiles(stats: List<Stat>, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .drawBehind { drawLine(c.rule, androidx.compose.ui.geometry.Offset(0f, 0.5f), androidx.compose.ui.geometry.Offset(size.width, 0.5f), 1.dp.toPx()) }
            .hairlineBelow(c.rule)
            .padding(vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        stats.forEach { stat ->
            Column(
                Modifier
                    .weight(1f)
                    .semantics(mergeDescendants = true) { text = AnnotatedString(stat.value + (stat.suffix ?: "")) }
                    .then(if (stat.tag != null) Modifier.testTag(stat.tag) else Modifier),
            ) {
                Row(verticalAlignment = Alignment.Bottom) {
                    dev.reins.android.design.RollingNumber(stat.value, RType.serif(32f, 34f), stat.color ?: c.ink)
                    stat.suffix?.let { RText(it, RType.serif(20f, 24f), c.inkMeta, Modifier.padding(start = 2.dp), maxLines = 1) }
                }
                RText(stat.label, RType.meta, c.inkMeta, maxLines = 1)
            }
        }
    }
}

/**
 * Autopilot's lines on Trust (Next): its error and notice, and, without a usable model, getting it. The week's
 * numbers ([withStats]) are no longer on the page.
 */
@Composable
fun AutopilotWeek(viewModel: AutopilotViewModel, s: AutopilotSettings, withStats: Boolean = false) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val model = ui.model ?: s.model
    ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "autopilotError") }
    ui.notice?.let { Banner(it, Modifier.padding(top = 10.dp), tag = "autopilotNotice") }
    if (!AutopilotText.modelUsable(model)) {
        GetScoreCard(
            model, ui.job, s.wifiOnly,
            onDownload = @DontMemoize { viewModel.download() },
            onPause = @DontMemoize { viewModel.cancelDownload() },
        )
    } else if (withStats) {
        StatTiles(
            listOf(
                Stat("${ui.stats?.scored ?: "—"}", "scored this week", tag = "statScored"),
                Stat(AutopilotText.share(ui.stats?.matched), "matched you", c.success, tag = "statMatched"),
                Stat("${ui.stats?.kindsOnAuto ?: "—"}", "kinds on Auto", tag = "statKinds"),
            ),
            Modifier.padding(top = 8.dp),
        )
    }
}

/** A section's label in mono capitals, with something at its end ("Ended", "New"). */
@Composable
fun TrustLabel(text: String, modifier: Modifier = Modifier, trailing: (@Composable () -> Unit)? = null) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().heightIn(min = 22.dp).semantics { heading() }, verticalAlignment = Alignment.CenterVertically) {
        LabelText(text, c.inkMeta, Modifier.weight(1f))
        trailing?.invoke()
    }
}

/** A quiet word at the end of a section's label that opens something ("Ended", "New"). */
@Composable
fun LabelLink(text: String, tag: String, glyph: Glyph? = null, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.clip(CircleShape).pressable(shape = CircleShape, label = text, onClick = onClick).testTag(tag).padding(horizontal = 6.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        glyph?.let { GlyphIcon(it, c.ink2, size = 14.dp) }
        RText(text, RType.sans(13f), c.ink2, maxLines = 1)
    }
}

/** The profiles: each one's glyph, name, who it learns from, and Default. */
@Composable
fun ProfilesSection(viewModel: AutopilotViewModel, s: AutopilotSettings, connections: List<dev.reins.core.ConnectionView>, sheets: AutopilotSheetsState, onProfile: (String) -> Unit) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    TrustLabel("Profiles", Modifier.padding(top = 26.dp, bottom = 2.dp)) { LabelLink("New", "newProfile", Glyph.Plus) { sheets.newProfile = true } }
    val labels = connections.map { it.id to it.label }
    ui.profiles.forEach { profile ->
        ProfileRow(profile, AutopilotText.profileLine(profile, AutopilotText.profileAis(profile, s, labels))) { onProfile(profile.id) }
    }
}

/** The on-device model (its version and when it was updated, how far a download is) and Try it. */
@Composable
fun ModelRows(viewModel: AutopilotViewModel, s: AutopilotSettings, sheets: AutopilotSheetsState, onTryIt: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val model = ui.model ?: s.model
    val modelReady = AutopilotText.modelUsable(model)
    TrustLabel("Autopilot", Modifier.padding(top = 26.dp, bottom = 2.dp))
    val now = rememberNowMillis(60_000) / 1000
    HairlineRow(
        "On-device model",
        Modifier.testTag("openModel"),
        // When it was updated is the row's second line (it wraps); the value on the right stays short ("v2").
        subtitle = model.takeIf { it.state == ModelState.INSTALLED }?.let { AutopilotText.updatedAgo(it.updatedAt, now) },
        leading = { PlainTile(Glyph.Chip) },
        trailing = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ModelValue(model, ui.job)
                GlyphIcon(Glyph.ChevronRight, c.inkFaint, modifier = Modifier.padding(start = 6.dp), size = 14.dp)
            }
        },
    ) { sheets.model = true }
    HairlineRow(
        "Try it",
        Modifier.testTag("openTryIt"),
        leading = { PlainTile(Glyph.Flask) },
        trailing = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (!modelReady) RText("after download", RType.sans(14.5f), c.inkMeta, maxLines = 1)
                GlyphIcon(Glyph.ChevronRight, c.inkFaint, modifier = Modifier.padding(start = 6.dp), size = 14.dp)
            }
        },
        onClick = onTryIt,
    )
}

/** A monochrome tile: nothing here asks anything (38 dp, radius 11, the sunken paper). */
@Composable
fun PlainTile(glyph: Glyph, size: androidx.compose.ui.unit.Dp = 38.dp) {
    val c = LocalColors.current
    Box(Modifier.size(size).clip(RoundedCornerShape(if (size >= 38.dp) 11.dp else 10.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
        GlyphIcon(glyph, c.ink, size = size * 0.53f)
    }
}

/**
 * Autopilot's sheets over Trust: Bypass and Overnight (held to start), the model's details, deleting the model, and
 * a new profile.
 */
@Composable
fun BoxScope.AutopilotSheets(viewModel: AutopilotViewModel, sheets: AutopilotSheetsState, authenticator: Authenticator, onProfile: (String) -> Unit) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val model = ui.model ?: settings?.model
    // v0.15.0's optimized sheet host retrieved an AutopilotViewModel from a memoized callback slot on
    // recomposition (ClassCastException). These event handlers capture unstable receivers: do not let the
    // compiler memoize them. Keep the model, timed-mode and profile handlers consistent with one another.
    BypassSheets(
        sheets.bypass,
        who = null,
        onStart = @DontMemoize { minutes -> viewModel.startBypass(minutes, authenticator = authenticator) },
        onClose = @DontMemoize { sheets.bypass = false },
    )
    OvernightSheets(
        sheets.overnight,
        who = null,
        risky = settings?.overnightWhenRisky ?: dev.reins.core.OvernightRisky.WAIT,
        ratesRisk = model?.risk == true,
        onRisky = @DontMemoize { viewModel.setOvernightWhenRisky(it, authenticator) },
        onStart = @DontMemoize { minutes -> viewModel.startOvernight(minutes, authenticator = authenticator) },
        onClose = @DontMemoize { sheets.overnight = false },
    )
    BottomSheet(sheets.model, null, onClose = @DontMemoize { sheets.model = false }, tag = "modelSheet") {
        // Honest about size and place (iOS-Autopilot 06): "On-device *model*", then the model on its plate.
        Row(Modifier.fillMaxWidth().padding(start = 22.dp, end = 16.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            AskText("On-device model", LocalColors.current.ink, Modifier.weight(1f), target = "model", style = RType.serif(30f, 34f), minSize = 24.sp, maxLines = 1)
            RoundIconButton(Glyph.Close, "Close", Modifier.testTag("closeSheet"), size = 36.dp, onClick = @DontMemoize { sheets.model = false })
        }
        ModelDetails(
            model,
            job = ui.job,
            wifiOnly = settings?.wifiOnly ?: true,
            onDownload = @DontMemoize { viewModel.download() },
            onCancel = @DontMemoize { viewModel.cancelDownload() },
            onDelete = @DontMemoize { sheets.deleteModel = true },
            onWifiOnly = @DontMemoize { viewModel.setWifiOnly(it) },
            autoUpdate = settings?.autoUpdate ?: true,
            onAutoUpdate = @DontMemoize { viewModel.setAutoUpdate(it) },
            checking = ui.checking,
            checkResult = ui.checkResult,
            onCheckNow = @DontMemoize { viewModel.checkNow() },
        )
    }
    if (sheets.deleteModel) {
        dev.reins.android.design.SignalDialog(
            title = "Delete the model?",
            text = "Manual asks without hints and Auto pauses until you download it again.",
            confirmLabel = "Delete",
            destructive = true,
            onConfirm = @DontMemoize {
                sheets.deleteModel = false
                sheets.model = false
                viewModel.deleteModel()
            },
            onDismiss = @DontMemoize { sheets.deleteModel = false },
        )
    }
    if (sheets.newProfile) {
        ProfileDialog(
            title = "New profile",
            initialName = "",
            initialIcon = AutopilotText.icons[2],
            confirmLabel = "Create",
            onConfirm = @DontMemoize { name, icon ->
                sheets.newProfile = false
                viewModel.createProfile(name, icon, onProfile)
            },
            onDismiss = @DontMemoize { sheets.newProfile = false },
        )
    }
}

/** What reads on a timed mode's solid colour: near black on the light dark-mode tints, white on the others. */
internal fun onTimed(kind: TimedKind, c: dev.reins.android.design.RColors): Color = when {
    !c.dark -> Color.White
    kind == TimedKind.Bypass -> Color(0xFF2A0606)
    else -> Color(0xFF0E1230)
}

/** A section's name in the accent colour, with something at its end ("+ New"), on the pages Trust opens. */
@Composable
internal fun SectionHeader(text: String, modifier: Modifier = Modifier, trailing: (@Composable () -> Unit)? = null) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(start = 22.dp, end = 14.dp, top = 22.dp, bottom = 8.dp).heightIn(min = 22.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(text, RType.sans(14f, FontWeight.Medium), c.accent, Modifier.weight(1f).semantics { heading() }, maxLines = 1)
        trailing?.invoke()
    }
}


/**
 * The model row's value: its version ("v2"; when it was updated is the row's second line), how far a download is (a
 * newer model updating by itself too), or Download.
 */
@Composable
private fun ModelValue(model: ModelStatus?, job: DownloadJob) {
    val c = LocalColors.current
    when {
        model == null -> Unit
        model.state == ModelState.INSTALLED -> Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("modelInstalled")) {
            Icon(Icons.Rounded.CheckCircle, null, Modifier.size(16.dp), tint = c.success)
            Spacer(Modifier.width(5.dp))
            RText(AutopilotText.modelVersion(model), RType.sans(15f, FontWeight.Medium), c.secondary, Modifier.testTag("modelVersion"), maxLines = 1)
        }
        model.state == ModelState.DOWNLOADING || job == DownloadJob.Running ->
            RText(AutopilotText.fraction(model)?.let(AutopilotText::percent) ?: "…", RType.mono(14f), c.secondary, maxLines = 1)
        else -> RText("Download", RType.sans(15f, FontWeight.Medium), c.accent, maxLines = 1)
    }
}

/** Without the model: what it is for, its size, and getting it (with the progress while it comes). */
@Composable
private fun GetScoreCard(model: ModelStatus, job: DownloadJob, wifiOnly: Boolean, onDownload: () -> Unit, onPause: () -> Unit) {
    val c = LocalColors.current
    val downloading = model.state == ModelState.DOWNLOADING || job == DownloadJob.Running
    val waiting = job == DownloadJob.Waiting && !downloading
    val shape = RoundedCornerShape(24.dp)
    Column(
        Modifier
            .padding(top = 12.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(c.paperRaised)
            .border(1.dp, c.rule, shape)
            .padding(14.dp)
            .testTag("getScore"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            PlainTile(Glyph.Chip)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText("Get Autopilot's score", RType.row, c.ink, maxLines = 1)
                RText("${AutopilotText.modelSize(model)} · runs on this phone", RType.meta, c.inkMeta, maxLines = 1)
            }
        }
        if (downloading || waiting) {
            val fraction = AutopilotText.fraction(model)
            if (fraction != null) MeterBar(fraction, c.search, Modifier.padding(top = 12.dp).testTag("modelProgress")) else IndeterminateBar(Modifier.padding(top = 12.dp).testTag("modelProgress"))
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(
                    if (waiting) AutopilotText.modelState(model, true, wifiOnly) else AutopilotText.progressText(model),
                    RType.sans(12.5f),
                    c.secondary,
                    Modifier.weight(1f).testTag("modelState"),
                    maxLines = 1,
                )
                if (waiting) {
                    RText("Pause", RType.sans(13f, FontWeight.Medium), c.accent, Modifier.clip(CircleShape).pressable(shape = CircleShape, onClick = onPause).testTag("pauseDownload").padding(horizontal = 6.dp, vertical = 2.dp), maxLines = 1)
                }
            }
        } else {
            if (model.state == ModelState.FAILED) {
                RText(AutopilotText.modelError(model.error), RType.sans(13f), c.danger, Modifier.padding(top = 10.dp).testTag("getModelError"))
            }
            SignalButton(
                if (model.state == ModelState.FAILED) "Try again" else "Download",
                Modifier.padding(top = 12.dp).fillMaxWidth().testTag("getModel"),
                size = ButtonSize.Small,
                glyph = Glyph.Download,
                onClick = onDownload,
            )
        }
    }
}

@Composable
private fun ProfileRow(profile: ProfileView, line: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .hairlineBelow(c.rule)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .semantics(mergeDescendants = true) {}
            .testTag("profile:${profile.id}")
            .padding(vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ProfileTile(profile, 38.dp)
        Column(Modifier.weight(1f)) {
            RText(untrusted(profile.name), RType.row, c.ink, maxLines = 1)
            RText(line, RType.meta, c.inkMeta, Modifier.testTag("profileLine:${profile.id}"), maxLines = 2)
        }
        if (profile.isDefault) Pill("Default", c.violet)
    }
}

/** A profile's glyph on a monochrome tile (the icon it keeps is shown as its glyph). */
@Composable
fun ProfileTile(profile: ProfileView, size: androidx.compose.ui.unit.Dp) {
    PlainTile(profileGlyph(AutopilotText.profileIcon(profile)), size)
}

/** The glyph shown for a profile's icon: the icons it can keep are drawn as glyphs, never as emoji. */
fun profileGlyph(icon: String?): Glyph = when (icon) {
    "💼" -> Glyph.Briefcase
    "🏠" -> Glyph.Home
    "🧪" -> Glyph.Flask
    "🚀" -> Glyph.Rocket
    "🔒" -> Glyph.Lock
    "🌙" -> Glyph.Moon
    "🎓" -> Glyph.Star
    else -> Glyph.Person
}

/** The on-device model: what it is, whether it is here, getting it and removing it (set-up's card). */
@Composable
internal fun ModelCard(
    model: ModelStatus?,
    job: DownloadJob,
    wifiOnly: Boolean,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onDelete: () -> Unit,
    onWifiOnly: (Boolean) -> Unit,
    autoUpdate: Boolean,
    onAutoUpdate: (Boolean) -> Unit,
    checking: Boolean,
    checkResult: String?,
    onCheckNow: () -> Unit,
) {
    Group(
        header = "On-device model",
        footer = "Runs on this phone, nothing leaves it. Without it, every request is left to you, without hints.",
    ) {
        ModelDetails(model, job, wifiOnly, onDownload, onCancel, onDelete, onWifiOnly, autoUpdate, onAutoUpdate, checking, checkResult, onCheckNow)
    }
}

/**
 * The model's details: version and size, its state, getting or deleting it, updating by itself and Wi-Fi only, and
 * behind More, "Check now".
 */
@Composable
internal fun ModelDetails(
    model: ModelStatus?,
    job: DownloadJob,
    wifiOnly: Boolean,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onDelete: () -> Unit,
    onWifiOnly: (Boolean) -> Unit,
    autoUpdate: Boolean,
    onAutoUpdate: (Boolean) -> Unit,
    checking: Boolean,
    checkResult: String?,
    onCheckNow: () -> Unit,
) {
    val c = LocalColors.current
    if (model == null) return
    val downloading = model.state == ModelState.DOWNLOADING || job == DownloadJob.Running
    val waiting = job == DownloadJob.Waiting && model.state != ModelState.DOWNLOADING
    Column(Modifier.padding(horizontal = 22.dp, vertical = 8.dp).testTag("modelCard")) {
        // The model on its plate: its glyph, name, version and size, and a word for where it is.
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(c.paperSunken).padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            PlainTile(Glyph.Chip, 40.dp)
            Column(Modifier.weight(1f)) {
                RText(model.label, RType.row, c.ink, maxLines = 2)
                RText("Version ${model.version} · ${AutopilotText.modelSize(model)}", RType.meta, c.inkMeta, Modifier.padding(top = 2.dp), maxLines = 2)
            }
            when {
                downloading -> Unit
                waiting -> Pill("Waiting", c.warning)
                model.state == ModelState.INSTALLED -> Pill("Ready", c.success)
                model.state == ModelState.FAILED -> Pill("Failed", c.danger)
                else -> Unit
            }
        }
        RText(
            AutopilotText.modelState(model, waiting, wifiOnly),
            RType.sans(14f, FontWeight.Medium),
            if (model.state == ModelState.FAILED) c.danger else c.ink2,
            Modifier.padding(top = 12.dp),
        )
        if (downloading) {
            val fraction = AutopilotText.fraction(model)
            if (fraction != null) {
                MeterBar(fraction, c.violet, Modifier.padding(top = 10.dp), height = 8.dp)
            } else {
                IndeterminateBar(Modifier.padding(top = 10.dp))
            }
        }
        if (model.state == ModelState.FAILED) {
            Banner(AutopilotText.modelError(model.error), Modifier.padding(top = 12.dp), BannerKind.Error)
        }
    }
    SwitchRow("Update automatically", autoUpdate, onAutoUpdate, tag = "autoUpdate")
    SwitchRow(
        "Download on Wi-Fi only",
        wifiOnly,
        onWifiOnly,
        subtitle = "Updates wait for Wi-Fi",
        tag = "wifiOnly",
    )
    RText(
        "Runs on this phone; nothing leaves it. Without it every request is left to you, without hints.",
        RType.meta,
        c.inkMeta,
        Modifier.padding(horizontal = 22.dp, vertical = 10.dp),
    )
    Column(Modifier.padding(horizontal = 22.dp).padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        when {
            downloading -> Unit
            waiting -> SignalButton("Cancel", Modifier.fillMaxWidth().testTag("cancelDownload"), style = dev.reins.android.design.ButtonStyle.Soft, onClick = onCancel)
            model.state == ModelState.INSTALLED ->
                SignalButton("Delete model", Modifier.fillMaxWidth().testTag("deleteModel"), style = dev.reins.android.design.ButtonStyle.DangerWash, onClick = onDelete)
            else -> SignalButton(
                if (model.state == ModelState.FAILED) "Try again" else "Download",
                Modifier.fillMaxWidth().testTag("downloadModel"),
                glyph = if (model.state == ModelState.FAILED) Glyph.Refresh else Glyph.Download,
                onClick = onDownload,
            )
        }
    }
    if (AutopilotText.modelUsable(model)) CheckNowMore(checking, model.state == ModelState.DOWNLOADING, checkResult, onCheckNow)
}

/**
 * "More" under the model's details (closed at first): "Check now" and what it found. While a newer model downloads,
 * its bytes show on the card above.
 */
@Composable
private fun CheckNowMore(checking: Boolean, downloading: Boolean, result: String?, onCheckNow: () -> Unit) {
    val c = LocalColors.current
    val feedback = dev.reins.android.feedback.LocalFeedback.current
    var open by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.padding(horizontal = 22.dp).padding(top = 6.dp)) {
        Row(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(14.dp))
                .pressable(highlight = c.controlFill, shape = RoundedCornerShape(14.dp)) {
                    open = !open
                    feedback.play(dev.reins.android.feedback.Event.expand(open))
                }
                .semantics { role = Role.Button }
                .testTag("modelMore")
                .padding(vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText("More", RType.sans(15f, FontWeight.Medium), c.accent, Modifier.weight(1f), maxLines = 1)
            Icon(
                if (open) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore,
                null,
                Modifier.size(20.dp),
                tint = c.accent,
            )
        }
        if (open) {
            SignalButton(
                "Check now",
                Modifier.fillMaxWidth().testTag("checkNow"),
                style = ButtonStyle.Soft,
                enabled = !checking && !downloading,
                onClick = onCheckNow,
            )
            if (checking && !downloading) {
                IndeterminateBar(Modifier.padding(top = 10.dp).testTag("checking"))
            } else if (result != null && !checking) {
                RText(result, RType.sans(13f), c.secondary, Modifier.padding(top = 8.dp).testTag("checkResult"))
            }
        }
    }
}

/** A bar for a download whose size is not known: a short band sweeping across. Still when clocks are frozen. */
@Composable
private fun IndeterminateBar(modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val live = dev.reins.android.design.LocalLiveTimers.current
    // Read only where the band is placed: the sweep moves it every frame without recomposing or measuring anything.
    val phase = if (live) {
        androidx.compose.animation.core.rememberInfiniteTransition(label = "sweep").animateFloat(
            0f,
            1f,
            androidx.compose.animation.core.infiniteRepeatable(tween(1300)),
            label = "sweepPhase",
        )
    } else {
        null
    }
    androidx.compose.foundation.layout.BoxWithConstraints(
        modifier.fillMaxWidth().height(8.dp).clip(CircleShape).background(c.controlFill.copy(alpha = if (c.dark) 0.12f else 0.09f)),
    ) {
        val band = maxWidth * 0.35f
        Box(
            Modifier
                .offset { IntOffset((((maxWidth + band) * (phase?.value ?: 0.35f) - band).toPx()).roundToInt(), 0) }
                .width(band)
                .height(8.dp)
                .clip(CircleShape)
                .background(c.accent),
        )
    }
}

/**
 * Naming a profile and picking its icon (new and rename). The icons are glyphs, never emoji; the one picked is
 * inverse. Each keeps the icon it stores as its key.
 */
@Composable
fun ProfileDialog(
    title: String,
    initialName: String,
    initialIcon: String?,
    confirmLabel: String,
    onConfirm: (String, String?) -> Unit,
    onDismiss: () -> Unit,
) {
    val c = LocalColors.current
    val feedback = dev.reins.android.feedback.LocalFeedback.current
    var name by remember { mutableStateOf(initialName) }
    var icon by remember { mutableStateOf(initialIcon) }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        val shape = RoundedCornerShape(28.dp)
        Column(
            Modifier
                .padding(horizontal = 24.dp)
                .fillMaxWidth()
                .clip(shape)
                .background(c.paperRaised)
                .border(1.dp, c.rule, shape)
                .padding(22.dp)
                .testTag("profileDialog"),
        ) {
            RText(title, RType.serif(28f, 32f), c.ink)
            RTextField(name, { name = it.take(40) }, "Name, e.g. Side project", Modifier.padding(top = 16.dp), tag = "profileName")
            dev.reins.android.design.LabelText("Icon", c.inkMeta, Modifier.padding(top = 18.dp, bottom = 10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                AutopilotText.icons.forEach { e ->
                    val on = e == icon
                    Box(
                        Modifier
                            .weight(1f)
                            .aspectRatio(1f)
                            .clip(RoundedCornerShape(12.dp))
                            .background(if (on) c.inverse else c.paperSunken)
                            .pressable(shape = RoundedCornerShape(12.dp), label = profileIconName(e)) {
                                if (!on) feedback.play(dev.reins.android.feedback.Event.Selection)
                                icon = e
                            }
                            .semantics {
                                selected = on
                                contentDescription = profileIconName(e)
                            }
                            .testTag("icon:$e"),
                        contentAlignment = Alignment.Center,
                    ) { GlyphIcon(profileGlyph(e), if (on) c.onInverse else c.ink, size = 18.dp) }
                }
            }
            Row(Modifier.padding(top = 22.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Outline, onClick = onDismiss)
                SignalButton(confirmLabel, Modifier.weight(1f).testTag("saveProfile"), enabled = name.isNotBlank()) { onConfirm(name, icon) }
            }
        }
    }
}

/** What a screen reader says for a profile's icon. */
fun profileIconName(icon: String?): String = when (profileGlyph(icon)) {
    Glyph.Briefcase -> "Work"
    Glyph.Home -> "Home"
    Glyph.Flask -> "Lab"
    Glyph.Rocket -> "Rocket"
    Glyph.Lock -> "Lock"
    Glyph.Moon -> "Night"
    Glyph.Star -> "Star"
    else -> "Person"
}
