package dev.reins.android.design

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathMeasure
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

/** Global switch so tests and screenshots can freeze the clock. */
object Timers {
    @Volatile
    var live: Boolean = true

    /** Fixed "now" (unix milliseconds) used when [live] is off and a value is set. */
    @Volatile
    var frozenNowMillis: Long? = null
}

/** The current time in unix milliseconds, refreshed every [intervalMillis] while timers are live. */
@Composable
fun rememberNowMillis(intervalMillis: Long = 250): Long = rememberNowState(intervalMillis).value

/**
 * The current time in unix milliseconds as a state, refreshed every [intervalMillis] while timers are live and until
 * [untilMillis] has passed (null: nothing counts down, so it never ticks). Read it where the time is shown (a text of
 * its own, a draw lambda), so a tick redraws that and not the whole card around it.
 */
@Composable
fun rememberNowState(intervalMillis: Long = 250, untilMillis: Long? = Long.MAX_VALUE): State<Long> {
    val now = remember { mutableLongStateOf(Timers.frozenNowMillis ?: System.currentTimeMillis()) }
    if (LocalLiveTimers.current && untilMillis != null) {
        LaunchedEffect(intervalMillis, untilMillis) {
            do {
                delay(intervalMillis)
                now.longValue = System.currentTimeMillis()
            } while (now.longValue < untilMillis)
        }
    }
    return now
}

/** After this many seconds left, the countdown turns red. */
const val URGENT_SECONDS = 15L

/** How urgent a request is, from when the AI stops waiting. */
data class Urgency(val remainingSeconds: Long, val fraction: Float, val stale: Boolean, val urgent: Boolean)

fun urgency(createdAt: Long, waitUntil: Long?, nowMillis: Long): Urgency? {
    if (waitUntil == null) return null
    val total = (waitUntil - createdAt).coerceAtLeast(1)
    val remaining = (waitUntil * 1000 - nowMillis) / 1000.0
    val fraction = (remaining / total).toFloat().coerceIn(0f, 1f)
    val stale = remaining <= 0
    return Urgency(remaining.toLong().coerceAtLeast(0), fraction, stale, !stale && remaining <= URGENT_SECONDS)
}

/**
 * A card whose 2 dp border is a progress bar around its perimeter: [fraction] of the outline is painted in [color]
 * (from the top-left corner round to where the time has run out); the rest shows the hairline track.
 */
@Composable
fun TimeBarFrame(
    fraction: Float,
    color: Color,
    modifier: Modifier = Modifier,
    corner: Dp = 18.dp,
    content: @Composable () -> Unit,
) = TimeBarFrame({ fraction }, { color }, modifier, corner, content)

/**
 * [TimeBarFrame] for a clock that ticks: [fraction] and [color] are read only while drawing, so a tick redraws the
 * border and recomposes nothing. The outline and its measure are built once per size.
 */
@Composable
fun TimeBarFrame(
    fraction: () -> Float,
    color: () -> Color,
    modifier: Modifier = Modifier,
    corner: Dp = 18.dp,
    content: @Composable () -> Unit,
) {
    val c = LocalColors.current
    Box(
        modifier.drawWithCache {
            val strokeWidth = 2.dp.toPx()
            val inset = strokeWidth / 2
            val rr = RoundRect(
                inset, inset, size.width - inset, size.height - inset,
                CornerRadius(corner.toPx() - inset),
            )
            val path = Path().apply { addRoundRect(rr) }
            val measure = PathMeasure().apply { setPath(path, false) }
            val length = measure.length
            val segment = Path()
            val track = Stroke(width = strokeWidth)
            val bar = Stroke(width = strokeWidth, cap = StrokeCap.Round)
            onDrawWithContent {
                drawContent()
                // The track is always the full outline; what is left of the time is painted over it.
                drawPath(path, c.hairline, style = track)
                val f = fraction()
                if (f > 0f) {
                    segment.rewind()
                    measure.getSegment(length * (1f - f.coerceAtMost(1f)), length, segment, true)
                    drawPath(segment, color(), style = bar)
                }
            }
        },
    ) { content() }
}

/**
 * The frame of a request that waits for the user: full in the accent colour when the request arrives, emptying as
 * the AI's patience runs out, turning red for the last [URGENT_SECONDS] seconds. [now] is the clock it runs on; a card
 * that also shows the seconds left passes its own, so one clock drives both.
 */
@Composable
fun CountdownFrame(
    createdAt: Long,
    waitUntil: Long?,
    modifier: Modifier = Modifier,
    corner: Dp = 18.dp,
    now: State<Long> = rememberNowState(untilMillis = waitUntil?.let { it * 1000 }),
    content: @Composable () -> Unit,
) {
    val c = LocalColors.current
    // Only the change of colour recomposes; the shrinking bar is read while drawing.
    val tone by remember(createdAt, waitUntil, c, now) {
        derivedStateOf {
            val u = urgency(createdAt, waitUntil, now.value)
            when {
                u == null -> c.accent
                u.stale -> c.tertiary
                u.urgent -> c.danger
                else -> c.accent
            }
        }
    }
    val active = animateColorAsState(tone, label = "countdown")
    TimeBarFrame(
        fraction = { urgency(createdAt, waitUntil, now.value)?.let { if (it.stale) 0f else it.fraction } ?: 1f },
        color = { active.value },
        modifier = modifier,
        corner = corner,
        content = content,
    )
}
