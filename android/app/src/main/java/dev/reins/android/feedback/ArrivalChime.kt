package dev.reins.android.feedback

import android.app.NotificationManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.SoundPool
import java.util.concurrent.ConcurrentHashMap

/**
 * What the phone's own settings let a notification do right now: sound, only vibrate, or nothing. Read for a request
 * that arrives while Reins is open, whose notification is skipped: its chime and haptic answer to the same settings.
 */
enum class Ringer {
    Normal,
    Vibrate,
    Silent,
    ;

    companion object {
        /**
         * From `AudioManager.getRingerMode()` and `NotificationManager.getCurrentInterruptionFilter()`. Do Not Disturb
         * in any form (priority only, alarms only, total silence) counts as silent: the request is on screen anyway.
         */
        fun of(ringerMode: Int, interruptionFilter: Int): Ringer = when {
            interruptionFilter > NotificationManager.INTERRUPTION_FILTER_ALL -> Silent
            ringerMode == AudioManager.RINGER_MODE_SILENT -> Silent
            ringerMode == AudioManager.RINGER_MODE_VIBRATE -> Vibrate
            else -> Normal
        }
    }
}

/** What a request arriving while the app is open gets. */
enum class ArrivalOutcome(val chime: Boolean, val haptic: Boolean) {
    ChimeAndHaptic(true, true),
    ChimeOnly(true, false),
    HapticOnly(false, true),
    Nothing(false, false),
    ;

    companion object {
        fun of(chime: Boolean, haptic: Boolean): ArrivalOutcome = entries.first { it.chime == chime && it.haptic == haptic }
    }
}

/**
 * Decides what a request that arrives while Reins is open gets instead of its notification: the request chime and the
 * attention haptic. Pure and clock-injected so its rules are unit tested (the iOS app's `ArrivalPolicy`).
 *
 * Rules, in order:
 *  - Only a request that waits for the user. What Autopilot, a bypass or a grant answered gets nothing, and its id is
 *    remembered, so a late repeat stays quiet too.
 *  - Once per request: one already announced (an update, the same request reaching the app twice) gets nothing.
 *  - Once per burst: after a chime or haptic, nothing more for [COALESCE_MS]. The requests of a burst still count as
 *    announced, so none of them chimes later.
 *  - The chime: Sounds & haptics, Sounds, Requests and "Sound when open" on, and the phone ringing ([Ringer.Normal]).
 *  - The haptic: Sounds & haptics and Haptics on, and the phone not silenced ([Ringer.Silent]).
 */
class ArrivalPolicy(private val clock: () -> Long) {
    private val announced = LinkedHashSet<String>()
    private var lastAt = Long.MIN_VALUE / 2

    @Synchronized
    fun decide(id: String, needsUser: Boolean, settings: FeedbackSettings, ringer: Ringer): ArrivalOutcome {
        val fresh = announced.add(id)
        while (announced.size > REMEMBERED) announced.remove(announced.first())
        if (!needsUser || !fresh) return ArrivalOutcome.Nothing
        val now = clock()
        if (now - lastAt < COALESCE_MS) return ArrivalOutcome.Nothing
        val chime = settings.allows(CueCategory.Requests) && settings.soundWhenOpen && ringer == Ringer.Normal
        val haptic = settings.hapticsOn && ringer != Ringer.Silent
        val outcome = ArrivalOutcome.of(chime, haptic)
        if (outcome != ArrivalOutcome.Nothing) lastAt = now
        return outcome
    }

    companion object {
        /** At most one chime this often, however many requests arrive. */
        const val COALESCE_MS = 3_000L

        /** Request ids kept to tell a new request from one already announced. */
        const val REMEMBERED = 256
    }
}

/**
 * The request chime on the notification stream (`USAGE_NOTIFICATION`), for a request that arrives while Reins is open.
 * The same file as the Approvals channel and the in-app cue (`fx_chime_request`), in its own one-voice `SoundPool`: the
 * notification volume sets its level, and the phone's silent and vibrate modes and Do Not Disturb mute it as they would
 * mute the notification it stands in for. No audio focus is requested, so the user's music keeps playing.
 */
class NotificationChime(private val context: Context) {
    private val pool: SoundPool = SoundPool.Builder()
        .setMaxStreams(1)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_NOTIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build(),
        )
        .build()
    private val ready = ConcurrentHashMap.newKeySet<Int>()

    @Volatile
    private var sample = 0

    init {
        pool.setOnLoadCompleteListener { _, id, status -> if (status == 0) ready.add(id) }
    }

    /** Starts decoding the chime; returns at once. Call off the main thread. */
    fun load() {
        val res = context.resources.getIdentifier(CueTable.spec(Cue.Request).resource, "raw", context.packageName)
        if (res != 0) sample = pool.load(context, res, 1)
    }

    /** Whether it started (false until it has loaded). At the file's full level, as the notification plays it. */
    fun play(): Boolean {
        val id = sample.takeIf { it != 0 && it in ready } ?: return false
        return pool.play(id, 1f, 1f, 1, 0, 1f) != 0
    }
}
