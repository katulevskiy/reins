package dev.rewarden.android.ui.approval

import dev.rewarden.core.GitFileView
import dev.rewarden.core.GitRefView
import java.util.Locale

/** Commits shown before "and N more". */
const val GIT_COMMITS_SHOWN = 5

/** Files shown before "and N more". */
const val GIT_FILES_SHOWN = 8

/** What an update does to the history of a ref. */
enum class GitHistory { Adds, Rewrites, Unknown }

/** An update that could not be checked is treated as a force push, and said so apart from one that is known to be. */
fun gitHistory(ref: GitRefView): GitHistory = when {
    ref.forceUnknown -> GitHistory.Unknown
    ref.force -> GitHistory.Rewrites
    else -> GitHistory.Adds
}

/** The chip next to a ref: "New branch", "Update", "Delete", "New tag", "Tag" (a tag that moves). */
fun gitChipLabel(ref: GitRefView): String = when (ref.change) {
    "delete" -> "Delete"
    "create" -> when (ref.kind) {
        "branch" -> "New branch"
        "tag" -> "New tag"
        else -> "New"
    }
    else -> if (ref.kind == "tag") "Tag" else "Update"
}

private fun files(n: UInt) = if (n == 1u) "1 file" else "$n files"

/** "+120 −14 in 12 files"; a count the desktop app could not make is left out. `null` when no file changes. */
fun gitTotalsLabel(ref: GitRefView): String? {
    if (ref.filesChanged == 0u) return null
    val counts = listOfNotNull(ref.additions?.let { "+$it" }, ref.deletions?.let { "−$it" })
    return if (counts.isEmpty()) "${files(ref.filesChanged)} changed" else "${counts.joinToString(" ")} in ${files(ref.filesChanged)}"
}

fun gitFileLetter(status: String): String = when (status) {
    "added" -> "A"
    "modified" -> "M"
    "deleted" -> "D"
    "type_changed" -> "T"
    else -> "?"
}

/** "+3 −1", "binary", or nothing when the lines were not counted. */
fun gitFileCounts(file: GitFileView): String {
    if (file.binary) return "binary"
    return listOfNotNull(file.additions?.let { "+$it" }, file.deletions?.let { "−$it" }).joinToString(" ")
}

private val SIZE_UNITS = listOf("KB", "MB", "GB")

/** "0 B", "12.4 KB", "3.0 MB" (powers of 1024). */
fun packSizeLabel(bytes: ULong): String {
    if (bytes < 1024u) return "$bytes B"
    var value = bytes.toDouble() / 1024.0
    var unit = 0
    while (value >= 1024.0 && unit < SIZE_UNITS.lastIndex) {
        value /= 1024.0
        unit++
    }
    return String.format(Locale.ROOT, "%.1f %s", value, SIZE_UNITS[unit])
}
