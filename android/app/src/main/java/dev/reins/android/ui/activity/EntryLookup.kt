package dev.reins.android.ui.activity

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.userMessage
import dev.reins.core.ActivityEntry
import dev.reins.core.ReinsCoreInterface
import kotlin.coroutines.cancellation.CancellationException

/** An Activity entry being opened by its id. */
sealed interface EntryLookup {
    /** Not in the history read at once; the core is being asked for it. */
    data object Loading : EntryLookup

    data class Found(val entry: ActivityEntry) : EntryLookup

    /** The core has no such entry. */
    data object Gone : EntryLookup

    /** The core could not be read. */
    data class Failed(val message: String) : EntryLookup
}

val EntryLookup.entry: ActivityEntry? get() = (this as? EntryLookup.Found)?.entry

/**
 * Entry [entryId]: from the history read at once ([AppState.activity], the newest few hundred) when it is there, else
 * read from [core] by its id; a day or a search opens entries far older than that history. Without a core (previews)
 * an entry outside the history is [EntryLookup.Gone].
 */
@Composable
fun rememberEntry(entryId: Long, state: AppState, core: ReinsCoreInterface?): EntryLookup {
    val entries by state.activity.collectAsStateWithLifecycle()
    val listed = entries.firstOrNull { it.id == entryId }
    var read by remember(entryId) { mutableStateOf<EntryLookup>(EntryLookup.Loading) }
    val missing = listed == null
    LaunchedEffect(entryId, listed, core) {
        // Kept, so an entry newer ones push out of that history while it is open stays as it was.
        if (listed != null) read = EntryLookup.Found(listed)
        if (!missing || read is EntryLookup.Found) return@LaunchedEffect
        read = if (core == null) {
            EntryLookup.Gone
        } else {
            try {
                core.activityEntry(entryId)?.let { EntryLookup.Found(it) } ?: EntryLookup.Gone
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                EntryLookup.Failed(e.userMessage())
            }
        }
    }
    return listed?.let { EntryLookup.Found(it) } ?: read
}
