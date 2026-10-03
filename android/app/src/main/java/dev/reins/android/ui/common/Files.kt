package dev.reins.android.ui.common

import java.util.Locale

/** "812 bytes", "18.0 KB", "1.4 MB": the way the core words sizes. */
fun fileSize(bytes: ULong): String {
    val b = bytes.toDouble()
    return when {
        bytes < 1024u -> "$bytes bytes"
        bytes < 1024u * 1024u -> String.format(Locale.ROOT, "%.1f KB", b / 1024.0)
        bytes < 1024u * 1024u * 1024u -> String.format(Locale.ROOT, "%.1f MB", b / (1024.0 * 1024.0))
        else -> String.format(Locale.ROOT, "%.1f GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

/** The start and the end of a SHA-256, enough to compare by eye: "9f86d081884c7d65…0a08". */
fun shortSha(hex: String): String = if (hex.length <= 24) hex else hex.take(16) + "…" + hex.takeLast(4)

/** A file whose preview is the start of its text (as opposed to a description of a binary file). */
fun isTextFile(contentType: String): Boolean {
    val type = contentType.substringBefore(';').trim().lowercase()
    return type.startsWith("text/") || type == "application/json" || type.endsWith("+json") || type == "application/xml" ||
        type.endsWith("+xml") || type == "application/x-ndjson"
}
