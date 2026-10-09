package dev.reins.android.push

import dev.reins.core.CoreException
import dev.reins.core.ReinsCoreInterface
import kotlin.coroutines.cancellation.CancellationException

enum class PushOutcome { DONE, RETRY }

/** What a push does, independent of WorkManager so it can be tested on the JVM. */
class PushHandler(
    private val core: ReinsCoreInterface,
    private val onReplaced: suspend () -> Unit,
) {
    suspend fun handle(payload: PushPayload): PushOutcome {
        if (payload.kind == "replaced") onReplaced()
        return try {
            core.handlePush(payload.kind, payload.id)
            PushOutcome.DONE
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            when (e) {
                // Transient: the network or the server had a hiccup. The foreground poll will also pick it up.
                is CoreException.Network -> PushOutcome.RETRY
                is CoreException.Server -> if (e.status.toInt() >= 500) PushOutcome.RETRY else PushOutcome.DONE
                else -> PushOutcome.DONE
            }
        }
    }

    companion object {
        /** The app's handler: a "replaced" push marks this phone as no longer approving and says so. */
        fun of(container: dev.reins.android.AppContainer) = PushHandler(container.core) {
            container.markReplaced()
            container.notifier.deviceReplaced()
        }
    }
}
