package dev.reins.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableFloatState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Hold to confirm, for what cannot be taken back (Bypass, Disconnect, Delete account): the button has to be held for
 * [durationMillis] (1.2 s). While held a fill sweeps across it and a small ring fills beside "Keep holding"; haptic
 * ticks close in as it fills; at the end a heavy thud and the success sound, then [onConfirm]. Letting go early
 * cancels and the fill drains in 180 ms, with no haptic beyond the soft release cue.
 *
 * The fill runs on elapsed frame time, not an animation, so Reduce Motion ("Remove animations") never shortens it.
 * On paper it is the danger wash ([tint] the danger colour; another [tint] gets the same treatment in its colour).
 * [done] shows the finished state: solid, with a tick and [doneTitle].
 *
 * A screen reader's activation (and a test's click) confirms at once: the deliberate double tap is its hold.
 */
@Composable
fun HoldButton(
    title: String,
    tint: Color,
    modifier: Modifier = Modifier,
    durationMillis: Int = HOLD_MILLIS,
    icon: ImageVector? = Icons.Rounded.Fingerprint,
    enabled: Boolean = true,
    busy: Boolean = false,
    height: Dp = 54.dp,
    done: Boolean = false,
    doneTitle: String = title,
    /** Draws a hold in progress at this fill, untouchable (the gallery's "Keep holding"). */
    previewProgress: Float? = null,
    onConfirm: () -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val scope = rememberCoroutineScope()
    val progress = remember { mutableFloatStateOf(0f) }
    var holding by remember { mutableStateOf(false) }
    val confirm by rememberUpdatedState(onConfirm)
    val active = enabled && !busy && !done && previewProgress == null
    val shownHolding = holding || previewProgress != null
    val fraction = { previewProgress ?: progress.floatValue }
    val danger = tint == c.danger
    val wash = if (danger) c.dangerWash else tint.copy(alpha = 0.13f)
    val fill = if (danger) c.dangerFill2 else tint.copy(alpha = 0.28f)
    val fg = if (done) Color.White else tint

    Box(
        modifier
            .height(height)
            .graphicsLayer { alpha = if (enabled || done) 1f else DISABLED_ALPHA }
            .clip(CircleShape)
            .background(if (done) tint else wash)
            // The fill that sweeps across while held: read while drawing, so a frame of the hold redraws, nothing more.
            .drawBehind {
                val p = fraction()
                if (p > 0f && !done) drawRect(fill, size = Size(size.width * p, size.height))
            }
            // One node with its title, so TalkBack reads the button by name and tests find its text by its tag.
            .semantics(mergeDescendants = true) {
                role = Role.Button
                if (!active) disabled()
                if (holding) stateDescription = "Keep holding"
                onClick(label = title) {
                    if (active) {
                        feedback.play(Event.HoldConfirmed)
                        confirm()
                    }
                    active
                }
            }
            .pointerInput(active, durationMillis) {
                if (!active) return@pointerInput
                var drain: Job? = null
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    drain?.cancel()
                    holding = true
                    var ticks: Job? = null
                    val run = scope.launch {
                        ticks = launch { tick(durationMillis) { step -> feedback.play(Event.HoldTick, step) } }
                        progress.floatValue = 0f
                        runHold(durationMillis, progress)
                        ticks?.cancel()
                        holding = false
                        feedback.play(Event.HoldConfirmed)
                        confirm()
                        progress.floatValue = 0f
                    }
                    waitForUpOrCancellation()
                    if (run.isActive) {
                        run.cancel()
                        ticks?.cancel()
                        holding = false
                        feedback.play(Event.HoldReleased)
                        drain = scope.launch { runDrain(progress) }
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Row(Modifier.padding(horizontal = 20.dp), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            when {
                busy -> {
                    Spinner(fg, size = 16.dp)
                    Spacer(Modifier.width(8.dp))
                }
                done -> {
                    GlyphIcon(Glyph.Check, fg, size = 20.dp)
                    Spacer(Modifier.width(8.dp))
                }
                shownHolding -> {
                    HoldRing(fraction, fg, fill)
                    Spacer(Modifier.width(10.dp))
                }
                icon != null -> {
                    Icon(icon, null, Modifier.size(20.dp), tint = fg)
                    Spacer(Modifier.width(8.dp))
                }
            }
            RText(
                when {
                    done -> doneTitle
                    shownHolding -> "Keep holding"
                    else -> title
                },
                RType.button,
                fg,
                maxLines = 1,
            )
        }
    }
}

/** The hold, everywhere (Signal `hold-duration`). */
const val HOLD_MILLIS = Motion.HOLD_MS

/** A released hold drains back in this long. */
const val HOLD_DRAIN_MILLIS = Motion.HOLD_DRAIN_MS

/** Disabled controls are drawn at 40 %. */
const val DISABLED_ALPHA = 0.4f

/** Pressed content dims to 55 %. */
const val PRESSED_ALPHA = 0.55f

/** How full a hold is after [elapsedNanos] of [durationMillis]: linear, 0 to 1. */
internal fun holdFraction(elapsedNanos: Long, durationMillis: Int): Float =
    (elapsedNanos / 1_000_000f / durationMillis).coerceIn(0f, 1f)

/** Fills [progress] from 0 to 1 over [durationMillis] of frame time; returns once full. */
internal suspend fun runHold(durationMillis: Int, progress: MutableFloatState) {
    val start = withFrameNanos { it }
    while (true) {
        val f = withFrameNanos { now -> holdFraction(now - start, durationMillis) }
        progress.floatValue = f
        if (f >= 1f) return
    }
}

/** Drains [progress] from where it is to 0 over [HOLD_DRAIN_MILLIS] of frame time. */
private suspend fun runDrain(progress: MutableFloatState) {
    val from = progress.floatValue
    if (from <= 0f) return
    val start = withFrameNanos { it }
    while (true) {
        val f = withFrameNanos { now -> holdFraction(now - start, HOLD_DRAIN_MILLIS) }
        progress.floatValue = from * (1f - f)
        if (f >= 1f) return
    }
}

/**
 * The haptic ticks of a hold: the gap between them shrinks as it fills (from about a quarter second to under a
 * tenth), each a step higher on the detent ladder, so the hold is felt and heard rising.
 */
internal suspend fun tick(durationMillis: Int, onTick: (step: Int) -> Unit) {
    var elapsed = 0L
    var step = 0
    while (elapsed < durationMillis) {
        onTick(step++)
        val gap = holdTickGap(elapsed.toFloat() / durationMillis)
        delay(gap)
        elapsed += gap
    }
}

/** The pause after a tick at [fraction] of the hold: 260 ms at the start, 70 ms at the end. */
internal fun holdTickGap(fraction: Float): Long = (260 - 190 * fraction.coerceIn(0f, 1f)).toLong()

/** The small ring beside "Keep holding": a track in the fill colour, the progress in the ink. */
@Composable
private fun HoldRing(progress: () -> Float, color: Color, track: Color) {
    Canvas(Modifier.size(20.dp)) {
        val stroke = size.width * 3f / 36f
        val inset = size.width * 4f / 36f
        val arc = Size(size.width - inset * 2, size.height - inset * 2)
        drawArc(track, 0f, 360f, false, Offset(inset, inset), arc, style = Stroke(stroke))
        drawArc(color, -90f, 360f * progress(), false, Offset(inset, inset), arc, style = Stroke(stroke, cap = StrokeCap.Round))
    }
}
