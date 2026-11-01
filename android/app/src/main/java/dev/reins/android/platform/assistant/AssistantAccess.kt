package dev.reins.android.platform.assistant

import android.content.Context
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.appfunctions.AppFunctionManager
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Turns the generated functions on or off together. They ship disabled; only this, from the Settings switch,
 * enables them. The ids are copied from the generated service so starting the app on Android 15 and earlier
 * never loads that class: its platform superclass exists only on API 36.
 */
object AssistantAccess {
    private val reconcileLock = Mutex()

    fun isSupported(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < 36) return false
        return managerOrNull(context) != null
    }

    /**
     * One reconcile at a time, always of the value stored now, not the value a click captured.
     * [read] runs inside the lock so a newer write that landed before this turn is what gets applied.
     */
    internal suspend fun reconcileLatest(read: () -> Boolean, applyState: suspend (Boolean) -> Unit) {
        reconcileLock.withLock { applyState(read()) }
    }

    /** Re-applies [profileId]'s switch, so a restart or a profile change covers an index that was not ready. */
    suspend fun sync(context: Context, profileId: String) {
        if (Build.VERSION.SDK_INT < 36) return
        reconcileLatest(read = { AssistantStore(context).enabled(profileId) }) { enabled ->
            applyOnApi36(context, enabled)
        }
    }

    /**
     * Separate from [sync] so Android 15 and earlier never resolves [AppFunctionManager] while starting.
     * Its platform types exist only on API 36.
     */
    @RequiresApi(36)
    private suspend fun applyOnApi36(context: Context, enabled: Boolean) {
        val manager = AppFunctionManager.getInstance(context) ?: return
        val state = if (enabled) {
            AppFunctionManager.APP_FUNCTION_STATE_ENABLED
        } else {
            AppFunctionManager.APP_FUNCTION_STATE_DISABLED
        }
        for (id in FUNCTION_IDS) {
            try {
                manager.setAppFunctionEnabled(id, state)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Indexing can lag behind install. The next start tries again.
            }
        }
    }

    @RequiresApi(36)
    private fun managerOrNull(context: Context): AppFunctionManager? = AppFunctionManager.getInstance(context)

    /**
     * `ReinsAppFunctionService`'s `FUNCTION_ID_*` values. Keep these identical to the generated constants.
     */
    internal val FUNCTION_IDS = listOf(
        "dev.reins.android.platform.assistant.ReinsAppFunctions#getStatus",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#listWaitingRequests",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#openRequest",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#denyRequest",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#denyAllWaiting",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#startLockdown",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#switchToManual",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#searchActivity",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#listStandingPermissions",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#revokeStandingPermission",
        "dev.reins.android.platform.assistant.ReinsAppFunctions#listConnectedAis",
    )
}
