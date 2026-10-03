package dev.reins.android.ui.pairing

import java.net.URI
import java.net.URISyntaxException
import java.net.URLDecoder

/**
 * The code a computer shows to pair with this phone ("BCDF-GHJK"): typed in, scanned from a QR code, or in a link
 * (`https://app.reins2fa.com/pair?code=BCDF-GHJK`, a self-hosted server's own host with the same path, or
 * `reins://pair?code=BCDF-GHJK`). Whatever arrives is untrusted: only a well-formed code ever leaves [parse], and the
 * core then asks the signed-in account's own server what it stands for.
 */
object PairingCode {
    /** No vowels (no words) and no letters that read like digits. */
    const val ALPHABET = "BCDFGHJKLMNPQRSTVWXZ"
    const val LENGTH = 8

    /** Longer input is not a code or a link to one. */
    private const val MAX_INPUT = 2048

    /**
     * The code in a scanned text, a link or what the user typed, as "XXXX-XXXX"; null when there is none. Case, spaces
     * and dashes in a bare code are ignored.
     */
    fun parse(text: String?): String? {
        val input = text?.trim() ?: return null
        if (input.isEmpty() || input.length > MAX_INPUT) return null
        normalize(input)?.let { return it }
        return fromLink(input)
    }

    /** A bare code, or null. */
    fun normalize(raw: String): String? {
        // ASCII only, like the core: no other script's letter is folded onto one of the alphabet's.
        val letters = raw.filterNot { it == '-' || it.isWhitespace() }.map { if (it in 'a'..'z') it.uppercaseChar() else it }.joinToString("")
        if (letters.length != LENGTH || letters.any { it !in ALPHABET }) return null
        return letters.substring(0, LENGTH / 2) + "-" + letters.substring(LENGTH / 2)
    }

    private fun fromLink(input: String): String? {
        val uri = try {
            URI(input)
        } catch (e: URISyntaxException) {
            return null
        }
        val pairLink = when (uri.scheme?.lowercase(java.util.Locale.ROOT)) {
            "https", "http" -> !uri.host.isNullOrEmpty() && uri.path.orEmpty().trimEnd('/').endsWith("/pair")
            "reins" -> uri.host.equals("pair", ignoreCase = true) && uri.path.orEmpty().trim('/').isEmpty()
            else -> false
        }
        if (!pairLink) return null
        val query = uri.rawQuery ?: return null
        val values = query.split('&').mapNotNull { part ->
            val eq = part.indexOf('=')
            if (eq < 0 || part.substring(0, eq) != "code") return@mapNotNull null
            try {
                // The charset-name overload: the Charset one needs API 33.
                URLDecoder.decode(part.substring(eq + 1), "UTF-8")
            } catch (e: IllegalArgumentException) {
                null
            }
        }
        // Exactly one code: a link carrying two is not one the server made.
        return values.singleOrNull()?.let(::normalize)
    }
}
