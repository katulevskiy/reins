package dev.rewarden.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.rewarden.core.GrantView

// ---- what a grant's clock says (pure, unit-tested) ----------------------------------------------------------------

private const val MINUTE = 60L
private const val HOUR = 3_600L
private const val DAY = 86_400L
private const val WEEK = 7 * DAY
private const val YEAR = 365 * DAY

/** "47m", "3h", "5d", "10w", "2y": the time left in its largest unit (seconds only for the last minute). */
fun compactDuration(seconds: Long): String {
    val s = seconds.coerceAtLeast(0)
    return when {
        s < MINUTE -> "${s}s"
        s < HOUR -> "${s / MINUTE}m"
        s < DAY -> "${s / HOUR}h"
        s < WEEK -> "${s / DAY}d"
        s < YEAR -> "${s / WEEK}w"
        else -> "${s / YEAR}y"
    }
}

/** How long before it ends a grant counts as ending soon (and the user is reminded): a tenth of its life, 5 min to 1 h. */
fun expiryLeadSeconds(createdAt: Long, expiresAt: Long): Long = ((expiresAt - createdAt) / 10).coerceIn(5 * MINUTE, HOUR)

/** The moment the reminder is due, or null for a grant that does not run out on its own. */
fun reminderAt(grant: GrantView): Long? = grant.expiresAt?.let { it - expiryLeadSeconds(grant.createdAt, it) }

/** What is left of a grant's time, as of [nowSeconds]. */
data class GrantClock(
    /** Seconds left; null when it never expires on its own. */
    val remaining: Long?,
    /** 1 when it has just begun, 0 when it ends; 1 for a grant without an end. */
    val fraction: Float,
    val soon: Boolean,
) {
    val label: String get() = remaining?.let(::compactDuration) ?: "∞"
}

fun grantClock(grant: GrantView, nowSeconds: Long): GrantClock {
    val end = grant.expiresAt ?: return GrantClock(null, 1f, false)
    val remaining = (end - nowSeconds).coerceAtLeast(0)
    val total = (end - grant.createdAt).coerceAtLeast(1)
    return GrantClock(
        remaining = remaining,
        fraction = (remaining.toFloat() / total).coerceIn(0f, 1f),
        soon = grant.active && remaining <= expiryLeadSeconds(grant.createdAt, end),
    )
}

/** The colour of a running grant's clock: calm, or the warning colour once it is about to end. */
fun clockColor(clock: GrantClock, colors: RColors): Color = if (clock.soon) colors.danger else colors.accent

// ---- visuals ------------------------------------------------------------------------------------------------------

/**
 * The time left as a pie that shrinks clockwise as time passes, with the amount in its largest unit written on top
 * ("47m", "3h", "5d").
 */
@Composable
fun ExpiryPie(clock: GrantClock, modifier: Modifier = Modifier, size: Dp = 52.dp) {
    val c = LocalColors.current
    val color = clockColor(clock, c)
    Box(modifier.size(size).testTag("pie"), contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(size)) {
            val stroke = 2.dp.toPx()
            val inner = Size(this.size.width - stroke, this.size.height - stroke)
            val topLeft = Offset(stroke / 2, stroke / 2)
            drawCircle(c.controlFill, radius = this.size.minDimension / 2)
            if (clock.fraction > 0f) {
                // The slice that is gone is bitten off clockwise from 12 o'clock; the wedge that remains ends there.
                drawArc(
                    color = color.copy(alpha = 0.30f),
                    startAngle = -90f + 360f * (1f - clock.fraction),
                    sweepAngle = 360f * clock.fraction,
                    useCenter = true,
                    topLeft = topLeft,
                    size = inner,
                )
            }
            drawCircle(color.copy(alpha = 0.55f), radius = (this.size.minDimension - stroke) / 2, style = Stroke(stroke / 2))
        }
        RText(clock.label, RType.sans(size.value * 0.27f, FontWeight.Bold), color, maxLines = 1, ltr = true)
    }
}

/** How much of a grant has been used: dots for a small cap, a bar for a big one, a count when there is no cap. */
@Composable
fun UsesMeter(uses: Int, maxUses: Int?, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier.testTag("uses"), verticalAlignment = Alignment.CenterVertically) {
        when {
            maxUses == null -> {
                GlyphIcon(Glyph.Refresh, if (uses == 0) c.tertiary else c.secondary, size = 13.dp)
                Spacer(Modifier.width(5.dp))
                RText(
                    when (uses) {
                        0 -> "Not used yet"
                        1 -> "Used once"
                        else -> "Used $uses times"
                    },
                    RType.sans(13f, FontWeight.Medium),
                    if (uses == 0) c.tertiary else c.secondary,
                )
            }
            maxUses <= 8 -> {
                repeat(maxUses) { i ->
                    val used = i < uses
                    Box(
                        Modifier
                            .size(9.dp)
                            .then(
                                if (used) {
                                    Modifier.background(c.accent, androidx.compose.foundation.shape.CircleShape)
                                } else {
                                    Modifier.border(1.5.dp, c.tertiary, androidx.compose.foundation.shape.CircleShape)
                                },
                            ),
                    )
                    Spacer(Modifier.width(4.dp))
                }
                Spacer(Modifier.width(4.dp))
                RText(if (uses >= maxUses) "All used" else "${maxUses - uses} left", RType.sans(13f, FontWeight.Medium), c.secondary)
            }
            else -> {
                val fraction = (uses.toFloat() / maxUses).coerceIn(0f, 1f)
                Box(
                    Modifier
                        .width(56.dp)
                        .height(6.dp)
                        .background(c.controlFill, androidx.compose.foundation.shape.CircleShape),
                ) {
                    Box(Modifier.fillMaxWidth(fraction).height(6.dp).background(c.accent, androidx.compose.foundation.shape.CircleShape))
                }
                Spacer(Modifier.width(8.dp))
                RText("$uses of $maxUses", RType.sans(13f, FontWeight.Medium), c.secondary)
            }
        }
    }
}
