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
 * "More" opens the full page. Lockdown is one tap (it only takes power away); Bypass first asks for how long.
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
) {
    val c = LocalColors.current
    val shown = remember { MutableTransitionState(false) }
    shown.targetState = open && settings != null
    if (!shown.currentState && !shown.targetState && shown.isIdle) return
    BackHandler(enabled = open, onBack = onDismiss)
    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(shown, enter = fadeIn(tween(120)), exit = fadeOut(tween(120))) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(c.scrim.copy(alpha = c.scrim.alpha * 0.6f))
                    .clickable(remember { MutableInteractionSource() }, null, onClick = onDismiss)
                    .testTag("quickScrim"),
            )
        }
        AnimatedVisibility(
            shown,
            enter = fadeIn(tween(110)) + scaleIn(spring(dampingRatio = 0.8f, stiffness = 900f), initialScale = 0.86f, transformOrigin = TransformOrigin(1f, 1f)),
            exit = fadeOut(tween(100)) + scaleOut(tween(100), targetScale = 0.94f, transformOrigin = TransformOrigin(1f, 1f)),
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .windowInsetsPadding(WindowInsets.navigationBars)
                // Just above the round button: the bar is 60dp tall with 10dp around it.
                .padding(start = 16.dp, end = 16.dp, bottom = 86.dp),
        ) {
            val s = settings ?: return@AnimatedVisibility
            val modelReady = s.model.state == ModelState.INSTALLED
            when (style) {
                QuickStyle.Slider -> SliderCard(s, modelReady, onSet, onStopBypass, onMore, onDismiss)
                else -> MenuCard(s, modelReady, onSet, onStopBypass, onMore, onDismiss)
            }
        }
    }
}

// ---- the menu --------------------------------------------------------------------------------------------------------

@Composable
private fun MenuCard(
    s: AutopilotSettings,
    modelReady: Boolean,
    onSet: (AutopilotMode, UInt?) -> Unit,
    onStopBypass: () -> Unit,
    onMore: () -> Unit,
    onDismiss: () -> Unit,
) {
    val c = LocalColors.current
    var askBypass by remember { mutableStateOf(false) }
    Column(
        Modifier
            .width(276.dp)
            .glass(c, RoundedCornerShape(22.dp), 16.dp)
            .animateContentSize(spring(dampingRatio = 0.9f, stiffness = 700f))
            .padding(6.dp)
            .testTag("autopilotQuick"),
    ) {
        // Riskiest at the top, nearest the thumb the safest: the list reads like the slider stood on end.
        AutopilotText.byRisk.reversed().forEach { mode ->
            val selected = mode == s.mode
            val tint = modeTint(mode, c)
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(16.dp))
                    .background(if (selected) tint.copy(alpha = if (c.dark) 0.14f else 0.09f) else Color.Transparent)
                    .pressable(highlight = c.controlFill, shape = RoundedCornerShape(16.dp)) {
                        when {
                            mode == AutopilotMode.BYPASS && s.mode != AutopilotMode.BYPASS -> askBypass = !askBypass
                            mode != s.mode -> {
                                onSet(mode, null)
                                onDismiss()
                            }
                            else -> onDismiss()
                        }
                    }
                    .semantics(mergeDescendants = true) { this.selected = selected }
                    .testTag("quick:${mode.name}")
                    .padding(horizontal = 10.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconTile(modeGlyph(mode), tint, size = 34.dp, filled = selected)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    RText(AutopilotText.name(mode), RType.sans(15.5f, FontWeight.SemiBold), c.text, maxLines = 1)
                    val needs = !modelReady && AutopilotText.needsModel(mode)
                    RText(if (needs) "Needs the model" else AutopilotText.line(mode), RType.sans(12.5f), if (needs) c.tertiary else c.secondary, maxLines = 1)
                }
                if (selected && mode == AutopilotMode.BYPASS && s.bypassUntil != null) {
                    BypassLeft(s.bypassUntil!!)
                } else if (selected) {
                    GlyphIcon(Glyph.Check, tint, size = 17.dp, weight = 2.2f)
                }
            }
            if (mode == AutopilotMode.BYPASS && s.mode == AutopilotMode.BYPASS) {
                QuickButton("Stop bypass", Glyph.Stop, c.danger, Modifier.padding(start = 56.dp, end = 10.dp, bottom = 6.dp).testTag("quickStopBypass")) {
                    onStopBypass()
                    onDismiss()
                }
            }
            if (mode == AutopilotMode.BYPASS && askBypass) {
                BypassTimes(Modifier.padding(start = 56.dp, end = 6.dp, bottom = 6.dp)) { minutes ->
                    onSet(AutopilotMode.BYPASS, minutes)
                    onDismiss()
                }
            }
        }
        Box(Modifier.padding(horizontal = 10.dp, vertical = 4.dp).fillMaxWidth().height(0.75.dp).background(c.hairline))
        Row(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(16.dp))
                .pressable(highlight = c.controlFill, shape = RoundedCornerShape(16.dp), onClick = onMore)
                .testTag("quickMore")
                .padding(horizontal = 14.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText("More", RType.sans(15f, FontWeight.Medium), c.text, Modifier.weight(1f), maxLines = 1)
            GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
        }
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
    onStopBypass: () -> Unit,
    onMore: () -> Unit,
    onDismiss: () -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val scope = rememberCoroutineScope()
    val modes = AutopilotText.byRisk
    val last = modes.size - 1
    val current = modes.indexOf(s.mode).coerceAtLeast(0)
    val pos = remember { Animatable(current.toFloat()) }
    var detent by remember { mutableIntStateOf(current) }
    var askBypass by remember { mutableStateOf(false) }
    LaunchedEffect(current) {
        if (!askBypass) {
            detent = current
            pos.animateTo(current.toFloat(), spring(dampingRatio = 0.75f, stiffness = 500f))
        }
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
            mode == AutopilotMode.BYPASS && s.mode != AutopilotMode.BYPASS -> askBypass = true
            mode != s.mode -> {
                askBypass = false
                onSet(mode, null)
                scope.launch {
                    delay(320)
                    onDismiss()
                }
            }
            else -> askBypass = false
        }
    }

    fun cancelBypass() {
        askBypass = false
        detent = current
        feedback.play(Event.Detent, current)
        scope.launch { pos.animateTo(current.toFloat(), spring(dampingRatio = 0.75f, stiffness = 500f)) }
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
            IconTile(modeGlyph(shown), tint, size = 40.dp, filled = true)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText(AutopilotText.name(shown), RType.sans(18f, FontWeight.SemiBold), c.text, Modifier.testTag("quickMode"), maxLines = 1)
                val needs = !modelReady && AutopilotText.needsModel(shown)
                RText(if (needs) "Needs the model" else AutopilotText.line(shown), RType.sans(13f), c.secondary, maxLines = 1)
            }
            if (s.mode == AutopilotMode.BYPASS && s.bypassUntil != null) BypassLeft(s.bypassUntil!!)
        }
        Spacer(Modifier.height(18.dp))
        RiskTrack(at, tint, risk, modes, detent, onMove = { index ->
            if (index != detent) {
                detent = index
                feedback.play(Event.Detent, index)
            }
        }, onDrag = { value -> scope.launch { pos.snapTo(value) } }, onRelease = ::land)
        if (askBypass) {
            BypassTimes(Modifier.padding(top = 14.dp), onCancel = ::cancelBypass) { minutes ->
                askBypass = false
                onSet(AutopilotMode.BYPASS, minutes)
                scope.launch {
                    delay(320)
                    onDismiss()
                }
            }
        } else if (s.mode == AutopilotMode.BYPASS) {
            QuickButton("Stop bypass", Glyph.Stop, c.danger, Modifier.padding(top = 14.dp).testTag("quickStopBypass")) {
                onStopBypass()
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
                        Box(Modifier.width(thumb), contentAlignment = Alignment.Center) { GlyphIcon(modeGlyph(mode), fg, size = 18.dp, weight = 2f) }
                    }
                }
            }
        }
    }
}

// ---- shared parts ----------------------------------------------------------------------------------------------------

/** How long a bypass runs: one tap on a length turns it on. */
@Composable
private fun BypassTimes(modifier: Modifier = Modifier, onCancel: (() -> Unit)? = null, onPick: (UInt) -> Unit) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().testTag("quickBypassTimes"), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        AutopilotText.bypassMinutes.forEach { m ->
            Box(
                Modifier
                    .weight(1f)
                    .height(36.dp)
                    .clip(CircleShape)
                    .background(c.danger.copy(alpha = 0.14f))
                    .border(0.75.dp, c.danger.copy(alpha = 0.4f), CircleShape)
                    .pressable(shape = CircleShape) { onPick(m) }
                    .testTag("quickBypass:$m"),
                contentAlignment = Alignment.Center,
            ) { RText(if (m == 60u) "1 h" else "$m min", RType.sans(13.5f, FontWeight.SemiBold), c.danger, maxLines = 1) }
        }
        if (onCancel != null) {
            Box(
                Modifier
                    .size(36.dp)
                    .clip(CircleShape)
                    .background(c.controlFill)
                    .pressable(shape = CircleShape, label = "Cancel", onClick = onCancel)
                    .semantics { contentDescription = "Cancel" }
                    .testTag("quickBypassCancel"),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.Close, c.secondary, size = 14.dp, weight = 2f) }
        }
    }
}

@Composable
private fun QuickButton(title: String, glyph: Glyph, tint: Color, modifier: Modifier = Modifier, onClick: () -> Unit) {
    Row(
        modifier
            .fillMaxWidth()
            .height(36.dp)
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.12f))
            .pressable(shape = CircleShape, onClick = onClick),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(glyph, tint, size = 14.dp)
        Spacer(Modifier.width(6.dp))
        RText(title, RType.sans(13.5f, FontWeight.SemiBold), tint, maxLines = 1)
    }
}

/** A running bypass's time left, ticking. */
@Composable
private fun BypassLeft(until: Long) {
    val c = LocalColors.current
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    RText(AutopilotText.clock(until, now), RType.mono(13f, FontWeight.SemiBold), c.danger, Modifier.testTag("quickBypassLeft"), maxLines = 1)
}
