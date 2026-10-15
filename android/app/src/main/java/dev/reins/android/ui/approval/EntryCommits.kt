package dev.reins.android.ui.approval

import androidx.compose.runtime.Stable
import androidx.compose.runtime.mutableStateMapOf
import dev.reins.core.GitCommitView
import dev.reins.core.GitFileView
import kotlin.coroutines.cancellation.CancellationException

/** Where one commit of an entry's push is, when its changes are loaded from the git host. */
sealed interface CommitLoad {
    data object Loading : CommitLoad

    data class Loaded(val commit: GitCommitView) : CommitLoad

    /** [reason]: the core's words, for the (?) beside "Couldn't load". */
    data class Failed(val reason: String) : CommitLoad
}

/**
 * The commits of an Activity entry's push whose changes were not kept on this phone: each can be loaded once from the
 * git host through the phone's own connection (the core's `activityCommitChange`, only commits of that push). Read-only:
 * loading asks nothing and decides nothing.
 */
@Stable
class EntryCommits(private val fetch: suspend (shortSha: String) -> GitCommitView) {
    val states = mutableStateMapOf<String, CommitLoad>()

    /** The commit as shown: the one loaded for [commit] when there is one, else [commit] itself. */
    fun shown(commit: GitCommitView): GitCommitView = (states[commit.shortSha] as? CommitLoad.Loaded)?.commit ?: commit

    suspend fun load(shortSha: String) {
        if (states[shortSha] == CommitLoad.Loading) return
        states[shortSha] = CommitLoad.Loading
        states[shortSha] = try {
            CommitLoad.Loaded(fetch(shortSha))
        } catch (e: CancellationException) {
            states.remove(shortSha)
            throw e
        } catch (e: Exception) {
            CommitLoad.Failed(e.message.orEmpty())
        }
    }
}

/**
 * The commit's changes line by line are not here: a file of text without its diff (left out of the synced copy, or
 * over the desktop app's budget), or its files not compared at all. A binary file, or one with no lines changed, has
 * none to show.
 */
fun changesMissing(commit: GitCommitView): Boolean = commit.filesChanged == null || commit.files.any { it.missesDiff() }

private fun GitFileView.missesDiff() = !binary && diff.isEmpty() && (additions ?: 1u) + (deletions ?: 1u) > 0u
