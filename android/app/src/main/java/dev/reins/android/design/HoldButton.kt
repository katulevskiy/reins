package dev.reins.android.design

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
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
import androidx.compose.ui.text.font.FontWeight
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
 * [durationMillis]. While held, a fill sweeps across it, a small ring fills beside "Keep holding", haptic ticks speed up
 * and climb in pitch; on completion a heavy haptic and the success sound, then [onConfirm]. Letting go early cancels,
 * with a soft release cue.
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
    onConfirm: () -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val scope = rememberCoroutineScope()
    val progress = remember { Animatable(0f) }
    var holding by remember { mutableStateOf(false) }
    val confirm by rememberUpdatedState(onConfirm)
    val active = enabled && !busy
    // Dark text on the bright reds of the dark theme; white on the deeper ones of the light theme.
    val fg = if (c.dark) Color(0xFF2A0606) else Color.White

    Box(
        modifier
            .height(height)
            .graphicsLayer { alpha = if (enabled) 1f else 0.45f }
            .clip(CircleShape)
            .background(tint)
            .semantics {
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
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    holding = true
                    var ticks: Job? = null
                    val run = scope.launch {
                        ticks = launch { tick(durationMillis) { step -> feedback.play(Event.HoldTick, step) } }
                        progress.snapTo(0f)
                        progress.animateTo(1f, tween(durationMillis, easing = LinearEasing))
                        ticks?.cancel()
                        holding = false
                        feedback.play(Event.HoldConfirmed)
                        confirm()
                        progress.snapTo(0f)
                    }
                    waitForUpOrCancellation()
                    if (run.isActive) {
                        run.cancel()
                        ticks?.cancel()
                        holding = false
                        feedback.play(Event.HoldReleased)
                        scope.launch { progress.animateTo(0f, tween(220)) }
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        // The fill that sweeps across while held.
        Box(
            Modifier
                .align(Alignment.CenterStart)
                .fillMaxHeight()
                .fillMaxWidth(progress.value.coerceIn(0f, 1f))
                .background(Color.Black.copy(alpha = 0.22f)),
        )
        Row(Modifier.padding(horizontal = 20.dp), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            when {
                busy -> {
                    Spinner(fg, size = 16.dp)
                    Spacer(Modifier.width(8.dp))
                }
                holding -> {
                    HoldRing(progress.value, fg)
                    Spacer(Modifier.width(8.dp))
                }
                icon != null -> {
                    Icon(icon, null, Modifier.size(19.dp), tint = fg)
                    Spacer(Modifier.width(8.dp))
                }
            }
            RText(if (holding) "Keep holding" else title, RType.sans(16.5f, FontWeight.SemiBold), fg, maxLines = 1)
        }
    }
}

/** The usual hold, about two seconds. */
const val HOLD_MILLIS = 2_000

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

/** The small ring beside "Keep holding". */
@Composable
private fun HoldRing(fraction: Float, color: Color) {
    Canvas(Modifier.size(18.dp)) {
        val stroke = 2.4.dp.toPx()
        val inset = stroke / 2
        val arc = Size(size.width - stroke, size.height - stroke)
        drawArc(color.copy(alpha = 0.25f), 0f, 360f, false, Offset(inset, inset), arc, style = Stroke(stroke))
        drawArc(color, -90f, 360f * fraction, false, Offset(inset, inset), arc, style = Stroke(stroke, cap = StrokeCap.Round))
    }
}
