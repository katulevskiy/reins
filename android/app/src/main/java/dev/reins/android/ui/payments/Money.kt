package dev.reins.android.ui.payments

import java.time.LocalDate
import java.time.ZoneId

/**
 * Amounts as the core keeps them: whole numbers in a currency's smallest unit (cents for USD, yen for JPY). The same
 * rules as `reins_proto::payments`, so what the user types is read exactly as the core would.
 */
object Money {
    private val noDecimals = setOf(
        "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "UYI", "VND", "VUV", "XAF", "XOF", "XPF",
    )
    private val threeDecimals = setOf("BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND")

    fun decimals(currency: String): Int = when (currency.uppercase()) {
        in noDecimals -> 0
        in threeDecimals -> 3
        else -> 2
    }

    /** "25", "25.5" or "25.50" in [currency] as minor units; null for anything else (signs, too many decimals). */
    fun parse(text: String, currency: String): Long? {
        val t = text.trim().removePrefix("$").removePrefix("€").removePrefix("£").trim()
        if (t.isEmpty() || t.length > 15) return null
        val parts = t.split('.')
        if (parts.size > 2 || parts[0].isEmpty() || !parts.all { p -> p.all { it.isDigit() } }) return null
        val exp = decimals(currency)
        val frac = parts.getOrNull(1) ?: ""
        if (frac.length > exp || (parts.size == 2 && frac.isEmpty())) return null
        val whole = parts[0].toLongOrNull() ?: return null
        var scale = 1L
        repeat(exp) { scale *= 10 }
        val minor = whole * scale + (frac.padEnd(exp, '0').ifEmpty { "0" }.toLong())
        return minor.takeIf { it > 0 && it <= 100_000_000_000L }
    }

    /** Minor units for people: "$19.98", "€5.00", "¥500", else "19.98 CHF". */
    fun format(minor: Long, currency: String): String {
        val exp = decimals(currency)
        val abs = kotlin.math.abs(minor)
        var scale = 1L
        repeat(exp) { scale *= 10 }
        val n = if (exp == 0) "$abs" else "${abs / scale}.${(abs % scale).toString().padStart(exp, '0')}"
        val sign = if (minor < 0) "-" else ""
        return when (currency.uppercase()) {
            "USD" -> "$sign$$n"
            "EUR" -> "$sign€$n"
            "GBP" -> "$sign£$n"
            "JPY" -> "$sign¥$n"
            "INR" -> "$sign₹$n"
            "CAD" -> "${sign}CA$$n"
            "AUD" -> "${sign}A$$n"
            else -> "$sign$n ${currency.uppercase()}"
        }
    }

    /** Minor units as a plain number for an input field ("25.00"). */
    fun plain(minor: Long, currency: String): String {
        val exp = decimals(currency)
        if (exp == 0) return "$minor"
        var scale = 1L
        repeat(exp) { scale *= 10 }
        return "${minor / scale}.${(minor % scale).toString().padStart(exp, '0')}"
    }

    /** The start of the current month on this phone, in unix seconds: "spent this month". */
    fun monthStart(zone: ZoneId = ZoneId.systemDefault(), today: LocalDate = LocalDate.now(zone)): Long =
        today.withDayOfMonth(1).atStartOfDay(zone).toEpochSecond()
}
