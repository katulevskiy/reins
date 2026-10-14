package dev.reins.android.ui.approval

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.ui.common.FileCard
import dev.reins.core.BlobView

/** "for 30 minutes", "for 1 hour", "for 1 h 30 min": how long the desktop app may keep secrets. */
fun leaseLabel(secs: ULong): String {
    val minutes = (secs / 60u).toLong().coerceAtLeast(1)
    if (minutes < 60) return if (minutes == 1L) "for 1 minute" else "for $minutes minutes"
    val hours = minutes / 60
    val rest = minutes % 60
    return when {
        rest != 0L -> "for $hours h $rest min"
        hours == 1L -> "for 1 hour"
        else -> "for $hours hours"
    }
}

/** The server of an SSH sign-in: its name when the desktop app knows it, else its host key. */
fun sshTarget(host: String?, hostKey: String?): String =
    host?.trim()?.takeIf { it.isNotEmpty() } ?: hostKey?.trim()?.takeIf { it.isNotEmpty() } ?: "an unknown server"

// ---- a file the change uses -----------------------------------------------------------------------------------------

/** The file an AI uploaded for this change, as the server saw it. */
@Composable
internal fun AttachedFileSection(blob: BlobView) {
    Column(Modifier.testTag("attachedFile")) {
        GroupHeader("Attached file")
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp)) { FileCard(blob) }
    }
}
