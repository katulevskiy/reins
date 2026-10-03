package dev.reins.android.push

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/** Not exported. FCM is a hint that something waits on the server; the phone fetches the real data itself. */
class ReinsMessagingService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        val payload = PushPayload.parse(message.data) ?: return
        PushWorker.enqueue(applicationContext, payload, highPriority = message.priority == RemoteMessage.PRIORITY_HIGH)
    }

    override fun onNewToken(token: String) {
        RegisterDeviceWorker.enqueue(applicationContext)
    }
}
