package dev.reins.android.platform

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.reins.android.R
import dev.reins.android.feedback.FeedbackStore
import dev.reins.android.ui.activity.dueIn
import dev.reins.android.ui.activity.STATE_NEEDS_INFO
import dev.reins.android.ui.activity.reminderTimes
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.untrustedName
import dev.reins.core.PendingItem
import org.json.JSONArray
import org.json.JSONObject

/**
 * "Still waiting" reminders for tickets (Decisions v2, 03 §5.2): at 50 % and 90 % of each ticket's window. Like
 * [GrantReminders] the plan is kept in sealed preferences so alarms survive a reboot, and rebuilt whenever the waiting
 * list is read: a ticket answered (here or anywhere) or gone takes its alarms with it. A reminder is for something to
 * act on: it opens the request, and a routine one can be denied from it.
 */
object TicketReminders {
    private const val PREFS = "ticket_reminders"
    private const val PLAN = "plan"
    private const val DONE = "done"
    private const val ACTION_DUE = "dev.reins.android.TICKET_REMINDER_DUE"
    private const val EXTRA_ID = "ticket_item"
    private const val EXTRA_STAGE = "stage"

    /** What one reminder needs without asking the core: the request, which stage, and its words. */
    data class Reminder(
        val id: String,
        /** 50 or 90 (% of the window). */
        val stage: Int,
        val fireAt: Long,
        val deadline: Long,
        val title: String,
        val who: String,
        val quick: Boolean,
    ) {
        /** Per deadline: a ticket given a new window reminds again. */
        val key get() = "$id:$deadline:$stage"
    }

    /**
     * The reminders the waiting [items] call for: each ticket still open, and not waiting for the agent, gets its 50 %
     * and 90 % reminders; of those already due only the latest stays (it fires at once), so a phone that was off never
     * sends two in a row. What Overnight parked reminds only after the morning: its window starts there (01 §11).
     */
    fun plan(items: List<PendingItem>, nowSeconds: Long): List<Reminder> = items.flatMap { item ->
        val t = item.ticket ?: return@flatMap emptyList()
        if (t.deadline <= nowSeconds) return@flatMap emptyList()
        // Waiting for the agent's answer to "Why?", not for the user: no reminder (its alarms go at the next sync).
        if (t.state == STATE_NEEDS_INFO) return@flatMap emptyList()
        val title = askSentence(item).text
        val all = reminderTimes(t, item.parkedUntil).zip(STAGES) { at, stage -> Reminder(item.id, stage, at, t.deadline, title, item.connectionLabel, item.quick) }
        val due = all.lastOrNull { it.fireAt <= nowSeconds }
        all.filter { it.fireAt > nowSeconds || it == due }
    }

    private val STAGES = listOf(50, 90)

    /** Aligns the alarms with the waiting [items]. */
    fun sync(context: Context, items: List<PendingItem>, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val wanted = plan(items, nowSeconds)
        val prefs = SealedPreferences(context, PREFS)
        val before = read(prefs.getString(PLAN, null))
        before.filter { old -> wanted.none { it.key == old.key } }.forEach { cancel(context, it) }
        val done = prefs.getStringSet(DONE, emptySet()).orEmpty().filter { key -> wanted.any { it.key == key } }.toSet()
        prefs.edit().putString(PLAN, write(wanted)).putStringSet(DONE, done).apply()
        wanted.filter { it.key !in done }.forEach { schedule(context, it, nowSeconds) }
    }

    /** After a reboot: puts the alarms back from the saved plan. */
    fun rearm(context: Context, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val prefs = SealedPreferences(context, PREFS)
        val done = prefs.getStringSet(DONE, emptySet()).orEmpty()
        read(prefs.getString(PLAN, null)).filter { it.deadline > nowSeconds && it.key !in done }.forEach { schedule(context, it, nowSeconds) }
    }

    /** The alarm for [id]'s [stage] went off. */
    fun due(context: Context, id: String, stage: Int, nowSeconds: Long = System.currentTimeMillis() / 1000) {
        val prefs = SealedPreferences(context, PREFS)
        val reminder = read(prefs.getString(PLAN, null)).firstOrNull { it.id == id && it.stage == stage } ?: return
        val done = prefs.getStringSet(DONE, emptySet()).orEmpty()
        if (reminder.key in done || reminder.deadline <= nowSeconds) return
        prefs.edit().putStringSet(DONE, done + reminder.key).apply()
        // Someone looking at the app sees it under Waiting for you already.
        if (Foreground.focused) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) return
        notify(context, manager, reminder, nowSeconds)
    }

    private fun notify(context: Context, manager: NotificationManager, r: Reminder, nowSeconds: Long) {
        // Quiet while there is time; under an hour left it sounds like a request.
        val kind = if (r.deadline - nowSeconds < 3_600) AppNotifier.Kind.Approvals else AppNotifier.Kind.Waiting
        val channel = AppNotifier.channel(context, kind, FeedbackStore(context).current, profile = dev.reins.android.profiles.WorkProfiles.PERSONAL)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Reins")
            .setContentText("Something is still waiting for you")
            .build()
        val who = untrustedName(r.who)
        val open = AppNotifier.openRequest(context, r.id)
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Still waiting: ${r.title}")
            .setContentText(listOf(who, dueIn(r.deadline, nowSeconds)).filter { it.isNotEmpty() }.joinToString(" · "))
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_REMINDER)
            .setAutoCancel(true)
            .setTimeoutAfter(((r.deadline - nowSeconds) * 1000).coerceAtLeast(1))
            .setContentIntent(open)
            .apply {
                if (r.quick) addAction(Notification.Action.Builder(null, "Deny", AppNotifier.denyRequest(context, r.id)).build())
                addAction(Notification.Action.Builder(null, "Review", open).build())
            }
            // The request's own notification id: answering it anywhere takes the reminder away too.
            .addExtras(android.os.Bundle().apply { putString(AppNotifier.EXTRA_ITEM, r.id) })
            .build()
        manager.notify(AppNotifier.itemNotificationId(r.id), notification)
    }

    private fun schedule(context: Context, r: Reminder, nowSeconds: Long) {
        val alarms = context.getSystemService(AlarmManager::class.java)
        val at = maxOf(r.fireAt, nowSeconds + 1) * 1000
        alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, broadcast(context, r.id, r.stage))
    }

    private fun cancel(context: Context, r: Reminder) {
        context.getSystemService(AlarmManager::class.java).cancel(broadcast(context, r.id, r.stage))
    }

    private fun broadcast(context: Context, id: String, stage: Int): PendingIntent = PendingIntent.getBroadcast(
        context,
        "$id:$stage".hashCode() and 0x7FFFFFFF or 0x400,
        Intent(context, Receiver::class.java).setAction(ACTION_DUE).putExtra(EXTRA_ID, id).putExtra(EXTRA_STAGE, stage),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun write(list: List<Reminder>): String = JSONArray().apply {
        list.forEach {
            put(
                JSONObject().put("id", it.id).put("stage", it.stage).put("fire", it.fireAt).put("end", it.deadline)
                    .put("title", it.title).put("who", it.who).put("quick", it.quick),
            )
        }
    }.toString()

    private fun read(json: String?): List<Reminder> = runCatching {
        val array = JSONArray(json ?: "[]")
        List(array.length()) { i ->
            val o = array.getJSONObject(i)
            Reminder(o.getString("id"), o.getInt("stage"), o.getLong("fire"), o.getLong("end"), o.getString("title"), o.getString("who"), o.getBoolean("quick"))
        }
    }.getOrDefault(emptyList())

    /** Alarm and boot events. Not exported: only the system and this app can reach it. */
    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                ACTION_DUE -> intent.getStringExtra(EXTRA_ID)?.let { due(context, it, intent.getIntExtra(EXTRA_STAGE, 0)) }
                Intent.ACTION_BOOT_COMPLETED -> rearm(context)
            }
        }
    }
}
