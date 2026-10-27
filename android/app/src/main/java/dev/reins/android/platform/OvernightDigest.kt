package dev.reins.android.platform

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.reins.android.MainActivity
import dev.reins.android.R
import dev.reins.android.feedback.FeedbackStore
import dev.reins.android.ui.common.untrusted

/**
 * The morning summary of what Overnight parked (Decisions v2, 01 §11): one local notification at the morning time,
 * "3 waiting" with who from, and only when something waits. Parking itself never notifies.
 *
 * The core says what and when ([dev.reins.core.ReinsCoreInterface.overnightDigest]); this keeps ONE alarm for it,
 * replaced on every change of what waits and cancelled when nothing does or the morning came (the app then shows it
 * when opened). The plan is kept in sealed preferences so the alarm survives a reboot, like [TicketReminders].
 */
object OvernightDigest {
    private const val PREFS = "overnight_digest"
    private const val AT = "at"
    private const val TITLE = "title"
    private const val BODY = "body"
    private const val ACTION_DUE = "dev.reins.android.OVERNIGHT_DIGEST_DUE"

    /** Its own notification id: a new summary replaces the last one. */
    const val NOTIFICATION_ID = 8

    /** What the alarm shows, without asking the core when it fires. */
    data class Plan(val at: Long, val title: String, val body: String)

    /** What [digest] calls for: a summary at its time, or none (nothing waits, or the morning came already). */
    fun plan(digest: dev.reins.core.OvernightDigest?): Plan? {
        val at = digest?.at ?: return null
        if (digest.total == 0u) return null
        return Plan(at, digest.title, digest.body)
    }

    /** Aligns the one alarm with [digest]: scheduled at its `at` (replacing any earlier one), else cancelled. */
    fun sync(context: Context, digest: dev.reins.core.OvernightDigest?, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val prefs = SealedPreferences(context, PREFS)
        val plan = plan(digest)
        if (plan == null) {
            prefs.edit().clear().apply()
            context.getSystemService(AlarmManager::class.java).cancel(broadcast(context))
            // Nothing waits any more: a summary still shown would point at nothing.
            if (digest == null) context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
            return
        }
        prefs.edit().putLong(AT, plan.at).putString(TITLE, plan.title).putString(BODY, plan.body).apply()
        schedule(context, plan, nowSeconds)
    }

    /** After a reboot: puts the alarm back from the saved plan. */
    fun rearm(context: Context, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        read(SealedPreferences(context, PREFS))?.let { schedule(context, it, nowSeconds) }
    }

    /** The alarm went off: the summary, once. */
    fun due(context: Context) {
        val prefs = SealedPreferences(context, PREFS)
        val plan = read(prefs) ?: return
        prefs.edit().clear().apply()
        // Someone looking at the app sees what waits under Waiting for you already.
        if (Foreground.focused) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) return
        val channel = AppNotifier.channel(context, AppNotifier.Kind.Waiting, FeedbackStore(context).current, profile = dev.reins.android.profiles.WorkProfiles.PERSONAL)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Reins")
            .setContentText("Something is waiting for you")
            .build()
        val body = untrusted(plan.body)
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(untrusted(plan.title))
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_REMINDER)
            .setAutoCancel(true)
            .setContentIntent(open(context))
            .build()
        manager.notify(NOTIFICATION_ID, notification)
    }

    private fun read(prefs: SealedPreferences): Plan? {
        val at = prefs.getLong(AT, 0)
        if (at == 0L) return null
        return Plan(at, prefs.getString(TITLE, null) ?: return null, prefs.getString(BODY, null).orEmpty())
    }

    private fun schedule(context: Context, plan: Plan, nowSeconds: Long) {
        val at = maxOf(plan.at, nowSeconds + 1) * 1000
        // One request code: a new time replaces the old alarm.
        context.getSystemService(AlarmManager::class.java).setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, broadcast(context))
    }

    private fun broadcast(context: Context): PendingIntent = PendingIntent.getBroadcast(
        context,
        NOTIFICATION_ID or 0x800,
        Intent(context, Receiver::class.java).setAction(ACTION_DUE),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** Opens Activity, where Waiting for you lists what waits by agent and task. */
    private fun open(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        NOTIFICATION_ID,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** Alarm and boot events. Not exported: only the system and this app can reach it. */
    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                ACTION_DUE -> due(context)
                Intent.ACTION_BOOT_COMPLETED -> rearm(context)
            }
        }
    }
}
