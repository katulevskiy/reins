package dev.reins.android.sync

import dev.reins.android.AppContainer
import dev.reins.android.state.SessionState
import dev.reins.core.CoreException
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.delay

/**
 * While the app is on screen, long-polls the server so requests show up within a second even without FCM.
 * Runs inside a lifecycle-bound coroutine; cancelling it ends the loop.
 */
class ForegroundSync(private val container: AppContainer, private val waitSecs: UInt = 25u) {
    suspend fun run() {
        val epoch = container.state.accountEpoch.value
        var failures = 0
        while (container.state.isCurrent(epoch)) {
            try {
                val items = container.core.sync(waitSecs)
                if (!container.state.isCurrent(epoch)) return
                container.state.setPending(items)
                // Requests that grants answered by themselves leave no prompt, only a new activity entry.
                container.refreshPending()
                failures = 0
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException) {
                if (!container.state.isCurrent(epoch)) return
                when {
                    e is CoreException.NotLoggedIn -> {
                        container.state.setSession(SessionState.SignedOut)
                        return
                    }
                    e is CoreException.Server && e.status.toInt() == 403 && container.state.approvalDevice.value -> {
                        container.markReplaced()
                        return
                    }
                    // Refused before this phone registered: right after a sign-in the first poll can run before the
                    // registration does. Ask again, instead of stopping until the app is opened again.
                    e is CoreException.Server && e.status.toInt() == 403 && container.state.deviceReplaced.value -> return
                    else -> {
                        delay(backoffMillis(failures))
                        failures++
                    }
                }
            }
        }
    }

    companion object {
        /** 1 s, 2 s, 4 s … capped at 30 s. */
        fun backoffMillis(failures: Int): Long = (1_000L shl failures.coerceIn(0, 5)).coerceAtMost(30_000L)
    }
}
