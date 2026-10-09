package dev.reins.android.ui.common

import java.text.DateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * A locale date format, built once per thread (a [DateFormat] is slow to build and not thread-safe) and again when the
 * locale changes. Lists format a date per row on every pass, so this keeps scrolling from building one each time.
 */
private class CachedDateFormat(private val build: () -> DateFormat) {
    private val local = ThreadLocal<Pair<Locale, DateFormat>>()

    fun format(epochSeconds: Long): String {
        val locale = Locale.getDefault()
        val format = local.get()?.takeIf { it.first == locale }?.second ?: build().also { local.set(locale to it) }
        format.timeZone = TimeZone.getDefault()
        return format.format(Date(epochSeconds * 1000))
    }
}

private val mediumDateShortTime = CachedDateFormat { DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT) }
private val mediumDate = CachedDateFormat { DateFormat.getDateInstance(DateFormat.MEDIUM) }
private val longDateShortTime = CachedDateFormat { DateFormat.getDateTimeInstance(DateFormat.LONG, DateFormat.SHORT) }

/** Unix seconds → locale date and time. */
fun formatTime(epochSeconds: Long): String = mediumDateShortTime.format(epochSeconds)

/** Unix seconds → locale date. */
fun formatDate(epochSeconds: Long): String = mediumDate.format(epochSeconds)

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
        else -> mediumDate.format(epochSeconds)
    }
}

/** Date and time, spelled out. */
fun formatFull(epochSeconds: Long): String = longDateShortTime.format(epochSeconds)
