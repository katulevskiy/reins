package dev.reins.android.ui.common

import java.text.DateFormat
import java.util.Date

/** Unix seconds → locale date and time. */
fun formatTime(epochSeconds: Long): String =
    DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(epochSeconds * 1000))

/** "expires in 2 h" style remaining time for an expiry in unix seconds. */
fun formatExpiry(
    expiresAt: Long?,
    nowSeconds: Long = (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000,
): String {
    if (expiresAt == null) return "no time limit"
    val left = expiresAt - nowSeconds
    return when {
        left <= 0 -> "expired"
        left < 3_600 -> "expires in ${maxOf(1, left / 60)} min"
        left < 86_400 -> "expires in ${left / 3_600} h"
        else -> "expires in ${left / 86_400} days"
    }
}

/** "just now", "5 min ago", "3 h ago", "yesterday", else the date. */
fun relativeTime(
    epochSeconds: Long,
    nowSeconds: Long = (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000,
): String {
    val d = nowSeconds - epochSeconds
    return when {
        d < 45 -> "just now"
        d < 3_600 -> "${maxOf(1, d / 60)} min ago"
        d < 86_400 -> "${d / 3_600} h ago"
        d < 2 * 86_400 -> "yesterday"
        d < 7 * 86_400 -> "${d / 86_400} days ago"
        else -> java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(epochSeconds * 1000))
    }
}

/** Date and time, spelled out. */
fun formatFull(epochSeconds: Long): String =
    java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.LONG, java.text.DateFormat.SHORT).format(java.util.Date(epochSeconds * 1000))
