package dev.reins.android.ui.approval

import dev.reins.core.ApprovalView
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.ReinsCoreInterface
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.cancellation.CancellationException

/**
 * Approval sheets read ahead for the requests that wait, so that a sheet (from the list or a notification) opens with
 * its content instead of a spinner. The sheet reads the request again as it opens; this is only the first frame.
 */
class ApprovalViews {
    private val views = ConcurrentHashMap<String, ApprovalView>()

    operator fun get(id: String): ApprovalView? = views[id]

    /** Keeps the views of what still waits and reads the missing ones; a failure only means a spinner later. */
    suspend fun prefetch(core: ReinsCoreInterface, pending: List<PendingItem>) {
        val ids = pending.filter { it.kind == PendingKind.REQUEST }.map { it.id }.toSet()
        views.keys.retainAll(ids)
        for (id in ids - views.keys) {
            try {
                views[id] = core.approvalView(id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                continue
            }
        }
    }
}
