package dev.reins.android.ui.autopilot

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalLiveTimers
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowMillis
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.NeighbourView
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict

/** Each mode's colour: neutral for Manual, blue for Assisted, the accent for Auto, red for Bypass, amber for Lockdown. */
fun modeTint(mode: AutopilotMode, c: RColors): Color = when (mode) {
    AutopilotMode.MANUAL -> c.secondary
    AutopilotMode.ASSISTED -> c.search
    AutopilotMode.AUTO -> c.accent
    AutopilotMode.BYPASS -> c.danger
    AutopilotMode.LOCKDOWN -> c.warning
}

fun modeGlyph(mode: AutopilotMode): Glyph = when (mode) {
    AutopilotMode.MANUAL -> Glyph.Hand
    AutopilotMode.ASSISTED -> Glyph.Sparkle
    AutopilotMode.AUTO -> Glyph.Navigate
    AutopilotMode.BYPASS -> Glyph.Bolt
    AutopilotMode.LOCKDOWN -> Glyph.Lock
}

fun verdictTint(v: Verdict, c: RColors): Color = when (v) {
    Verdict.APPROVE -> c.success
    Verdict.DENY -> c.danger
    Verdict.ASK -> c.search
}

fun verdictGlyph(v: Verdict): Glyph = when (v) {
    Verdict.APPROVE -> Glyph.Check
    Verdict.DENY -> Glyph.Close
    Verdict.ASK -> Glyph.Hand
}

/** A rounded square holding a glyph on a wash of its colour; [filled] turns it solid. */
@Composable
fun IconTile(glyph: Glyph, tint: Color, modifier: Modifier = Modifier, size: Dp = 40.dp, filled: Boolean = false) {
    val bg by animateColorAsState(if (filled) tint else tint.copy(alpha = 0.13f), label = "tileBg")
    val fg by animateColorAsState(if (filled) Color.White else tint, label = "tileFg")
    Box(modifier.size(size).clip(RoundedCornerShape(size * 0.3f)).background(bg), contentAlignment = Alignment.Center) {
        GlyphIcon(glyph, fg, size = size * 0.5f, weight = 1.8f)
    }
}

/**
 * A ring from twelve o'clock over a faint track. It fills from empty when it first appears, then springs to new
 * values; [content] sits in the middle.
 */
@Composable
fun ProgressRing(
    fraction: Float,
    color: Color,
    modifier: Modifier = Modifier,
    size: Dp = 52.dp,
    stroke: Dp = 5.dp,
    content: @Composable () -> Unit = {},
) {
    val c = LocalColors.current
    val shown = remember { Animatable(0f) }
    LaunchedEffect(fraction) { shown.animateTo(fraction.coerceIn(0f, 1f), spring(dampingRatio = 0.85f, stiffness = 120f)) }
    Box(modifier.size(size), contentAlignment = Alignment.Center) {
        Canvas(Modifier.fillMaxSize()) {
            val w = stroke.toPx()
            val arc = Size(this.size.width - w, this.size.height - w)
            val at = Offset(w / 2, w / 2)
            drawArc(c.controlFill.copy(alpha = if (c.dark) 0.12f else 0.09f), 0f, 360f, false, at, arc, style = Stroke(w))
            if (shown.value > 0.001f) drawArc(color, -90f, 360f * shown.value, false, at, arc, style = Stroke(w, cap = StrokeCap.Round))
        }
        content()
    }
}

/** A thin bar that fills to [fraction] in [color], over the same faint track. */
@Composable
fun MeterBar(fraction: Float, color: Color, modifier: Modifier = Modifier, height: Dp = 6.dp) {
    val c = LocalColors.current
    val shown = remember { Animatable(0f) }
    LaunchedEffect(fraction) { shown.animateTo(fraction.coerceIn(0f, 1f), spring(dampingRatio = 0.9f, stiffness = 160f)) }
    Box(modifier.fillMaxWidth().height(height).clip(CircleShape).background(c.controlFill.copy(alpha = if (c.dark) 0.12f else 0.09f))) {
        Box(Modifier.fillMaxHeight().fillMaxWidth(shown.value).clip(CircleShape).background(color))
    }
}

/** "Approve  ███████░ 97%": one labelled probability. */
@Composable
fun ProbabilityRow(label: String, p: Float, color: Color, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        RText(label, RType.sans(13.5f, FontWeight.Medium), c.secondary, Modifier.width(92.dp), maxLines = 1)
        MeterBar(p, color, Modifier.weight(1f))
        RText(AutopilotText.percent(p), RType.mono(13f, FontWeight.Medium), c.text, Modifier.padding(start = 10.dp).width(40.dp), maxLines = 1)
    }
}

/** A remembered decision like the one at hand: what the user did, what it was, how alike. */
@Composable
fun NeighbourRow(n: NeighbourView, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val tint = verdictTint(n.verdict, c)
    Row(modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(22.dp).clip(CircleShape).background(tint.copy(alpha = 0.14f)), contentAlignment = Alignment.Center) {
            GlyphIcon(verdictGlyph(n.verdict), tint, size = 13.dp, weight = 2.2f)
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            RText(dev.reins.android.ui.common.untrusted(n.label), RType.sans(14f, FontWeight.Medium), c.text, maxLines = 2)
            RText(
                "${AutopilotText.pastVerdict(n.verdict)} · ${dev.reins.android.ui.common.relativeTime(n.at)}",
                RType.sans(12f),
                c.tertiary,
                maxLines = 1,
            )
        }
        Spacer(Modifier.width(8.dp))
        RText("${AutopilotText.percent(n.similarity)} alike", RType.mono(12f, FontWeight.Medium), c.secondary, maxLines = 1)
    }
}

/** A dot that breathes, for something running now. Still when clocks are frozen (screenshots). */
@Composable
fun PulseDot(color: Color, size: Dp = 8.dp) {
    val live = LocalLiveTimers.current
    // Read only in the layer: the breathing never recomposes the dot.
    val alpha = if (live) {
        val t = rememberInfiniteTransition(label = "pulse")
        t.animateFloat(1f, 0.35f, infiniteRepeatable(tween(900, easing = LinearEasing), RepeatMode.Reverse), label = "pulseAlpha")
    } else {
        null
    }
    Box(Modifier.size(size).graphicsLayer { this.alpha = alpha?.value ?: 1f }.clip(CircleShape).background(color))
}

/**
 * The big mode selector: five rows of equal height and one highlight that springs from the old row to the new one,
 * taking on the new mode's colour on the way. Each row says what the mode does; Assisted and Auto say when they need
 * the model.
 */
@Composable
fun ModePicker(
    selected: AutopilotMode,
    modelReady: Boolean,
    modifier: Modifier = Modifier,
    tagPrefix: String = "mode",
    onPick: (AutopilotMode) -> Unit,
) {
    val c = LocalColors.current
    val modes = AutopilotText.modes
    val index = modes.indexOf(selected).coerceAtLeast(0)
    val y by animateFloatAsState(index.toFloat(), spring(dampingRatio = 0.78f, stiffness = 420f), label = "modeY")
    val tint by animateColorAsState(modeTint(selected, c), spring(stiffness = 300f), label = "modeTint")
    Box(modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp)).background(c.elevated).padding(6.dp)) {
        // The highlight behind the selected row.
        Box(
            Modifier
                .offset(y = ROW_HEIGHT * y)
                .fillMaxWidth()
                .height(ROW_HEIGHT)
                .clip(RoundedCornerShape(17.dp))
                .background(tint.copy(alpha = if (c.dark) 0.14f else 0.09f))
                .border(1.dp, tint.copy(alpha = 0.35f), RoundedCornerShape(17.dp)),
        )
        Column {
            modes.forEach { mode ->
                ModeRow(mode, mode == selected, !modelReady && AutopilotText.needsModel(mode), "$tagPrefix:${mode.name}") { onPick(mode) }
            }
        }
    }
}

private val ROW_HEIGHT = 70.dp

@Composable
private fun ModeRow(mode: AutopilotMode, selected: Boolean, needsModel: Boolean, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    val tint = modeTint(mode, c)
    Row(
        Modifier
            .fillMaxWidth()
            .height(ROW_HEIGHT)
            .clip(RoundedCornerShape(17.dp))
            .pressable(shape = RoundedCornerShape(17.dp), onClick = onClick)
            .semantics(mergeDescendants = true) {
                this.selected = selected
            }
            .testTag(tag)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconTile(modeGlyph(mode), tint, size = 42.dp, filled = selected)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                RText(AutopilotText.name(mode), RType.sans(16.5f, FontWeight.SemiBold), c.text, maxLines = 1)
                if (needsModel) {
                    Spacer(Modifier.width(8.dp))
                    RText(
                        "NEEDS MODEL",
                        RType.sans(10f, FontWeight.SemiBold).copy(letterSpacing = androidx.compose.ui.unit.TextUnit(0.6f, androidx.compose.ui.unit.TextUnitType.Sp)),
                        c.tertiary,
                        Modifier.clip(CircleShape).background(c.controlFill).padding(horizontal = 7.dp, vertical = 3.dp),
                        maxLines = 1,
                    )
                }
            }
            if (selected) RText(AutopilotText.line(mode), RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 2)
        }
        Spacer(Modifier.width(10.dp))
        RadioDot(selected, tint)
    }
}

/** A round selection mark that fills with a spring. */
@Composable
fun RadioDot(selected: Boolean, tint: Color) {
    val c = LocalColors.current
    val fill by animateFloatAsState(if (selected) 1f else 0f, spring(dampingRatio = 0.55f, stiffness = 600f), label = "radio")
    Box(
        Modifier.size(22.dp).clip(CircleShape).border(1.75.dp, if (selected) tint else c.tertiary.copy(alpha = 0.6f), CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(12.dp).graphicsLayer { scaleX = fill; scaleY = fill }.clip(CircleShape).background(tint))
    }
}

/**
 * A row of capsules with one sliding indicator (Zeron's segmented control in Reins's colours). The choice is
 * felt as a selection.
 */
@Composable
fun <T> Segmented(
    options: List<T>,
    selected: T,
    label: (T) -> String,
    modifier: Modifier = Modifier,
    tag: (T) -> String = { label(it) },
    onSelect: (T) -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val index = options.indexOf(selected).coerceAtLeast(0)
    val x by animateFloatAsState(index.toFloat(), spring(dampingRatio = 0.8f, stiffness = 500f), label = "segment")
    BoxWithConstraints(modifier.fillMaxWidth().height(44.dp).clip(CircleShape).background(c.controlFill).padding(4.dp)) {
        val w = maxWidth / options.size
        Box(
            Modifier
                .offset(x = w * x)
                .width(w)
                .fillMaxHeight()
                .clip(CircleShape)
                .background(if (c.dark) Color.White.copy(alpha = 0.12f) else Color.White)
                .border(0.75.dp, c.glassEdge, CircleShape),
        )
        Row(Modifier.fillMaxSize()) {
            options.forEach { option ->
                val on = option == selected
                val fg by animateColorAsState(if (on) c.text else c.secondary, label = "segFg")
                Box(
                    Modifier
                        .weight(1f)
                        .fillMaxHeight()
                        .clip(CircleShape)
                        .pressable(shape = CircleShape, dim = 0.7f) {
                            if (!on) {
                                onSelect(option)
                                feedback.play(Event.Selection)
                            }
                        }
                        .semantics {
                            this.selected = on
                            role = Role.Tab
                        }
                        .testTag(tag(option)),
                    contentAlignment = Alignment.Center,
                ) { RText(label(option), RType.sans(14.5f, if (on) FontWeight.SemiBold else FontWeight.Medium), fg, maxLines = 1) }
            }
        }
    }
}

/**
 * The approval sheet's line from Autopilot: what it would do and how sure it is; tapping opens why (the remembered
 * decisions it is like, and anything that keeps it from deciding).
 */
@Composable
fun SuggestionStrip(s: SuggestionView, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var open by remember { mutableStateOf(false) }
    val tint = when {
        s.floor -> c.secondary
        !s.judged -> c.tertiary
        else -> verdictTint(s.verdict, c)
    }
    val turn by animateFloatAsState(if (open) 90f else 0f, spring(stiffness = 500f), label = "chevron")
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(18.dp))
            .background(tint.copy(alpha = if (c.dark) 0.10f else 0.07f))
            .border(0.75.dp, tint.copy(alpha = 0.25f), RoundedCornerShape(18.dp))
            .animateContentSize(spring(dampingRatio = 0.9f, stiffness = 500f))
            .testTag("suggestion"),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .pressable(shape = RoundedCornerShape(18.dp), dim = 0.75f) {
                    open = !open
                    feedback.play(Event.expand(open))
                }
                .padding(horizontal = 14.dp, vertical = 12.dp)
                .testTag("suggestionToggle"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (s.judged && !s.floor) {
                ProgressRing(if (s.verdict == Verdict.DENY) s.pDeny else s.pApprove, tint, size = 34.dp, stroke = 3.5.dp) {
                    GlyphIcon(Glyph.Sparkle, tint, size = 15.dp, weight = 1.9f)
                }
            } else {
                IconTile(if (s.floor) Glyph.Shield else Glyph.Sparkle, tint, size = 34.dp)
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText(AutopilotText.suggestionHeadline(s), RType.sans(15f, FontWeight.SemiBold), c.text, Modifier.testTag("suggestionHeadline"), maxLines = 2)
                if (!open && s.judged && s.reason.isNotBlank()) {
                    RText(dev.reins.android.ui.common.untrusted(s.reason), RType.sans(12.5f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 1)
                }
            }
            Spacer(Modifier.width(8.dp))
            GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp, modifier = Modifier.rotate(turn))
        }
        if (open) {
            Column(Modifier.padding(start = 14.dp, end = 14.dp, bottom = 14.dp).testTag("suggestionDetail")) {
                if (s.judged) {
                    ProbabilityRow("Approve", s.pApprove, c.success)
                    Spacer(Modifier.height(8.dp))
                    ProbabilityRow("Deny", s.pDeny, c.danger)
                    Spacer(Modifier.height(8.dp))
                    ProbabilityRow("Confidence", s.confidence, c.accent)
                    if (s.reason.isNotBlank()) {
                        RText(dev.reins.android.ui.common.untrusted(s.reason), RType.sans(14f, lineHeight = 19f), c.text, Modifier.padding(top = 12.dp))
                    }
                }
                if (s.neighbours.isNotEmpty()) {
                    RText("LIKE THESE DECISIONS OF YOURS", RType.sans(11.5f, FontWeight.SemiBold), c.tertiary, Modifier.padding(top = 12.dp, bottom = 2.dp))
                    s.neighbours.take(4).forEachIndexed { i, n -> NeighbourRow(n, Modifier.testTag("neighbour:$i")) }
                }
                AutopilotText.suggestionNotes(s).forEach { note ->
                    Row(Modifier.padding(top = 10.dp), verticalAlignment = Alignment.Top) {
                        GlyphIcon(Glyph.Info, c.secondary, size = 16.dp)
                        Spacer(Modifier.width(8.dp))
                        RText(note, RType.sans(13.5f, lineHeight = 18f), c.secondary)
                    }
                }
                RText(
                    "Profile ${dev.reins.android.ui.common.untrusted(s.profileName)} · decided on this phone",
                    RType.sans(12f),
                    c.tertiary,
                    Modifier.padding(top = 12.dp),
                )
            }
        }
    }
}

/** The small mark on activity entries Autopilot, a bypass or Lockdown decided. */
@Composable
fun AutopilotBadge(decidedBy: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val (label, glyph, tint) = when (decidedBy) {
        "bypass" -> Triple("Bypass", Glyph.Bolt, c.danger)
        "lockdown" -> Triple("Lockdown", Glyph.Lock, c.warning)
        else -> Triple("Autopilot", Glyph.Sparkle, c.accent)
    }
    Row(
        modifier.height(22.dp).clip(CircleShape).background(tint.copy(alpha = 0.12f)).padding(start = 6.dp, end = 8.dp).testTag("autoBadge"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(glyph, tint, size = 12.dp, weight = 2f)
        Spacer(Modifier.width(4.dp))
        RText(label, RType.sans(11.5f, FontWeight.SemiBold), tint, maxLines = 1)
    }
}

/** A small section caption inside a card. */
@Composable
fun Caption(text: String, modifier: Modifier = Modifier) {
    RText(
        text.uppercase(),
        RType.sans(11.5f, FontWeight.SemiBold).copy(letterSpacing = androidx.compose.ui.unit.TextUnit(0.6f, androidx.compose.ui.unit.TextUnitType.Sp)),
        LocalColors.current.tertiary,
        modifier,
    )
}

/** Spacing between stacked rows of a card. */
val CardGap = Arrangement.spacedBy(8.dp)
