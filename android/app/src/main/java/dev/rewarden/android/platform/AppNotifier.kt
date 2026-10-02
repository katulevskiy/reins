package dev.rewarden.android.platform

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.net.Uri
import dev.rewarden.android.MainActivity
import dev.rewarden.android.BuildConfig
import dev.rewarden.android.R
import dev.rewarden.android.feedback.CueCategory
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.Feedback
import dev.rewarden.android.feedback.FeedbackSettings
import dev.rewarden.android.feedback.NoFeedback
import dev.rewarden.android.feedback.play
import dev.rewarden.android.autopilot.AutopilotText
import dev.rewarden.android.ui.common.fullTitle
import dev.rewarden.android.ui.common.untrusted
import dev.rewarden.core.AutoDecisionView
import dev.rewarden.core.AutopilotEvent
import dev.rewarden.core.Notifier
import dev.rewarden.core.PendingItem
import dev.rewarden.core.PendingKind
import dev.rewarden.core.Verdict

/**
 * Local notifications for items that wait for the user. The core calls this from its own threads, so everything
 * here is quick and thread-safe. Lock-screen text is content-free (VISIBILITY_PRIVATE with a public version).
 *
 * Notifications sound the chimes the app plays in front (`fx_chime_request` for what waits, `fx_chime_attention` for
 * alerts), with vibrations that match the in-app haptics, and follow the in-app Sounds & haptics switches: one channel
 * per kind and per sound / vibration combination those ask for, created lazily. Channel sounds are immutable once a
 * channel exists, so the ids carry [CHANNEL_VERSION] and [migrateChannels] deletes channels of other versions.
 */
class AppNotifier(
    private val context: Context,
    private val settings: () -> FeedbackSettings = { FeedbackSettings() },
    private val feedback: () -> Feedback = { NoFeedback },
    /** Autopilot's state changed (a mode, a bypass ending, a pause): the app re-reads it. */
    private val onAutopilot: (AutopilotEvent) -> Unit = {},
    private val onChanged: () -> Unit,
) : Notifier {
    private val manager = context.getSystemService(NotificationManager::class.java)

    /** Notification kinds: wording, importance, chime, and a vibration (off / on pairs in ms) like the in-app haptic. */
    enum class Kind(
        val key: String,
        val label: String,
        val description: String,
        val importance: Int,
        /** The chime in `res/raw`; null: the channel never sounds. */
        val sound: String?,
        val category: CueCategory,
        /** null: the channel never vibrates. */
        val pattern: LongArray?,
        val private: Boolean,
        /** The settings group the channel is listed under. */
        val group: String? = null,
    ) {
        Approvals(
            "approvals", "Approvals", "Requests from your AI clients that wait for your approval", NotificationManager.IMPORTANCE_HIGH,
            "fx_chime_request", CueCategory.Requests, longArrayOf(0, 18, 90, 18), private = true,
        ),
        Grants(
            "grants", "Grants ending", "A reminder shortly before a grant runs out", NotificationManager.IMPORTANCE_DEFAULT,
            "fx_chime_attention", CueCategory.Alerts, longArrayOf(0, 35, 55, 45), private = true,
        ),
        Status(
            "status", "Status", "Changes to this phone's role, Autopilot pausing itself", NotificationManager.IMPORTANCE_DEFAULT,
            "fx_chime_attention", CueCategory.Alerts, longArrayOf(0, 35, 55, 45), private = false,
        ),

        /** What Autopilot approved by itself: listed, never heard. */
        AutoApproved(
            "autopilot-approved", "Approved by Autopilot", "Requests Autopilot or a bypass approved for you", NotificationManager.IMPORTANCE_LOW,
            null, CueCategory.Autopilot, null, private = true, group = GROUP_AUTOPILOT,
        ),

        /** What Autopilot (or Lockdown) denied by itself: a soft click, a short tick. */
        AutoDenied(
            "autopilot-denied", "Denied by Autopilot", "Requests Autopilot or Lockdown denied for you", NotificationManager.IMPORTANCE_DEFAULT,
            "fx_close", CueCategory.Autopilot, longArrayOf(0, 12), private = true, group = GROUP_AUTOPILOT,
        ),

        /** A bypass that runs: there to be seen, never heard. */
        Bypass(
            "autopilot-bypass", "Bypass", "Shown while a bypass runs, with Stop", NotificationManager.IMPORTANCE_LOW,
            null, CueCategory.Autopilot, null, private = false, group = GROUP_AUTOPILOT,
        ),

        /** The model coming down. */
        ModelDownload(
            "autopilot-model", "Model download", "Progress of downloading Autopilot's model", NotificationManager.IMPORTANCE_LOW,
            null, CueCategory.Autopilot, null, private = false, group = GROUP_AUTOPILOT,
        ),
    }

    fun createChannels() {
        migrateChannels(manager)
        Kind.entries.forEach { channel(it) }
        if (BuildConfig.SELF_UPDATE) dev.rewarden.android.platform.update.UpdateNotifier.createChannel(manager)
    }

    /** The channel for [kind] as the in-app switches stand now; [silent] forces the quiet one. */
    fun channel(kind: Kind, silent: Boolean = false): String = channel(context, kind, settings(), silent)

    override fun itemPending(item: PendingItem) {
        onChanged()
        // Someone looking at the app already sees the request (it pops up and chimes); a notification would only repeat it.
        if (Foreground.focused) {
            feedback().play(Event.RequestArrived)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val channel = channel(Kind.Approvals)
        val title = when (item.kind) {
            PendingKind.REQUEST ->
                if (item.action == "grant") "Permission requested" else "Approval needed"
            PendingKind.PAIRING -> "Connect an AI"
            PendingKind.BLOB -> "File to check"
        }
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Rewarden")
            .setContentText("Something is waiting for you")
            .build()
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(
                if (item.kind == PendingKind.PAIRING) {
                    untrusted(item.title)
                } else {
                    fullTitle(item.connectionLabel, item.action, item.count.toInt(), item.service, item.opTitle, item.op)
                },
            )
            .setSubText(untrusted(item.subtitle).ifBlank { null })
            .apply {
                // Assisted: what Autopilot would do, under the request.
                item.suggestion?.let { line ->
                    val text = fullTitle(item.connectionLabel, item.action, item.count.toInt(), item.service, item.opTitle, item.op)
                    setStyle(Notification.BigTextStyle().bigText("$text\n$line"))
                }
            }
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setAutoCancel(true)
            .setContentIntent(openItem(item))
            .setWhen(item.createdAt * 1000)
            .build()
        manager.notify(notificationId(item.id), notification)
    }

    override fun autoDecided(decision: AutoDecisionView) {
        onChanged()
        val approved = decision.verdict == Verdict.APPROVE
        // In front the list changes before the user's eyes; a quiet sound says why.
        if (Foreground.focused) {
            feedback().play(if (approved) Event.AutoApproved else Event.AutoDenied)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val channel = channel(if (approved) Kind.AutoApproved else Kind.AutoDenied)
        val title = AutopilotText.decisionTitle(decision)
        val text = AutopilotText.decisionText(decision)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText("Open Rewarden to see what it was")
            .build()
        val open = decision.activityId?.let { openActivity(it) } ?: openApp()
        val builder = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(text)
            .setSubText(AutopilotText.decisionDetail(decision))
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_STATUS)
            .setGroup(GROUP_AUTOPILOT)
            .setAutoCancel(true)
            .setContentIntent(open)
        // "Report" opens the entry, where "This was wrong" teaches Autopilot (an approval cannot be taken back).
        decision.activityId?.let { builder.addAction(Notification.Action.Builder(null, "Report", openActivity(it)).build()) }
        manager.notify(notificationId("auto:" + decision.requestId), builder.build())
        summarize(title, text)
    }

    override fun autopilotChanged(event: AutopilotEvent) {
        onAutopilot(event)
        when (event) {
            is AutopilotEvent.BypassEnded -> if (Foreground.focused) feedback().play(Event.BypassOff)
            is AutopilotEvent.Paused -> paused(event)
            is AutopilotEvent.ModeChanged -> Unit
        }
    }

    /** The automatic decisions of a while, folded into one group with a summary. */
    private val recent = ArrayDeque<String>()

    private fun summarize(title: String, text: String) {
        val lines = synchronized(recent) {
            recent.addFirst("$title: $text")
            while (recent.size > SUMMARY_LINES) recent.removeLast()
            recent.toList()
        }
        val style = Notification.InboxStyle()
        lines.forEach { style.addLine(it) }
        val summary = Notification.Builder(context, channel(Kind.AutoApproved))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Autopilot")
            .setContentText(if (lines.size == 1) "1 request decided" else "${lines.size} requests decided")
            .setStyle(style)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setGroup(GROUP_AUTOPILOT)
            .setGroupSummary(true)
            .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(AUTOPILOT_SUMMARY_ID, summary)
    }

    /** Auto-approvals stopped for a connection (unusual volume); its requests wait for the user. */
    private fun paused(event: AutopilotEvent.Paused) {
        val inFront = Foreground.focused
        if (inFront) feedback().play(Event.Alert)
        if (!manager.areNotificationsEnabled()) return
        val reason = untrusted(event.reason).trim().trimEnd('.').ifEmpty { "unusual volume" }
        val text = "Its requests wait for you again: $reason."
        val notification = Notification.Builder(context, channel(Kind.Status, silent = inFront))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Autopilot paused for ${untrusted(event.connectionLabel)}")
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText("$text Autopilot carries on by itself once things calm down."))
            .setAutoCancel(true)
            .setContentIntent(openAutopilot())
            .build()
        manager.notify(notificationId("paused:" + event.connectionId), notification)
    }

    /**
     * The ongoing "Bypass on" notification while [notice] says a bypass runs, with a live countdown and Stop; null
     * takes it away. It also times itself out at the end (the core ends the bypass by itself either way).
     */
    fun showBypass(notice: AutopilotText.BypassNotice?, nowMillis: Long = System.currentTimeMillis()) {
        if (notice == null || notice.until * 1000 <= nowMillis) {
            manager.cancel(BYPASS_ID)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val stop = PendingIntent.getBroadcast(
            context,
            BYPASS_ID,
            Intent(context, BypassStopReceiver::class.java).setAction(BypassStopReceiver.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, channel(Kind.Bypass))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(notice.title)
            .setContentText(notice.text)
            .setStyle(Notification.BigTextStyle().bigText(notice.text))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(true)
            .setWhen(notice.until * 1000)
            .setUsesChronometer(true)
            .setChronometerCountDown(true)
            .setTimeoutAfter(notice.until * 1000 - nowMillis)
            .setColor(BYPASS_RED)
            .setCategory(Notification.CATEGORY_STATUS)
            .setContentIntent(openAutopilot())
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
        manager.notify(BYPASS_ID, notification)
    }

    /** The model download as a progress bar (the foreground notification of its job). */
    fun downloadNotification(downloaded: Long, total: Long): Notification {
        val builder = Notification.Builder(context, channel(Kind.ModelDownload))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Downloading Autopilot's model")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_PROGRESS)
            .setContentIntent(openAutopilot())
        if (total > 0) {
            builder.setContentText("${AutopilotText.mb(downloaded.toULong())} of ${AutopilotText.mb(total.toULong())}")
            builder.setProgress(1000, (downloaded * 1000 / total).toInt().coerceIn(0, 1000), false)
        } else {
            builder.setContentText(if (downloaded > 0) AutopilotText.mb(downloaded.toULong()) else "Starting")
            builder.setProgress(0, 0, true)
        }
        return builder.build()
    }

    /** How the download ended, when the user is not looking at the app. */
    fun downloadFinished(failure: String?) {
        if (Foreground.focused || !manager.areNotificationsEnabled()) return
        val notification = Notification.Builder(context, channel(Kind.ModelDownload))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(if (failure == null) "Autopilot's model is ready" else "The model could not be downloaded")
            .setContentText(failure ?: "It runs on this phone; nothing leaves it.")
            .setAutoCancel(true)
            .setContentIntent(openAutopilot())
            .build()
        manager.notify(DOWNLOAD_DONE_ID, notification)
    }

    override fun itemResolved(id: String) {
        manager.cancel(notificationId(id))
        onChanged()
    }

    /** "This phone is no longer your approval device." Posted in front too (it explains itself), but silently there. */
    fun deviceReplaced() {
        onChanged()
        val inFront = Foreground.focused
        if (inFront) feedback().play(Event.Alert)
        if (!manager.areNotificationsEnabled()) return
        val notification = Notification.Builder(context, channel(Kind.Status, silent = inFront))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("This phone is no longer your approval device")
            .setContentText("Another phone took over. Open Rewarden to register this one again.")
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(REPLACED_ID, notification)
    }

    private fun openItem(item: PendingItem): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_ITEM)
            .putExtra(
                EXTRA_KIND,
                when (item.kind) {
                    PendingKind.REQUEST -> KIND_REQUEST
                    PendingKind.PAIRING -> KIND_PAIRING
                    PendingKind.BLOB -> KIND_BLOB
                },
            )
            .putExtra(EXTRA_ID, item.id)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        return PendingIntent.getActivity(
            context,
            notificationId(item.id),
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    /** Opens one activity entry (the "Report" of an automatic decision). */
    private fun openActivity(activityId: Long): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_ACTIVITY)
            .putExtra(EXTRA_ACTIVITY_ID, activityId)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val code = (activityId and 0x3FFF_FFFF).toInt() or 0x4000_0000
        return PendingIntent.getActivity(context, code, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** Opens Settings > Autopilot (the bypass, the download). */
    private fun openAutopilot(): PendingIntent = PendingIntent.getActivity(
        context,
        BYPASS_ID,
        Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_AUTOPILOT)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun notificationId(id: String): Int = id.hashCode() and 0x7FFFFFFF or 0x100

    companion object {
        /** Bumped whenever a channel's sound or vibration changes (they cannot be edited once created). */
        const val CHANNEL_VERSION = 2

        /** The unversioned channels of builds without chimes. */
        private val LEGACY_CHANNELS = setOf("approvals", "grants", "status")

        const val ACTION_OPEN_ITEM = "dev.rewarden.android.OPEN_ITEM"
        const val EXTRA_KIND = "kind"
        const val EXTRA_ID = "id"
        const val KIND_REQUEST = "request"
        const val KIND_PAIRING = "pairing"
        const val KIND_BLOB = "blob"
        const val ACTION_OPEN_ACTIVITY = "dev.rewarden.android.OPEN_ACTIVITY"
        const val EXTRA_ACTIVITY_ID = "activity"
        const val ACTION_OPEN_AUTOPILOT = "dev.rewarden.android.OPEN_AUTOPILOT"
        private const val REPLACED_ID = 1
        const val AUTOPILOT_SUMMARY_ID = 2
        const val BYPASS_ID = 3
        const val DOWNLOAD_ID = 4
        private const val DOWNLOAD_DONE_ID = 5
        private const val SUMMARY_LINES = 6

        /** The settings group of Autopilot's channels, and the group its notifications fold into. */
        const val GROUP_AUTOPILOT = "autopilot"

        /** The bypass notification's accent: Rewarden's danger red. */
        private const val BYPASS_RED = 0xFFDC2626.toInt()

        /** A channel id, e.g. `approvals-v2-sv` (sound and vibration), `-s`, `-v` or `-q` (quiet). */
        fun channelId(kind: Kind, sound: Boolean, vibrate: Boolean): String =
            "${kind.key}-v$CHANNEL_VERSION-" + (if (sound) "s" else "") + (if (vibrate) "v" else "") + (if (!sound && !vibrate) "q" else "")

        /** Creates (once) and returns the channel [settings] ask for: the kind's chime under its category switch, vibration under Haptics. */
        fun channel(context: Context, kind: Kind, settings: FeedbackSettings, silent: Boolean = false): String {
            val sound = !silent && kind.sound != null && settings.allows(kind.category)
            val vibrate = !silent && kind.pattern != null && settings.hapticsOn
            val id = channelId(kind, sound, vibrate)
            val manager = context.getSystemService(NotificationManager::class.java)
            if (manager.getNotificationChannel(id) != null) return id
            kind.group?.let { group ->
                if (manager.getNotificationChannelGroup(group) == null) {
                    manager.createNotificationChannelGroup(android.app.NotificationChannelGroup(group, "Autopilot"))
                }
            }
            val suffix = when {
                // A channel that never sounds has only the one variant.
                kind.sound == null && kind.pattern == null -> ""
                sound && vibrate -> ""
                sound -> " (sound only)"
                vibrate -> " (vibration only)"
                else -> " (silent)"
            }
            val channel = NotificationChannel(id, kind.label + suffix, kind.importance)
            channel.description = kind.description
            kind.group?.let { channel.group = it }
            if (kind.private) channel.lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            if (sound) {
                channel.setSound(
                    Uri.parse("android.resource://${context.packageName}/raw/${kind.sound}"),
                    AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION).setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build(),
                )
            } else {
                channel.setSound(null, null)
            }
            channel.enableVibration(vibrate)
            if (vibrate) channel.vibrationPattern = kind.pattern
            manager.createNotificationChannel(channel)
            return id
        }

        /** Removes the channels of other versions (their sounds cannot be edited in place) and the unversioned ones. */
        fun migrateChannels(manager: NotificationManager) {
            val current = "-v$CHANNEL_VERSION-"
            manager.notificationChannels
                .filter { ch -> ch.id in LEGACY_CHANNELS || Kind.entries.any { ch.id.startsWith(it.key + "-v") } && !ch.id.contains(current) }
                .forEach { manager.deleteNotificationChannel(it.id) }
        }
    }
}
