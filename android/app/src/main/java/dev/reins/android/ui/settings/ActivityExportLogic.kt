package dev.reins.android.ui.settings

import dev.reins.core.ActivityExport
import dev.reins.core.ActivityExportFormat
import java.text.NumberFormat
import java.util.Locale

/** The words of Settings > Export decision history. */
internal object ActivityExportText {
    const val ROW_TITLE = "Export decision history"
    const val ROW_SUBTITLE = "Save this phone's Activity as a file"
    const val SHEET_BODY =
        "Includes up to the last 5,000 decisions on this phone. Message contents aren't included. " +
            "The file isn't encrypted, so keep it somewhere safe."
    const val CSV_LABEL = "Spreadsheet (CSV)"
    const val JSON_LABEL = "JSON"
    const val EMPTY = "No decisions to export yet"
    const val AUTH_TITLE = "Export decision history"
    const val AUTH_SUBTITLE = "The file isn't encrypted"
    const val AUTH_UNAVAILABLE = "Set a screen lock or fingerprint on this phone to export your decision history."
    const val WRITE_FAILED = "Couldn't save the file. Try again."

    /** "Exported 1 decision", "Exported 1,204 decisions". */
    fun exported(count: UInt, locale: Locale = Locale.getDefault()): String {
        val n = NumberFormat.getIntegerInstance(locale).format(count.toLong())
        return if (count == 1u) "Exported $n decision" else "Exported $n decisions"
    }
}

/** An export the core made, held only until the person has picked where it goes (or backed out). [launched]: the picker is up. */
internal data class PendingExport(val format: ActivityExportFormat, val export: ActivityExport, val launched: Boolean = false)
