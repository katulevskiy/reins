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
import dev.reins.android.design.compactDuration
import dev.reins.android.design.reminderAt
import dev.reins.android.feedback.FeedbackStore
import dev.reins.android.ui.common.untrusted
import dev.reins.core.GrantView
import org.json.JSONArray
import org.json.JSONObject

/**
 * "This grant is about to end" reminders. One alarm per running grant that ends by itself, due shortly before it does
 * (see [dev.reins.android.design.expiryLeadSeconds]). The plan is kept in preferences so alarms survive a reboot,
 * and is rebuilt whenever the grants are read, so resumed, deleted or used-up grants never remind.
 *
 * The alarm is inexact and needs no permission; a reminder may arrive a few minutes late, never early.
 */
object GrantReminders {
    private const val PREFS = "grant_reminders"
    private const val PLAN = "plan"
    private const val DONE = "done"
    private const val ACTION_DUE = "dev.reins.android.GRANT_REMINDER_DUE"
    const val ACTION_OPEN_GRANT = "dev.reins.android.OPEN_GRANT"
    const val EXTRA_GRANT_ID = "grant_id"

    /** What one reminder needs to be shown without asking the core (which may not be open). */
    data class Reminder(val id: String, val fireAt: Long, val expiresAt: Long, val label: String, val summary: String) {
        /** Reminders are per end time: resuming a grant makes a new one. */
        val key get() = "$id:$expiresAt"
    }

    /** The reminders the running [grants] call for. */
    fun plan(grants: List<GrantView>): List<Reminder> = grants.filter { it.active }.mapNotNull { g ->
        val fireAt = reminderAt(g) ?: return@mapNotNull null
        Reminder(g.id, fireAt, g.expiresAt!!, g.connectionLabel, g.summary)
    }

    /** Aligns the alarms with [grants]. */
    fun sync(context: Context, grants: List<GrantView>, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val wanted = plan(grants).filter { it.expiresAt > nowSeconds }
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val before = read(prefs.getString(PLAN, null))
        before.filter { old -> wanted.none { it.key == old.key } }.forEach { cancel(context, it.id) }
        val done = prefs.getStringSet(DONE, emptySet()).orEmpty().filter { key -> wanted.any { it.key == key } }.toSet()
        prefs.edit().putString(PLAN, write(wanted)).putStringSet(DONE, done).apply()
        wanted.filter { it.key !in done }.forEach { schedule(context, it, nowSeconds) }
    }

    /** After a reboot: puts the alarms back from the saved plan. */
    fun rearm(context: Context, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val done = prefs.getStringSet(DONE, emptySet()).orEmpty()
        read(prefs.getString(PLAN, null)).filter { it.expiresAt > nowSeconds && it.key !in done }.forEach { schedule(context, it, nowSeconds) }
    }

    /** The alarm for [id] went off. */
    fun due(context: Context, id: String, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val reminder = read(prefs.getString(PLAN, null)).firstOrNull { it.id == id } ?: return
        val done = prefs.getStringSet(DONE, emptySet()).orEmpty()
        if (reminder.key in done || reminder.expiresAt <= nowSeconds) return
        prefs.edit().putStringSet(DONE, done + reminder.key).apply()
        // Someone looking at the app sees the "Ends soon" mark on the grant already.
        if (Foreground.focused) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) return
        notify(context, manager, reminder, nowSeconds)
    }

    private fun notify(context: Context, manager: NotificationManager, r: Reminder, nowSeconds: Long) {
        val left = compactDuration(r.expiresAt - nowSeconds)
        val channel = AppNotifier.channel(context, AppNotifier.Kind.Grants, FeedbackStore(context).current)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Reins")
            .setContentText("A grant ends soon")
            .build()
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Access ends in $left")
            .setContentText("${untrusted(r.label)}: ${untrusted(r.summary)}")
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_REMINDER)
            .setAutoCancel(true)
            .setTimeoutAfter(((r.expiresAt - nowSeconds) * 1000).coerceAtLeast(1))
            .setContentIntent(open(context, r.id))
            .build()
        manager.notify(notificationId(r.id), notification)
    }

    private fun open(context: Context, id: String): PendingIntent = PendingIntent.getActivity(
        context,
        notificationId(id),
        Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_GRANT)
            .putExtra(EXTRA_GRANT_ID, id)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun schedule(context: Context, r: Reminder, nowSeconds: Long) {
        val alarms = context.getSystemService(AlarmManager::class.java)
        // A grant already inside its last stretch reminds at once.
        val at = maxOf(r.fireAt, nowSeconds + 1) * 1000
        alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, broadcast(context, r.id))
    }

    private fun cancel(context: Context, id: String) {
        context.getSystemService(AlarmManager::class.java).cancel(broadcast(context, id))
    }

    private fun broadcast(context: Context, id: String): PendingIntent = PendingIntent.getBroadcast(
        context,
        notificationId(id),
        Intent(context, Receiver::class.java).setAction(ACTION_DUE).putExtra(EXTRA_GRANT_ID, id),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun notificationId(id: String): Int = id.hashCode() and 0x7FFFFFFF or 0x200

    private fun write(list: List<Reminder>): String = JSONArray().apply {
        list.forEach {
            put(JSONObject().put("id", it.id).put("fire", it.fireAt).put("end", it.expiresAt).put("label", it.label).put("summary", it.summary))
        }
    }.toString()

    private fun read(json: String?): List<Reminder> = runCatching {
        val array = JSONArray(json ?: "[]")
        List(array.length()) { i ->
            val o = array.getJSONObject(i)
            Reminder(o.getString("id"), o.getLong("fire"), o.getLong("end"), o.getString("label"), o.getString("summary"))
        }
    }.getOrDefault(emptyList())

    /** Alarm and boot events. Not exported: only the system and this app can reach it. */
    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                ACTION_DUE -> intent.getStringExtra(EXTRA_GRANT_ID)?.let { due(context, it) }
                Intent.ACTION_BOOT_COMPLETED -> rearm(context)
            }
        }
    }
}
