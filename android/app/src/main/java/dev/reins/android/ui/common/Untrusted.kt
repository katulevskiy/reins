package dev.reins.android.ui.common

/**
 * Text that came from outside the app (senders, subjects, snippets, client names). Bidirectional controls are
 * removed so it cannot reorder what is shown around it, and other control characters become spaces.
 */
fun untrusted(text: String): String {
    val out = StringBuilder(text.length)
    for (ch in text) {
        when {
            isBidiControl(ch) -> Unit
            ch == '\n' || ch == '\t' -> out.append(ch)
            ch.isISOControl() || ch.code == LINE_SEPARATOR || ch.code == PARAGRAPH_SEPARATOR -> out.append(' ')
            else -> out.append(ch)
        }
    }
    return out.toString().trim()
}

private const val LINE_SEPARATOR = 0x2028
private const val PARAGRAPH_SEPARATOR = 0x2029

/** LRM, RLM, ALM, the embedding/override controls (LRE..RLO) and the isolates (LRI..PDI). */
private fun isBidiControl(ch: Char): Boolean = when (ch.code) {
    0x200E, 0x200F, 0x061C -> true
    in 0x202A..0x202E -> true
    in 0x2066..0x2069 -> true
    else -> false
}
