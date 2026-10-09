package dev.reins.android.push

import android.content.Context
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import dev.reins.android.ReinsApp
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull

/** Not exported. FCM is a hint that something waits on the server; the phone fetches the real data itself. */
class ReinsMessagingService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        val payload = PushPayload.parse(message.data) ?: return
        // Right away, while FCM keeps the process awake for this message; WorkManager only when that did not finish
        // (no network, a server hiccup): a job can wait for its turn, and once the expedited quota is spent, longer.
        if (!handleNow(applicationContext, payload)) {
            PushWorker.enqueue(applicationContext, payload, highPriority = message.priority == RemoteMessage.PRIORITY_HIGH)
        }
    }

    override fun onNewToken(token: String) {
        RegisterDeviceWorker.enqueue(applicationContext)
    }

    companion object {
        /** FCM keeps the process awake for about ten seconds after a high-priority message. */
        const val INLINE_BUDGET_MS = 8_000L

        /** Handles [payload] on the calling thread (FCM's, never the main one); false when it has to be retried. */
        fun handleNow(context: Context, payload: PushPayload): Boolean {
            val container = (context.applicationContext as ReinsApp).container
            val outcome = runBlocking { withTimeoutOrNull(INLINE_BUDGET_MS) { PushHandler.of(container).handle(payload) } }
            if (outcome != PushOutcome.DONE) return false
            container.refreshAfterAnswer()
            return true
        }
    }
}
