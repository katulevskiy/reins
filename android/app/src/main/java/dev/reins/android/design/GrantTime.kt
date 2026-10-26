package dev.reins.android.design

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
import dev.reins.core.GrantView

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

