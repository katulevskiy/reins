package dev.reins.android.ui.autopilot

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import dev.reins.android.design.OnWarning
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.Icons
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.material.icons.rounded.AllInclusive
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.reins.android.BuildConfig
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ModelState
import dev.reins.core.OvernightRisky
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * What the round Autopilot button opens. [Menu] and [Slider] are compact, over the screen; [Page] is the full Autopilot
 * page. Debug builds can switch between them (Settings, Developer); release builds use [DEFAULT].
 */
enum class QuickStyle(val label: String) {
    Menu("Menu"),
    Slider("Slider"),
    Page("Page"),
    ;

    companion object {
        val DEFAULT = Menu
    }
}

/** The debug switch between the [QuickStyle]s, kept on this phone. */
object QuickStyles {
    private const val PREFS = "dev"
    private const val KEY = "autopilot.quick"
    private var state: MutableState<QuickStyle>? = null

    fun state(context: Context): MutableState<QuickStyle> = state ?: mutableStateOf(load(context)).also { state = it }

    fun set(context: Context, style: QuickStyle) {
        prefs(context).edit().putString(KEY, style.name).apply()
        state(context).value = style
    }

    private fun load(context: Context): QuickStyle {
        if (!BuildConfig.DEBUG) return QuickStyle.DEFAULT
        val name = prefs(context).getString(KEY, null)
        return QuickStyle.entries.firstOrNull { it.name == name } ?: QuickStyle.DEFAULT
    }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}

/**
 * The compact Autopilot switcher, rising from the round button at the bottom right. A tap outside or Back closes it;
 * "More" opens the full page. Lockdown is one tap (it only takes power away); Bypass and Overnight open their sheets
 * (how long, then a hold), and [onBypass] / [onOvernight] get the minutes (null: until turned off).
 */
@Composable
fun AutopilotQuick(
    open: Boolean,
    style: QuickStyle,
    settings: AutopilotSettings?,
    onSet: (AutopilotMode, UInt?) -> Unit,
    onStopBypass: () -> Unit,
    onMore: () -> Unit,
    onDismiss: () -> Unit,
    onBypass: (UInt?) -> Unit = {},
    onOvernight: (UInt?) -> Unit = {},
    onStopOvernight: () -> Unit = {},
    onWhenRisky: (OvernightRisky) -> Unit = {},
) {
    val c = LocalColors.current
    val shown = remember { MutableTransitionState(false) }
    var bypassSheet by remember { mutableStateOf(false) }
    var overnightSheet by remember { mutableStateOf(false) }
    shown.targetState = open && settings != null
    if (!shown.currentState && !shown.targetState && shown.isIdle && !bypassSheet && !overnightSheet) return
    val askBypass = {
        bypassSheet = true
        onDismiss()
    }
    val askOvernight = {
        overnightSheet = true
        onDismiss()
    }
    val stop = { kind: TimedKind -> if (kind == TimedKind.Bypass) onStopBypass() else onStopOvernight() }
    BackHandler(enabled = open, onBack = onDismiss)
    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(shown, enter = fadeIn(tween(120)), exit = fadeOut(tween(120))) {
            Box(
                Modifier
                    .fillMaxSize()
                    // The page under the menu steps back, as in the design.
                    .background(Color.Black.copy(alpha = if (c.dark) 0.62f else 0.3f))
                    .clickable(remember { MutableInteractionSource() }, null, onClick = onDismiss)
                    .testTag("quickScrim"),
            )
        }
        // Under the mode pill in the header that opened it, its right edge on the pill's.
        val density = androidx.compose.ui.platform.LocalDensity.current
        val anchor = dev.reins.android.ui.main.ModePillAnchor.bounds
        var rootWidth by remember { mutableIntStateOf(0) }
        Box(Modifier.fillMaxSize().onSizeChanged { rootWidth = it.width }) {
            AnimatedVisibility(
                shown,
                enter = fadeIn(tween(110)) + scaleIn(spring(dampingRatio = 0.8f, stiffness = 900f), initialScale = 0.9f, transformOrigin = TransformOrigin(1f, 0f)),
                exit = fadeOut(tween(100)) + scaleOut(tween(100), targetScale = 0.95f, transformOrigin = TransformOrigin(1f, 0f)),
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .then(
                        if (anchor != null && rootWidth > 0) {
                            with(density) { Modifier.padding(top = anchor.bottom.toDp() + 8.dp, end = (rootWidth - anchor.right).coerceAtLeast(0f).toDp(), start = 16.dp) }
                        } else {
                            Modifier.windowInsetsPadding(WindowInsets.statusBars).padding(top = 62.dp, end = 14.dp, start = 16.dp)
                        },
                    ),
            ) {
                val s = settings ?: return@AnimatedVisibility
                val modelReady = s.model.state == ModelState.INSTALLED
                when (style) {
                    QuickStyle.Slider -> SliderCard(s, modelReady, onSet, stop, onMore, onDismiss, askBypass, askOvernight)
                    else -> MenuCard(s, modelReady, onSet, stop, onMore, onDismiss, askBypass, askOvernight)
                }
            }
        }
        BypassSheets(bypassSheet, who = null, onStart = onBypass, onClose = { bypassSheet = false })
        OvernightSheets(
            overnightSheet,
            who = null,
            risky = settings?.overnightWhenRisky ?: OvernightRisky.WAIT,
            ratesRisk = settings?.model?.risk == true,
            onRisky = onWhenRisky,
            onStart = onOvernight,
            onClose = { overnightSheet = false },
        )
    }
}

// ---- the menu --------------------------------------------------------------------------------------------------------

/**
 * The quick menu, dropped from the header's mode pill: Manual and Auto side by side (the one in force outlined in its
 * colour), Overnight under them, then Bypass and Lock down, then the Autopilot page. A running bypass or Overnight is
 * its button itself ("Bypass on · 8:12", tap to stop), and a lockdown the Lock down button: there is no other Stop.
 */
@Composable
private fun MenuCard(
    s: AutopilotSettings,
    modelReady: Boolean,
    onSet: (AutopilotMode, UInt?) -> Unit,
    onStop: (TimedKind) -> Unit,
    onMore: () -> Unit,
    onDismiss: () -> Unit,
    onAskBypass: () -> Unit,
    onAskOvernight: () -> Unit,
) {
    val c = LocalColors.current
    val bypass = s.mode == AutopilotMode.BYPASS
    val overnight = s.mode == AutopilotMode.OVERNIGHT
    val lockdown = s.mode == AutopilotMode.LOCKDOWN
    val shape = RoundedCornerShape(22.dp)
    Column(
        Modifier
            .width(300.dp)
            .shadow(24.dp, shape, ambientColor = Color.Black.copy(alpha = 0.5f), spotColor = Color.Black.copy(alpha = 0.65f))
            .clip(shape)
            .background(if (c.dark) Color(0xFF18181B) else c.elevated)
            .border(1.dp, if (c.dark) Color(0xFF2A2A30) else c.hairline, shape)
            .padding(12.dp)
            .testTag("autopilotQuick"),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.height(IntrinsicSize.Min)) {
            listOf(AutopilotMode.MANUAL, AutopilotMode.AUTO).forEach { mode ->
                val needs = !modelReady && AutopilotText.needsModel(mode)
                QuickModeCard(
                    mode,
                    selected = s.mode == mode,
                    quiet = bypass || overnight || lockdown,
                    line = if (needs) "Needs the model" else AutopilotText.heroLine(mode),
                    modifier = Modifier.weight(1f).fillMaxHeight(),
                ) {
                    if (mode != s.mode) onSet(mode, null)
                    onDismiss()
                }
            }
        }
        if (overnight) {
            val until = s.overnightUntil
            QuickStateButton(
                title = "${AutopilotText.OVERNIGHT} on",
                until = until?.takeUnless { AutopilotText.isUntilOff(it) },
                hint = "Tap to stop",
                background = c.night,
                foreground = onTimed(TimedKind.Overnight, c),
                modifier = Modifier.padding(top = 8.dp).fillMaxWidth().testTag("quick:OVERNIGHT"),
                clockTag = "quickOvernightLeft",
            ) {
                onStop(TimedKind.Overnight)
                onDismiss()
            }
        } else {
            QuickButton(
                AutopilotText.OVERNIGHT,
                OvernightIcon,
                c.night.copy(alpha = 0.13f),
                c.night,
                Modifier.padding(top = 8.dp).fillMaxWidth().testTag("quick:OVERNIGHT"),
                onClick = onAskOvernight,
            )
        }
        Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (bypass) {
                val until = s.bypassUntil
                QuickStateButton(
                    title = "Bypass on",
                    until = until?.takeUnless { AutopilotText.isUntilOff(it) },
                    hint = "Tap to stop",
                    background = c.danger,
                    foreground = onTimed(TimedKind.Bypass, c),
                    modifier = Modifier.weight(1f).testTag("quick:BYPASS"),
                ) {
                    onStop(TimedKind.Bypass)
                    onDismiss()
                }
            } else {
                QuickButton("Bypass", Icons.Rounded.Bolt, c.danger.copy(alpha = 0.12f), c.danger, Modifier.weight(1f).testTag("quick:BYPASS"), onClick = onAskBypass)
            }
            if (lockdown) {
                QuickStateButton(
                    title = "Locked down",
                    until = null,
                    hint = "Tap to end",
                    background = c.warning,
                    foreground = OnWarning,
                    modifier = Modifier.weight(1f).testTag("quick:LOCKDOWN"),
                ) {
                    onSet(s.baseMode.takeIf { it != AutopilotMode.LOCKDOWN } ?: AutopilotMode.MANUAL, null)
                    onDismiss()
                }
            } else {
                QuickButton(
                    "Lock down",
                    Icons.Rounded.Lock,
                    if (c.dark) Color.White.copy(alpha = 0.06f) else c.controlFill,
                    c.text,
                    Modifier.weight(1f).testTag("quick:LOCKDOWN"),
                ) {
                    onSet(AutopilotMode.LOCKDOWN, null)
                    onDismiss()
                }
            }
        }
        Box(Modifier.padding(top = 10.dp).fillMaxWidth().height(1.dp).background(if (c.dark) Color(0xFF26262B) else c.hairline))
        Row(
            Modifier
                .padding(top = 2.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(10.dp))
                .pressable(shape = RoundedCornerShape(10.dp), onClick = onMore)
                .testTag("quickMore")
                .padding(start = 4.dp, end = 4.dp, top = 8.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText("Autopilot settings", RType.sans(13.5f, FontWeight.Medium), c.accent, Modifier.weight(1f), maxLines = 1)
            GlyphIcon(Glyph.ChevronRight, c.accent, size = 14.dp)
        }
    }
}

/** Manual or Auto in the menu: its tile, name and what it does; the one in force outlined and washed in its colour. */
@Composable
private fun QuickModeCard(mode: AutopilotMode, selected: Boolean, quiet: Boolean, line: String, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val tint = modeTint(mode, c)
    val shape = RoundedCornerShape(16.dp)
    Column(
        modifier
            // While a bypass or a lockdown is in force neither is: both stay tappable, quieter.
            .graphicsLayer { alpha = if (quiet) 0.45f else 1f }
            .clip(shape)
            .background(if (selected && !quiet) tint.copy(alpha = 0.10f) else if (c.dark) Color(0xFF0D0D0F) else c.background)
            .border(1.5.dp, if (selected && !quiet) tint else if (c.dark) Color(0xFF26262B) else c.hairline, shape)
            .pressable(shape = shape, onClick = onClick)
            .semantics(mergeDescendants = true) {
                this.selected = selected
                role = Role.RadioButton
            }
            .testTag("quick:${mode.name}")
            .padding(11.dp),
    ) {
        Box(Modifier.size(32.dp).clip(RoundedCornerShape(10.dp)).background(tint.copy(alpha = 0.17f)), contentAlignment = Alignment.Center) {
            androidx.compose.material3.Icon(modeIcon(mode), null, Modifier.size(18.dp), tint = tint)
        }
        RText(AutopilotText.name(mode), RType.sans(15f, FontWeight.SemiBold), c.text, Modifier.padding(top = 8.dp), maxLines = 1)
        RText(line, RType.sans(11.5f, lineHeight = 15f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 2)
    }
}

/** A button of the menu that is also a state: "Bypass on · 8:12" ([title] and the time left) over "Tap to stop". */
@Composable
private fun QuickStateButton(
    title: String,
    until: Long?,
    hint: String,
    background: Color,
    foreground: Color,
    modifier: Modifier,
    clockTag: String = "quickBypassLeft",
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(14.dp)
    Column(
        modifier
            .height(44.dp)
            .clip(shape)
            .background(background)
            .pressable(shape = shape, onClick = onClick)
            .semantics(mergeDescendants = true) {},
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        if (until != null) {
            val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
            RText("$title · ${AutopilotText.clock(until, now)}", RType.sans(14f, FontWeight.SemiBold), foreground, Modifier.testTag(clockTag), maxLines = 1)
        } else {
            RText(title, RType.sans(14f, FontWeight.SemiBold), foreground, maxLines = 1)
        }
        RText(hint, RType.sans(11f, FontWeight.Medium), foreground.copy(alpha = 0.75f), maxLines = 1)
    }
}

// ---- the slider ------------------------------------------------------------------------------------------------------

/**
 * Lockdown to Bypass on one track. The thumb's colour and glow follow the risk as it moves; each detent ticks, climbing
 * the scale. Letting go sets the mode, except Bypass, which waits for a length (or Cancel, which slides back).
 */
@Composable
private fun SliderCard(
    s: AutopilotSettings,
    modelReady: Boolean,
    onSet: (AutopilotMode, UInt?) -> Unit,
    onStop: (TimedKind) -> Unit,
    onMore: () -> Unit,
    onDismiss: () -> Unit,
    onAskBypass: () -> Unit,
    onAskOvernight: () -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val scope = rememberCoroutineScope()
    val modes = AutopilotText.byRisk
    val last = modes.size - 1
    val current = modes.indexOf(s.mode).coerceAtLeast(0)
    val pos = remember { Animatable(current.toFloat()) }
    var detent by remember { mutableIntStateOf(current) }
    LaunchedEffect(current) {
        detent = current
        pos.animateTo(current.toFloat(), spring(dampingRatio = 0.75f, stiffness = 500f))
    }
    val at = pos.value.coerceIn(0f, last.toFloat())
    val lo = at.toInt().coerceAtMost(last - 1)
    val tint = lerp(modeTint(modes[lo], c), modeTint(modes[lo + 1], c), at - lo)
    val risk = at / last
    val shown = modes[detent]

    fun land(index: Int) {
        detent = index
        scope.launch { pos.animateTo(index.toFloat(), spring(dampingRatio = 0.7f, stiffness = 600f)) }
        val mode = modes[index]
        when {
            // Turning a bypass or Overnight on is held to confirm on its sheet; the thumb goes back to the mode in
            // force meanwhile.
            AutopilotText.isTimed(mode) && s.mode != mode -> {
                detent = current
                scope.launch { pos.animateTo(current.toFloat(), spring(dampingRatio = 0.75f, stiffness = 500f)) }
                if (mode == AutopilotMode.BYPASS) onAskBypass() else onAskOvernight()
            }
            mode != s.mode -> {
                onSet(mode, null)
                scope.launch {
                    delay(320)
                    onDismiss()
                }
            }
            else -> Unit
        }
    }

    Column(
        Modifier
            .fillMaxWidth()
            .glass(c, RoundedCornerShape(26.dp), 16.dp)
            .background(Brush.verticalGradient(listOf(tint.copy(alpha = if (c.dark) 0.16f + 0.10f * risk else 0.10f + 0.06f * risk), Color.Transparent)))
            .animateContentSize(spring(dampingRatio = 0.9f, stiffness = 700f))
            .padding(horizontal = 18.dp, vertical = 16.dp)
            .testTag("autopilotQuick"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconTile(modeIcon(shown), tint, size = 40.dp, filled = true)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText(AutopilotText.name(shown), RType.sans(18f, FontWeight.SemiBold), c.text, Modifier.testTag("quickMode"), maxLines = 1)
                val needs = !modelReady && AutopilotText.needsModel(shown)
                RText(if (needs) "Needs the model" else AutopilotText.line(shown), RType.sans(13f), c.secondary, maxLines = 1)
            }
            running(s)?.let { TimeLeft(it.kind, it.until) }
        }
        Spacer(Modifier.height(18.dp))
        RiskTrack(at, tint, risk, modes, detent, onMove = { index ->
            if (index != detent) {
                detent = index
                feedback.play(Event.Detent, index)
            }
        }, onDrag = { value -> scope.launch { pos.snapTo(value) } }, onRelease = ::land)
        if (s.mode == AutopilotMode.BYPASS) {
            QuickButton("Stop bypass", null, c.danger.copy(alpha = 0.12f), c.danger, Modifier.padding(top = 14.dp).fillMaxWidth().testTag("quickStopBypass")) {
                onStop(TimedKind.Bypass)
                onDismiss()
            }
        } else if (s.mode == AutopilotMode.OVERNIGHT) {
            QuickButton("Stop ${AutopilotText.OVERNIGHT}", null, c.night.copy(alpha = 0.13f), c.night, Modifier.padding(top = 14.dp).fillMaxWidth().testTag("quickStopOvernight")) {
                onStop(TimedKind.Overnight)
                onDismiss()
            }
        }
        Row(Modifier.padding(top = 10.dp).fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            Row(
                Modifier
                    .clip(CircleShape)
                    .pressable(shape = CircleShape, onClick = onMore)
                    .testTag("quickMore")
                    .padding(horizontal = 10.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RText("More", RType.sans(14f, FontWeight.Medium), c.secondary, maxLines = 1)
                Spacer(Modifier.width(4.dp))
                GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 12.dp)
            }
        }
    }
}

/** The track: the five colours in a row, a dot per detent, the glowing thumb, and each mode's icon underneath. */
@Composable
private fun RiskTrack(
    at: Float,
    tint: Color,
    risk: Float,
    modes: List<AutopilotMode>,
    detent: Int,
    onMove: (Int) -> Unit,
    onDrag: (Float) -> Unit,
    onRelease: (Int) -> Unit,
) {
    val c = LocalColors.current
    val last = modes.size - 1
    // The gesture detectors outlive a recomposition; they call whatever the latest callbacks are.
    val move by rememberUpdatedState(onMove)
    val drag by rememberUpdatedState(onDrag)
    val release by rememberUpdatedState(onRelease)
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val thumb = 30.dp
        val span = maxWidth - thumb
        val step = span / last
        val d = androidx.compose.ui.platform.LocalDensity.current
        val thumbPx = with(d) { thumb.toPx() }
        val stepPx = with(d) { step.toPx() }
        val toIndex = { x: Float -> ((x - thumbPx / 2) / stepPx).coerceIn(0f, last.toFloat()) }
        Column {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(44.dp)
                    .pointerInput(stepPx) {
                        detectTapGestures { o ->
                            val i = toIndex(o.x).roundToInt()
                            move(i)
                            release(i)
                        }
                    }
                    .pointerInput(stepPx) {
                        var value = 0f
                        detectHorizontalDragGestures(
                            onDragStart = { o -> value = toIndex(o.x) },
                            onDragEnd = { release(value.roundToInt()) },
                            onDragCancel = { release(value.roundToInt()) },
                        ) { change, _ ->
                            change.consume()
                            value = toIndex(change.position.x)
                            drag(value)
                            move(value.roundToInt())
                        }
                    }
                    .semantics { contentDescription = "Autopilot: ${AutopilotText.name(modes[detent])}" }
                    .testTag("riskSlider"),
                contentAlignment = Alignment.CenterStart,
            ) {
                val colors = modes.map { modeTint(it, c) }
                Canvas(Modifier.padding(horizontal = thumb / 2).fillMaxWidth().height(8.dp)) {
                    drawRoundRect(
                        Brush.horizontalGradient(colors.map { it.copy(alpha = if (c.dark) 0.55f else 0.45f) }),
                        cornerRadius = androidx.compose.ui.geometry.CornerRadius(size.height / 2),
                    )
                    val gap = size.width / last
                    for (i in 0..last) drawCircle(Color.White.copy(alpha = if (c.dark) 0.55f else 0.85f), 2.dp.toPx(), Offset(gap * i, size.height / 2))
                }
                // The glow grows with the risk; it is drawn outside the thumb, behind it.
                Box(
                    Modifier
                        .offset { IntOffset(((step * at).toPx() + (thumb / 2).toPx() - (thumb * 1.5f).toPx()).roundToInt(), 0) }
                        .size(thumb * 3f)
                        .background(Brush.radialGradient(listOf(tint.copy(alpha = 0.18f + 0.32f * risk), Color.Transparent))),
                )
                Box(
                    Modifier
                        .offset { IntOffset((step * at).toPx().roundToInt(), 0) }
                        .size(thumb)
                        .shadow(6.dp + 10.dp * risk, CircleShape, ambientColor = tint, spotColor = tint)
                        .clip(CircleShape)
                        .background(tint)
                        .border(2.dp, Color.White.copy(alpha = 0.9f), CircleShape)
                        .testTag("riskThumb"),
                )
            }
            Row(Modifier.fillMaxWidth().padding(top = 4.dp)) {
                modes.forEachIndexed { i, mode ->
                    val on = i == detent
                    val fg by animateColorAsState(if (on) modeTint(mode, c) else c.tertiary, label = "detentFg")
                    Box(
                        Modifier
                            .weight(1f)
                            .clip(RoundedCornerShape(10.dp))
                            .pressable(shape = RoundedCornerShape(10.dp)) {
                                onMove(i)
                                onRelease(i)
                            }
                            .semantics { contentDescription = AutopilotText.name(mode) }
                            .testTag("detent:${mode.name}")
                            .padding(vertical = 6.dp),
                        contentAlignment = when (i) {
                            0 -> Alignment.CenterStart
                            last -> Alignment.CenterEnd
                            else -> Alignment.Center
                        },
                    ) {
                        Box(Modifier.width(thumb), contentAlignment = Alignment.Center) { androidx.compose.material3.Icon(modeIcon(mode), null, Modifier.size(18.dp), tint = fg) }
                    }
                }
            }
        }
    }
}

// ---- shared parts ----------------------------------------------------------------------------------------------------

/** A button of the menu: an optional icon and its words, on its own fill. */
@Composable
private fun QuickButton(title: String, icon: ImageVector?, background: Color, foreground: Color, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val shape = RoundedCornerShape(14.dp)
    Row(
        modifier
            .height(44.dp)
            .clip(shape)
            .background(background)
            .pressable(shape = shape, onClick = onClick),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            androidx.compose.material3.Icon(icon, null, Modifier.size(16.dp), tint = foreground)
            Spacer(Modifier.width(7.dp))
        }
        RText(title, RType.sans(14f, FontWeight.SemiBold), foreground, maxLines = 1)
    }
}

/** A running bypass's or Overnight's time left, ticking, in its colour. */
@Composable
private fun TimeLeft(kind: TimedKind, until: Long) {
    val c = LocalColors.current
    val tint = kind.tint(c)
    val tag = if (kind == TimedKind.Bypass) "quickBypassLeft" else "quickOvernightLeft"
    if (AutopilotText.isUntilOff(until)) {
        androidx.compose.material3.Icon(androidx.compose.material.icons.Icons.Rounded.AllInclusive, "No end", Modifier.size(18.dp).testTag(tag), tint = tint)
        return
    }
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    RText(AutopilotText.clock(until, now), RType.mono(13f, FontWeight.SemiBold), tint, Modifier.testTag(tag), maxLines = 1)
}
