package dev.rewarden.android.platform.update

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import dev.rewarden.android.MainActivity
import dev.rewarden.android.R

/** "Rewarden 0.2.0 is ready to install": one notification per new version, on the "App updates" channel. */
class UpdateNotifier(context: Context) : ReleaseNotifier {
    private val context = context.applicationContext
    private val manager = this.context.getSystemService(NotificationManager::class.java)

    override fun announce(release: Release, ready: Boolean) {
        if (!manager.areNotificationsEnabled()) return
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(if (ready) "Rewarden ${release.versionName} is ready to install" else "Rewarden ${release.versionName} is available")
            .setContentText("Tap to update.")
            .setCategory(Notification.CATEGORY_STATUS)
            .setAutoCancel(true)
            .setContentIntent(open())
            .build()
        manager.notify(NOTIFICATION_ID, notification)
    }

    private fun open(): PendingIntent = PendingIntent.getActivity(
        context,
        NOTIFICATION_ID,
        Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_UPDATE)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    companion object {
        const val CHANNEL = "updates"
        const val ACTION_OPEN_UPDATE = "dev.rewarden.android.OPEN_UPDATE"
        private const val NOTIFICATION_ID = 2

        fun createChannel(manager: NotificationManager) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "App updates", NotificationManager.IMPORTANCE_DEFAULT).apply {
                    description = "A new version of Rewarden is ready to install"
                },
            )
        }
    }
}
