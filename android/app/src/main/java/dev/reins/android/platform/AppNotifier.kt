package dev.reins.android.platform

import dev.reins.android.profiles.WorkProfiles

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.net.Uri
import dev.reins.android.ui.activity.field
import dev.reins.android.ui.activity.pendingTone
import dev.reins.android.ui.activity.pressing
import androidx.compose.ui.graphics.toArgb
import dev.reins.android.MainActivity
import dev.reins.android.BuildConfig
import dev.reins.android.R
import dev.reins.android.feedback.CueCategory
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.Feedback
import dev.reins.android.feedback.FeedbackSettings
import dev.reins.android.feedback.NoFeedback
import dev.reins.android.feedback.play
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.ui.common.AskSentence
import dev.reins.android.ui.common.IntegrationBitmap
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.untrustedName
import dev.reins.android.ui.common.untrusted
import dev.reins.core.AutoDecisionView
import dev.reins.core.AutopilotEvent
import dev.reins.core.CircleEvent
import dev.reins.core.Notifier
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.Verdict

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
    /** The profile this notifier posts for, fixed when it's made: its channels, and the profile its intents name. */
    val profile: WorkProfiles.Active = WorkProfiles.PERSONAL,
    private val settings: () -> FeedbackSettings = { FeedbackSettings() },
    private val feedback: () -> Feedback = { NoFeedback },
    /** Autopilot's state changed (a mode, a bypass ending, a pause): the app re-reads it. */
    private val onAutopilot: (AutopilotEvent) -> Unit = {},
    /** A mode or Autopilot decided this request: it no longer waits, and an ask open on it closes. */
    private val onDecided: (String) -> Unit = {},
    /** Something about Circles changed (a review for this phone, a person, an owner's progress): read it again. */
    private val onCircles: () -> Unit = {},
    private val onChanged: () -> Unit,
) : Notifier {
    private val manager = context.getSystemService(NotificationManager::class.java)

    /** A work profile's notifications carry its id as their tag, so one profile never replaces or takes away
     *  another's; the Personal profile's are untagged, as before. */
    private val tag: String? = profile.id.takeIf { profile.managed }

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
        /** A string resource for the channel's name, which then wins over [label]. */
        val labelRes: Int? = null,
    ) {
        Approvals(
            "approvals", "Approvals", "Requests from your AI clients that wait for your approval", NotificationManager.IMPORTANCE_HIGH,
            "fx_chime_request", CueCategory.Requests, longArrayOf(0, 18, 90, 18), private = true,
        ),
        /**
         * A request that can wait (a ticket, Decisions v2): it arrives quietly, under Waiting for you, and its
         * reminders come here too while more than an hour is left (then they use [Approvals]).
         */
        Waiting(
            "waiting", "Waiting for you", "Requests that can wait for you, and reminders before they are due", NotificationManager.IMPORTANCE_DEFAULT,
            null, CueCategory.Requests, null, private = true,
        ),
        Grants(
            "grants", "Grants ending", "A reminder shortly before a grant runs out", NotificationManager.IMPORTANCE_DEFAULT,
            "fx_chime_attention", CueCategory.Alerts, longArrayOf(0, 35, 55, 45), private = true,
        ),
        Status(
            "status", "Status", "Changes to this phone's role, Autopilot pausing itself", NotificationManager.IMPORTANCE_DEFAULT,
            "fx_chime_attention", CueCategory.Alerts, longArrayOf(0, 35, 55, 45), private = false,
        ),

        /** A bypass that runs: there to be seen, never heard. */
        Bypass(
            "autopilot-bypass", "Bypass", "Shown while a bypass runs, with Stop", NotificationManager.IMPORTANCE_LOW,
            null, CueCategory.Autopilot, null, private = false, group = GROUP_AUTOPILOT,
        ),

        /** An Overnight that runs, like a bypass: seen, never heard. Named by `R.string.overnight_name`. */
        Overnight(
            "autopilot-overnight", AutopilotText.OVERNIGHT, "Shown while it runs, with Stop", NotificationManager.IMPORTANCE_LOW,
            null, CueCategory.Autopilot, null, private = false, group = GROUP_AUTOPILOT, labelRes = R.string.overnight_name,
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
        if (BuildConfig.SELF_UPDATE) dev.reins.android.platform.update.UpdateNotifier.createChannel(manager)
    }

    /** The channel for [kind] as the in-app switches stand now; [silent] forces the quiet one. */
    fun channel(kind: Kind, silent: Boolean = false): String = channel(context, kind, settings(), silent, profile)

    override fun itemPending(item: PendingItem) {
        onChanged()
        val nowSeconds = (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
        // Parked by Overnight for the morning (Decisions v2, 01 §11): parking never notifies, in front or not. The one
        // morning summary ([OvernightDigest]) covers it.
        // `parked`, not the morning time: an Overnight without an end parks until now.
        if (item.parked) return
        // A ticket can wait: it arrives quietly unless less than an hour is left (Decisions v2, 03 §5.2).
        val quiet = item.ticket?.let { !it.pressing(nowSeconds) } ?: false
        // Someone looking at the app already sees the request (it pops up); a notification would only repeat it. The
        // chime still sounds, on the notification stream, for a phone left open on the desk.
        if (Foreground.focused) {
            feedback().requestArrived(item.id, needsUser = !quiet)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val channel = channel(if (quiet) Kind.Waiting else Kind.Approvals)
        // The ask in the one formatter's words (ASK-TEXT.md): the sentence with its italic target as the title, who
        // asks beside the app's name, the machine part in mono under it, and the request's icon as the large icon.
        val ask = askSentence(item)
        val who = if (item.kind == PendingKind.REQUEST || item.kind == PendingKind.BLOB) untrustedName(item.connectionLabel).ifEmpty { null } else null
        val color = notificationColor(item)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Reins")
            .setContentText("Something is waiting for you")
            .build()
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(notificationTitle(ask))
            .setContentText(ask.details?.let(::mono) ?: who ?: ask.kind)
            .setSubText(if (ask.details != null) who else null)
            .apply {
                largeIcon(ask, color)?.let { setLargeIcon(it) }
                // Expanded: the machine part whole, what approving does, then what Autopilot would do (Manual).
                val more = listOfNotNull(untrusted(item.headline).ifBlank { null }, item.suggestion)
                val lines = listOfNotNull<CharSequence>(ask.details?.let(::mono)) + more
                if (lines.isNotEmpty()) {
                    val big = android.text.SpannableStringBuilder()
                    lines.forEachIndexed { i, line -> if (i > 0) big.append('\n'); big.append(line) }
                    setStyle(Notification.BigTextStyle().bigText(big))
                }
                // Next (Outside 01 and 02): Deny and Review. Approving always opens Reins, where the whole ask is shown;
                // a routine request can be denied from here. What is asked every time only opens the sheet.
                if (item.kind == PendingKind.REQUEST && item.quick) {
                    addAction(Notification.Action.Builder(null, "Deny", answer(item.id, approve = false)).build())
                    addAction(Notification.Action.Builder(null, "Review", openItem(item)).build())
                }
                // The time left, counting down on the notification itself.
                item.waitUntil?.takeIf { it * 1000 > System.currentTimeMillis() }?.let { until ->
                    setUsesChronometer(true)
                    setChronometerCountDown(true)
                    setWhen(until * 1000)
                    setShowWhen(true)
                }
            }
            // Colorized in the field's colour (And-Outside 01): the kind of ask, before a word is read.
            .setColor(color)
            .setColorized(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setAutoCancel(true)
            .setContentIntent(openItem(item))
            .apply { if (item.waitUntil == null || item.waitUntil!! * 1000 <= System.currentTimeMillis()) setWhen(item.createdAt * 1000) }
            // Gone when the server lets the request go; a refresh takes it away sooner if it was answered elsewhere.
            .setTimeoutAfter(lifetimeMillis(item))
            .addExtras(android.os.Bundle().apply { putString(EXTRA_ITEM, item.id) })
            .build()
        manager.notify(tag, notificationId(item.id), notification)
    }

    /**
     * Takes away the notifications of items that no longer wait ([waiting] are the ids that do): answered from the
     * notification, on another device, by a grant or Autopilot, or expired while the phone was asleep. Only what was
     * posted before [readAt] (when the list was read): anything newer is not in the list yet.
     */
    fun dropStale(waiting: Set<String>, readAt: Long) {
        manager.activeNotifications
            .filter { n -> n.tag == tag && n.postTime < readAt && n.notification.extras.getString(EXTRA_ITEM)?.let { it !in waiting } == true }
            .forEach { manager.cancel(tag, it.id) }
    }

    /** How long an item's notification may stay: until the server lets it go ([ITEM_LIFETIME_SECS] after it was made). */
    private fun lifetimeMillis(item: PendingItem, nowMillis: Long = System.currentTimeMillis()): Long {
        // A ticket waits until its deadline.
        item.ticket?.let { return (it.deadline * 1000 - nowMillis).coerceAtLeast(MIN_LIFETIME_MS) }
        return ((item.createdAt + ITEM_LIFETIME_SECS) * 1000 - nowMillis).coerceIn(MIN_LIFETIME_MS, ITEM_LIFETIME_SECS * 1000)
    }

    override fun autoDecided(decision: AutoDecisionView) {
        // Decided without the owner, so nothing about it needs them: no notification (a notification means something
        // to do). Whatever asked them about it is withdrawn (an ask posted while it waited, before a bypass or Lockdown
        // started), an ask open on screen closes, and the decision is in Activity.
        manager.cancel(tag, notificationId(decision.requestId))
        onDecided(decision.requestId)
        onChanged()
        // In front, a denial (Lockdown, a "never" rule) gets its quiet sound so the list's change is explained; an
        // approval stays silent (a bypass approves hundreds).
        if (Foreground.focused) {
            feedback().requestArrived(decision.requestId, needsUser = false) // answered: never the request chime
            if (decision.verdict != Verdict.APPROVE) feedback().play(Event.AutoDenied)
        }
    }

    override fun autopilotChanged(event: AutopilotEvent) {
        onAutopilot(event)
        when (event) {
            is AutopilotEvent.BypassEnded -> if (Foreground.focused) feedback().play(Event.BypassOff)
            // Back to the mode it interrupted, which lets less happen.
            is AutopilotEvent.OvernightEnded -> if (Foreground.focused) feedback().play(Event.AutopilotOff)
            is AutopilotEvent.Paused -> paused(event)
            is AutopilotEvent.ModeChanged -> Unit
        }
    }

    /**
     * Circles (Decisions v2, 04). A request of another account waiting for this phone is the one Circle notification,
     * titled by person ("Sam: Buy headphones waiting"), with Open only: it is answered in For others, never inline.
     * Anything else (a review settled, a person added, an owner's progress) refreshes the screens quietly. A guardian
     * notice (a ward asked to leave, ended early, reset the account) is the one guardianship notification: "Sam:
     * guardianship changed", opening Settings › Circles.
     */
    override fun circleChanged(event: CircleEvent) {
        onCircles()
        if (event is CircleEvent.GuardianNotice) {
            guardianNotice(event.notice)
            return
        }
        val review = (event as? CircleEvent.ReviewWaiting)?.review ?: return
        if (Foreground.focused) {
            // In front, For others shows it; the chime says something arrived.
            feedback().requestArrived(CIRCLE_PREFIX + review.id, needsUser = true)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val channel = channel(Kind.Approvals)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Reins")
            .setContentText("Something is waiting for you")
            .build()
        val text = listOfNotNull(untrusted(review.subtitle).ifBlank { null }, untrusted(review.agent).ifBlank { null }).joinToString(" · ")
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(dev.reins.android.ui.circles.reviewNotificationTitle(review.copy(person = untrusted(review.person), title = untrusted(review.title))))
            .setContentText(text.ifEmpty { "For others" })
            .addAction(Notification.Action.Builder(null, "Open", openForOthers(review.id)).build())
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setAutoCancel(true)
            .setContentIntent(openForOthers(review.id))
            .setWhen(review.createdAt * 1000)
            .setTimeoutAfter((review.deadline * 1000 - System.currentTimeMillis()).coerceAtLeast(MIN_LIFETIME_MS))
            .addExtras(android.os.Bundle().apply { putString(EXTRA_CIRCLE, review.id) })
            .build()
        manager.notify(tag, notificationId(CIRCLE_PREFIX + review.id), notification)
    }

    /** "Sam: guardianship changed", with what changed; posted in front too, but silently there. */
    private fun guardianNotice(notice: dev.reins.core.GuardianNoticeView) {
        val inFront = Foreground.focused
        if (inFront) feedback().play(Event.Alert)
        if (!manager.areNotificationsEnabled()) return
        val channel = channel(Kind.Status, silent = inFront)
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Reins")
            .setContentText("A guardianship changed")
            .build()
        val line = untrusted(notice.line).ifBlank { "Changed" }
        val text = if (notice.verified) line else "$line · ${dev.reins.android.ui.circles.REPORTED_BY_SERVER}"
        val notification = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(dev.reins.android.ui.circles.guardianNotificationTitle(untrusted(notice.ward)))
            .setContentText(text)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setAutoCancel(true)
            .setContentIntent(openCircles(notice.id))
            .setWhen(notice.at * 1000)
            .build()
        manager.notify(tag, notificationId(GUARDIAN_PREFIX + notice.id), notification)
    }

    /** Opens Settings › Circles (a guardian notice's notification). */
    private fun openCircles(id: String): PendingIntent = PendingIntent.getActivity(
        context,
        notificationId(GUARDIAN_PREFIX + id),
        Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_CIRCLES)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .forProfile(),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** Takes away the notifications of Circle reviews that no longer wait ([waiting] are the ids that do). */
    fun dropStaleCircles(waiting: Set<String>) {
        manager.activeNotifications
            .filter { n -> n.tag == tag && n.notification.extras.getString(EXTRA_CIRCLE)?.let { it !in waiting } == true }
            .forEach { manager.cancel(tag, it.id) }
    }

    /** Opens For others in Activity (a Circle review's notification). */
    private fun openForOthers(id: String): PendingIntent = PendingIntent.getActivity(
        context,
        notificationId(CIRCLE_PREFIX + id),
        Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_FOR_OTHERS)
            .putExtra(EXTRA_ID, id)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .forProfile(),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

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
        manager.notify(tag, notificationId("paused:" + event.connectionId), notification)
    }

    /**
     * The ongoing "Bypass on" notification while [notice] says a bypass runs, with a live countdown and Stop; null
     * takes it away. It also times itself out at the end (the core ends the bypass by itself either way). A bypass
     * without an end shows how long it has run instead, and stays until it is stopped.
     */
    fun showBypass(notice: AutopilotText.BypassNotice?, nowMillis: Long = System.currentTimeMillis()) =
        showRunning(BYPASS_ID, Kind.Bypass, BypassStopReceiver.ACTION_STOP, runningColor(Kind.Bypass), notice, nowMillis)

    /** The ongoing "Overnight on" notification, as [showBypass] (its own channel, colour and Stop); null takes it away. */
    fun showOvernight(notice: AutopilotText.BypassNotice?, nowMillis: Long = System.currentTimeMillis()) =
        showRunning(OVERNIGHT_ID, Kind.Overnight, BypassStopReceiver.ACTION_STOP_OVERNIGHT, runningColor(Kind.Overnight), notice, nowMillis)

    private fun showRunning(id: Int, kind: Kind, stopAction: String, color: Int, notice: AutopilotText.BypassNotice?, nowMillis: Long) {
        if (notice == null || !notice.untilOff && notice.until * 1000 <= nowMillis) {
            manager.cancel(tag, id)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val stop = PendingIntent.getBroadcast(
            context,
            id,
            Intent(context, BypassStopReceiver::class.java).setAction(stopAction).forProfile(),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = Notification.Builder(context, channel(kind))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(notice.title)
            .setContentText(notice.text)
            .setStyle(Notification.BigTextStyle().bigText(notice.text))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(true)
            .setUsesChronometer(true)
        if (notice.untilOff) {
            // No end to count down to: the chronometer counts up from when it started, and it stays until stopped.
            builder.setWhen((notice.since ?: nowMillis / 1000) * 1000).setChronometerCountDown(false)
        } else {
            builder.setWhen(notice.until * 1000).setChronometerCountDown(true).setTimeoutAfter(notice.until * 1000 - nowMillis)
        }
        // Android 16 Live Update: promoted to the top of the shade and the status bar chip, with its chronometer. A
        // promoted notification is coloured with setColor only, never colorized.
        if (promotes(android.os.Build.VERSION.SDK_INT)) builder.setRequestPromotedOngoing(true)
        val notification = builder
            .setColor(color)
            .setCategory(Notification.CATEGORY_STATUS)
            .setContentIntent(openAutopilot())
            .addAction(Notification.Action.Builder(null, "Stop now", stop).build())
            .build()
        manager.notify(tag, id, notification)
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
        manager.notify(tag, DOWNLOAD_DONE_ID, notification)
    }

    override fun itemResolved(id: String) {
        manager.cancel(tag, notificationId(id))
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
            .setContentText("Another phone took over. Open Reins to register this one again.")
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(tag, REPLACED_ID, notification)
    }

    /** Approve or Deny from the notification ([ApprovalActionReceiver]). */
    private fun answer(id: String, approve: Boolean): PendingIntent = PendingIntent.getBroadcast(
        context,
        notificationId(id) xor if (approve) 0x1 else 0x2,
        Intent(context, ApprovalActionReceiver::class.java)
            .setAction(if (approve) ApprovalActionReceiver.ACTION_APPROVE else ApprovalActionReceiver.ACTION_DENY)
            .putExtra(EXTRA_ID, id)
            .forProfile(),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /**
     * Held emails went too late and now ask the owner "Still send?" (08 R10): an ask, so it is notified (a send held
     * automatically never is). Says nothing of the emails; tapping opens Activity, where Send and Discard are.
     */
    fun stillSend(count: Int) {
        onChanged()
        if (count <= 0 || !manager.areNotificationsEnabled()) return
        val notification = Notification.Builder(context, channel(Kind.Approvals, silent = Foreground.focused))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(dev.reins.android.ui.activity.STILL_SEND)
            .setContentText(if (count == 1) "An email was not sent" else "$count emails were not sent")
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(tag, STILL_SEND_ID, notification)
    }

    /** Nothing asks "Still send?" any more (sent or discarded, here or elsewhere): its notification goes. */
    fun dropStillSend() = manager.cancel(tag, STILL_SEND_ID)

    /** An answer from the notification did not go through: the request still waits, one tap away. */
    fun answerFailed(id: String, approved: Boolean, message: String) {
        onChanged()
        if (!manager.areNotificationsEnabled()) return
        val notification = Notification.Builder(context, channel(Kind.Approvals, silent = Foreground.focused))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(if (approved) "Not approved yet" else "Not denied yet")
            .setContentText(message)
            .setStyle(Notification.BigTextStyle().bigText("$message Open it to try again."))
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setAutoCancel(true)
            .setContentIntent(
                PendingIntent.getActivity(
                    context,
                    notificationId(id),
                    Intent(context, MainActivity::class.java)
                        .setAction(ACTION_OPEN_ITEM)
                        .putExtra(EXTRA_KIND, KIND_REQUEST)
                        .putExtra(EXTRA_ID, id)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                        .forProfile(),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
            .build()
        manager.notify(tag, notificationId(id), notification)
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
                    PendingKind.JOIN -> KIND_JOIN
                },
            )
            .putExtra(EXTRA_ID, item.id)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .forProfile()
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
            .forProfile()
        val code = (activityId and 0x3FFF_FFFF).toInt() or 0x4000_0000
        return PendingIntent.getActivity(context, code, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** Opens Settings > Autopilot (the bypass, the download). */
    private fun openAutopilot(): PendingIntent = PendingIntent.getActivity(
        context,
        BYPASS_ID,
        Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_AUTOPILOT)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .forProfile(),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP).forProfile(),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** Names this notifier's profile on an intent, and keeps it apart from the same intent of another profile (an
     *  identifier counts in `filterEquals`, so one profile's pending intent never takes over the other's extras). */
    private fun Intent.forProfile(): Intent = WorkProfiles.stamp(context, this, profile.id)

    /**
     * A push or an action for another profile than this one: says so without any detail, and opens the app on that
     * profile (the current core never handles it).
     */
    fun otherProfile(target: WorkProfiles.Active, id: Int = OTHER_PROFILE_ID) {
        if (!manager.areNotificationsEnabled()) return
        val open = PendingIntent.getActivity(
            context,
            id,
            WorkProfiles.stamp(
                context,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                target.id,
            ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, channel(context, Kind.Status, settings(), silent = false, profile = target))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(if (target.managed) target.label else "Reins")
            .setContentText("Something waits for you. Open Reins.")
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()
        manager.notify(target.id, id, notification)
    }

    /**
     * An org review waits for this member's signature (O2): something to act on, so it is notified, on [target]'s
     * channel; tapping opens that profile's reviews. Says nothing of the review itself.
     */
    fun orgReview(target: WorkProfiles.Active, reviewId: String) = orgAction(
        target,
        reviewId,
        // A family's ask (P7): a member's request to leave for its guardians, or a guardian's question for the member.
        when (reviewId) {
            "family:leave" -> "A request to leave waits for you."
            "family:minor" -> "Your family asks you something."
            else -> "A review waits for you."
        },
    )

    /**
     * A web passkey the member added in the browser waits for this phone's confirmation (O3): notified like a review,
     * opening the same screen, where its card shows the fingerprint.
     */
    fun orgPasskey(target: WorkProfiles.Active, credentialId: String) =
        orgAction(target, credentialId, "Confirm your new passkey.")

    private fun orgAction(target: WorkProfiles.Active, reviewId: String, text: String) {
        onChanged()
        if (!manager.areNotificationsEnabled()) return
        // A family's ask (P7): a member's request to leave for its guardians, or a guardian's question for the member.
        val family = reviewId.startsWith(FAMILY_PREFIX)
        val open = PendingIntent.getActivity(
            context,
            notificationId(reviewId),
            WorkProfiles.stamp(
                context,
                Intent(context, MainActivity::class.java)
                    .setAction(if (family) ACTION_OPEN_FAMILY else ACTION_OPEN_ORG_REVIEWS)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                target.id,
            ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, channel(context, Kind.Approvals, settings(), silent = false, profile = target))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(if (target.managed) target.label else "Reins")
            .setContentText(text)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()
        manager.notify(target.id, notificationId(reviewId), notification)
    }

    /** Takes away every notification of this profile (signing out), and none of another's. */
    fun cancelOwn() {
        manager.activeNotifications.filter { it.tag == tag }.forEach { manager.cancel(it.tag, it.id) }
    }

    private fun notificationId(id: String): Int = itemNotificationId(id)

    /** The request's icon (the integration with the action on its corner), ringed in the notification's colour. */
    private fun largeIcon(ask: AskSentence, ring: Int): android.graphics.drawable.Icon? = try {
        val px = (LARGE_ICON_DP * context.resources.displayMetrics.density).toInt().coerceAtLeast(1)
        android.graphics.drawable.Icon.createWithBitmap(IntegrationBitmap.draw(context, ask.provider, ask.action, px, ring))
    } catch (e: Exception) {
        android.util.Log.w("Reins", "No large icon for a notification", e)
        null
    }

    companion object {
        /** The notification id of a waiting item: its arrival, a failed answer and a ticket's reminders share it. */
        fun itemNotificationId(id: String): Int = id.hashCode() and 0x7FFFFFFF or 0x100

        /** Approve or Deny from a notification ([ApprovalActionReceiver]). */
        private fun answer(context: Context, id: String, approve: Boolean, profile: WorkProfiles.Active): PendingIntent =
            PendingIntent.getBroadcast(
                context,
                itemNotificationId(id) xor if (approve) 0x1 else 0x2,
                WorkProfiles.stamp(
                    context,
                    Intent(context, ApprovalActionReceiver::class.java)
                        .setAction(if (approve) ApprovalActionReceiver.ACTION_APPROVE else ApprovalActionReceiver.ACTION_DENY)
                        .putExtra(EXTRA_ID, id),
                    profile.id,
                ),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )

        /** Deny [profile]'s request from a notification (a ticket's reminder). */
        fun denyRequest(context: Context, id: String, profile: WorkProfiles.Active = WorkProfiles.PERSONAL): PendingIntent =
            answer(context, id, approve = false, profile = profile)

        /** Opens [profile]'s waiting request's approval screen. */
        fun openRequest(context: Context, id: String, profile: WorkProfiles.Active = WorkProfiles.PERSONAL): PendingIntent =
            PendingIntent.getActivity(
                context,
                itemNotificationId(id),
                WorkProfiles.stamp(
                    context,
                    Intent(context, MainActivity::class.java)
                        .setAction(ACTION_OPEN_ITEM)
                        .putExtra(EXTRA_KIND, KIND_REQUEST)
                        .putExtra(EXTRA_ID, id)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                    profile.id,
                ),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )

        /** The large icon's size: what a notification shows at its side. */
        private const val LARGE_ICON_DP = 64

        /** The ask as a notification title: the target in italic (ASK-TEXT.md). */
        fun notificationTitle(ask: AskSentence): CharSequence {
            val text = android.text.SpannableString(ask.text)
            val target = ask.target
            val at = if (target.isNullOrEmpty()) -1 else ask.text.lastIndexOf(target)
            if (at >= 0) text.setSpan(android.text.style.StyleSpan(android.graphics.Typeface.ITALIC), at, at + target!!.length, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            return text
        }

        /** Machine text (a command, a branch, an address) in mono. */
        fun mono(text: String): CharSequence = android.text.SpannableString(text).apply {
            setSpan(android.text.style.TypefaceSpan("monospace"), 0, length, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }

        /** Android 16 (API 36): Live Updates, `setRequestPromotedOngoing`. */
        const val PROMOTED_SDK = 36

        /** Whether a running bypass or Overnight asks to be a Live Update on [sdk]. */
        fun promotes(sdk: Int): Boolean = sdk >= PROMOTED_SDK

        /** A waiting item's notification colour: its field, as on the stack's card (Reins Signal). */
        fun notificationColor(item: PendingItem): Int =
            (pendingTone(item, null).field ?: dev.reins.android.design.Field.Push).fill().toArgb()

        /** A running bypass is red like what can't be undone; Overnight is violet like power handed over. */
        fun runningColor(kind: Kind): Int = when (kind) {
            Kind.Overnight -> dev.reins.android.design.Field.Grant
            else -> dev.reins.android.design.Field.Delete
        }.fill().toArgb()

        /** Bumped whenever a channel's sound or vibration changes (they cannot be edited once created). */
        const val CHANNEL_VERSION = 2

        /** The unversioned channels of builds without chimes. */
        private val LEGACY_CHANNELS = setOf("approvals", "grants", "status")

        /** Automatic decisions are never notified any more (a notification means something to do): their channels go. */
        private val RETIRED_CHANNEL_PREFIXES = listOf("autopilot-approved-v", "autopilot-denied-v")

        /** The server holds a request this long; the phone's core the same. */
        const val ITEM_LIFETIME_SECS = 600L

        /** A clock far off the server's never hides a notification at once. */
        private const val MIN_LIFETIME_MS = 60_000L

        /** The item a notification is about, in its extras (see [dropStale]). */
        const val EXTRA_ITEM = "dev.reins.android.ITEM"

        const val ACTION_OPEN_ITEM = "dev.reins.android.OPEN_ITEM"
        const val EXTRA_KIND = "kind"
        const val EXTRA_ID = "id"
        const val KIND_REQUEST = "request"
        const val KIND_PAIRING = "pairing"
        const val KIND_BLOB = "blob"
        const val KIND_JOIN = "join"
        const val ACTION_OPEN_ACTIVITY = "dev.reins.android.OPEN_ACTIVITY"
        const val EXTRA_ACTIVITY_ID = "activity"
        const val ACTION_OPEN_AUTOPILOT = "dev.reins.android.OPEN_AUTOPILOT"
        /** A work profile's org reviews ([orgReview]). */
        const val ACTION_OPEN_ORG_REVIEWS = "dev.reins.android.OPEN_ORG_REVIEWS"
        const val ACTION_OPEN_FAMILY = "dev.reins.android.OPEN_FAMILY"
        /** An org-review push id the server sends for a family's ask (`family:leave`, `family:minor`). */
        const val FAMILY_PREFIX = "family:"
        /** A Circle review's notification: Activity, at For others. */
        const val ACTION_OPEN_FOR_OTHERS = "dev.reins.android.OPEN_FOR_OTHERS"
        /** The Circle review a notification is about, in its extras (see [dropStaleCircles]). */
        const val EXTRA_CIRCLE = "dev.reins.android.CIRCLE"
        /** Keeps a review's notification id apart from a request's of the same id. */
        private const val CIRCLE_PREFIX = "circle:"
        /** A guardian notice's notification: Settings › Circles. */
        const val ACTION_OPEN_CIRCLES = "dev.reins.android.OPEN_CIRCLES"
        private const val GUARDIAN_PREFIX = "guardian:"
        private const val REPLACED_ID = 1
        const val BYPASS_ID = 3
        const val DOWNLOAD_ID = 4
        private const val DOWNLOAD_DONE_ID = 5
        const val OVERNIGHT_ID = 6
        /** The neutral "Open Reins" of a push or action for another profile ([otherProfile]). */
        const val OTHER_PROFILE_ID = 7
        /** "Still send?" of held emails that went too late ([stillSend]); 8 is the Overnight digest's. */
        const val STILL_SEND_ID = 9

        /** The settings group of Autopilot's channels, and the group its notifications fold into. */
        const val GROUP_AUTOPILOT = "autopilot"

        /** A channel id, e.g. `approvals-v2-sv` (sound and vibration), `-s`, `-v` or `-q` (quiet). */
        fun channelId(kind: Kind, sound: Boolean, vibrate: Boolean, prefix: String = WorkProfiles.PERSONAL.channelPrefix): String =
            prefix + "${kind.key}-v$CHANNEL_VERSION-" + (if (sound) "s" else "") + (if (vibrate) "v" else "") + (if (!sound && !vibrate) "q" else "")

        /** Creates (once) and returns the channel [settings] ask for: the kind's chime under its category switch, vibration under Haptics. */
        fun channel(
            context: Context,
            kind: Kind,
            settings: FeedbackSettings,
            silent: Boolean = false,
            profile: WorkProfiles.Active,
        ): String {
            val sound = !silent && kind.sound != null && settings.allows(kind.category)
            val vibrate = !silent && kind.pattern != null && settings.hapticsOn
            val id = channelId(kind, sound, vibrate, profile.channelPrefix)
            val manager = context.getSystemService(NotificationManager::class.java)
            if (manager.getNotificationChannel(id) != null) return id
            // A work profile's channels are its own, grouped under the org's name; the Personal ones are as before.
            val group = if (profile.managed) profile.channelPrefix + "profile" else kind.group
            group?.let { g ->
                if (manager.getNotificationChannelGroup(g) == null) {
                    val name = if (profile.managed) profile.label else "Autopilot"
                    manager.createNotificationChannelGroup(android.app.NotificationChannelGroup(g, name))
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
            val label = kind.labelRes?.let(context::getString) ?: kind.label
            val channel = NotificationChannel(id, label + suffix, kind.importance)
            channel.description = kind.description
            group?.let { channel.group = it }
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
                .filter { ch ->
                    ch.id in LEGACY_CHANNELS || RETIRED_CHANNEL_PREFIXES.any { ch.id.startsWith(it) } ||
                        Kind.entries.any { ch.id.startsWith(it.key + "-v") } && !ch.id.contains(current)
                }
                .forEach { manager.deleteNotificationChannel(it.id) }
        }
    }
}
