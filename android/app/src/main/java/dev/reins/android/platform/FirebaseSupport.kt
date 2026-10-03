package dev.reins.android.platform

import android.content.Context
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import dev.reins.android.BuildConfig

/** Firebase is optional: without `google-services.json` the app runs on the foreground long-poll alone. */
object FirebaseSupport {
    fun available(context: Context): Boolean =
        BuildConfig.HAS_FIREBASE && FirebaseApp.getApps(context).isNotEmpty()

    /** The current FCM registration token, or null when Firebase is absent or unreachable. */
    suspend fun token(context: Context): String? {
        if (!available(context)) return null
        return try {
            FirebaseMessaging.getInstance().token.await()
        } catch (e: Exception) {
            if (e is kotlin.coroutines.cancellation.CancellationException) throw e
            null
        }
    }
}
