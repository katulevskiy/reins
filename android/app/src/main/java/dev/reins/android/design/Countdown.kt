package dev.reins.android.design

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
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
fun rememberNowMillis(intervalMillis: Long = 250): Long {
    var now by remember { mutableLongStateOf(Timers.frozenNowMillis ?: System.currentTimeMillis()) }
    val live = LocalLiveTimers.current
    if (live) {
        LaunchedEffect(intervalMillis) {
            while (true) {
                delay(intervalMillis)
                now = System.currentTimeMillis()
            }
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
) {
    val c = LocalColors.current
    Box(
        modifier.drawWithContent {
            drawContent()
            val strokeWidth = 2.dp.toPx()
            val inset = strokeWidth / 2
            val rr = RoundRect(
                inset, inset, size.width - inset, size.height - inset,
                CornerRadius(corner.toPx() - inset),
            )
            val path = Path().apply { addRoundRect(rr) }
            // The track is always the full outline; what is left of the time is painted over it.
            drawPath(path, c.hairline, style = Stroke(width = strokeWidth))
            if (fraction > 0f) {
                val measure = PathMeasure().apply { setPath(path, false) }
                val length = measure.length
                val segment = Path()
                measure.getSegment(length * (1f - fraction.coerceAtMost(1f)), length, segment, true)
                drawPath(segment, color, style = Stroke(width = strokeWidth, cap = StrokeCap.Round))
            }
        },
    ) { content() }
}

/**
 * The frame of a request that waits for the user: full in the accent colour when the request arrives, emptying as
 * the AI's patience runs out, turning red for the last [URGENT_SECONDS] seconds.
 */
@Composable
fun CountdownFrame(
    createdAt: Long,
    waitUntil: Long?,
    modifier: Modifier = Modifier,
    corner: Dp = 18.dp,
    content: @Composable () -> Unit,
) {
    val c = LocalColors.current
    val now = rememberNowMillis()
    val u = urgency(createdAt, waitUntil, now)
    val active by animateColorAsState(
        when {
            u == null -> c.accent
            u.stale -> c.tertiary
            u.urgent -> c.danger
            else -> c.accent
        },
        label = "countdown",
    )
    TimeBarFrame(if (u?.stale == true) 0f else u?.fraction ?: 1f, active, modifier, corner, content)
}
