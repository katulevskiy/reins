package dev.reins.android.ui.common

/**
 * Text that came from outside the app (senders, subjects, snippets, commands, client names). Bidirectional controls,
 * zero-width and other invisible format characters are removed so it cannot reorder or hide what is shown, and other
 * control characters become spaces. Never shortened: what is approved is shown whole ([untrustedName] is the capped
 * one, for names and labels only).
 */
fun untrusted(text: String): String = clean(text).trim()

/**
 * A name the other side chose (a computer, an AI app, a phone, a file), for one line: [untrusted], with line breaks and
 * tabs as spaces, runs of spaces collapsed, and at most [max] characters (then "…"). It is shown quoted, apart from
 * what Reins itself says, so it cannot pass for Reins. Only for names, labels and titles, never for what is approved.
 */
fun untrustedName(text: String, max: Int = NAME_MAX): String {
    val one = untrusted(text)
        .replace('\n', ' ')
        .replace('\t', ' ')
        .replace(Regex(" {2,}"), " ")
        .trim()
    if (one.length <= max) return one
    // Never split a surrogate pair.
    val cut = one.take(max - 1).let { if (it.last().isHighSurrogate()) it.dropLast(1) else it }
    return cut.trimEnd() + "…"
}

/** [untrustedName] in quotes, so it reads as what the other side calls itself; "No name" when it sent none. */
fun quotedName(text: String, max: Int = NAME_MAX): String = untrustedName(text, max).let { if (it.isEmpty()) "No name" else "“$it”" }

/** The longest name shown from the other side. */
const val NAME_MAX = 64

/** [untrusted] for a line whose leading spaces matter (a line of code in a diff): nothing is trimmed. */
fun untrustedLine(text: String): String = clean(text)

private fun clean(text: String): String {
    val out = StringBuilder(text.length)
    var i = 0
    while (i < text.length) {
        val cp = text.codePointAt(i)
        i += Character.charCount(cp)
        when {
            isHidden(cp) -> Unit
            cp == '\n'.code || cp == '\t'.code -> out.appendCodePoint(cp)
            Character.isISOControl(cp) || cp == LINE_SEPARATOR || cp == PARAGRAPH_SEPARATOR -> out.append(' ')
            else -> out.appendCodePoint(cp)
        }
    }
    return out.toString()
}

/**
 * What shows nothing yet changes how text reads: the bidi controls, zero-width spaces and joiners (U+200B..U+200F),
 * the word joiner (U+2060..U+2064), the byte-order mark, the soft hyphen, tag characters (U+E0000..U+E007F) and any
 * other format character.
 */
private fun isHidden(cp: Int): Boolean =
    (cp <= 0xFFFF && isBidiControl(cp.toChar())) ||
        cp in 0x200B..0x200F || cp in 0x2060..0x2064 || cp == 0xFEFF || cp == 0x00AD || cp in 0xE0000..0xE007F ||
        Character.getType(cp) == Character.FORMAT.toInt()

private const val LINE_SEPARATOR = 0x2028
private const val PARAGRAPH_SEPARATOR = 0x2029

/** LRM, RLM, ALM, the embedding/override controls (LRE..RLO) and the isolates (LRI..PDI). */
private fun isBidiControl(ch: Char): Boolean = when (ch.code) {
    0x200E, 0x200F, 0x061C -> true
    in 0x202A..0x202E -> true
    in 0x2066..0x2069 -> true
    else -> false
}
