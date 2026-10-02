package dev.rewarden.android.feedback

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** How hard haptics hit: scales composition primitives (and waveform amplitudes) where the hardware allows. */
enum class HapticStrength(val label: String, val scale: Float) {
    Subtle("Subtle", 0.5f),
    Standard("Standard", 1f),
    Strong("Strong", 1.5f),
}

/** Which in-app switch governs a sound. */
enum class CueCategory {
    /** Taps, toggles, sheets, approve and deny: the answer to the user's own hand. */
    Interface,

    /** A request, pairing or upload is waiting (in-app and its notification). */
    Requests,

    /** What Autopilot decides by itself. */
    Autopilot,

    /** Grants ending, this phone losing its role: the attention chime (in-app and its notification). */
    Alerts,
}

/**
 * The user's choices for sound and haptics. Everything defaults on.
 *
 * [master] ("Sounds & haptics") is absolute: off silences every sound and every vibration, in the app and in the
 * notifications it posts; on hands the decision to the switches below it, and nothing the phone itself has set (touch
 * sounds, touch feedback, silent mode, Do Not Disturb) is consulted. See [FeedbackGate].
 */
data class FeedbackSettings(
    /** The one switch above all others: off = no sound and no haptic anywhere. */
    val master: Boolean = true,
    /** Every sound, under [master]. */
    val sounds: Boolean = true,
    val interfaceSounds: Boolean = true,
    val requestSounds: Boolean = true,
    val autopilotSounds: Boolean = true,
    val alertSounds: Boolean = true,
    /** 0..1 slider; the audible gain follows [gain]. 50% plays the (mastered, loud) files at half volume, 100% at full. */
    val volume: Float = DEFAULT_VOLUME,
    val haptics: Boolean = true,
    val strength: HapticStrength = HapticStrength.Standard,
) {
    /** Sounds are wanted at all: the master and the sounds switch. */
    val soundsOn: Boolean get() = master && sounds

    /** Haptics are wanted at all: the master and the haptics switch. */
    val hapticsOn: Boolean get() = master && haptics

    fun allows(category: CueCategory): Boolean = soundsOn && when (category) {
        CueCategory.Interface -> interfaceSounds
        CueCategory.Requests -> requestSounds
        CueCategory.Autopilot -> autopilotSounds
        CueCategory.Alerts -> alertSounds
    }

    /** Linear gain on the cue's native level, 0..[MAX_GAIN]; see [gainFor]. */
    val gain: Float get() = gainFor(volume)

    companion object {
        /** Half way. */
        const val DEFAULT_VOLUME = 0.5f

        /** At 100% every cue is twice as loud (+6 dB) as at the default 50%; the files carry the headroom. */
        const val MAX_GAIN = 2f

        /**
         * The slider is linear, loudness is not. Up to the default the gain is the square of twice the slider (25%
         * reads about -12 dB), above it equal dB steps up to +6 dB at 100%. Both pieces meet at gain 1 with no jump.
         */
        fun gainFor(slider: Float): Float {
            val v = 2f * slider.coerceIn(0f, 1f)
            return if (v <= 1f) v * v else Math.pow(2.0, (v - 1f).toDouble()).toFloat()
        }
    }
}

/** Plain key/value form of [FeedbackSettings], free of Android types so it can be tested. */
object FeedbackSettingsCodec {
    private const val P = "feedback."

    /** Layout of the stored values, for a future migration. */
    const val VERSION = 1
    const val VERSION_KEY = P + "version"

    fun write(s: FeedbackSettings): Map<String, Any> = mapOf(
        P + "master" to s.master,
        P + "sounds" to s.sounds,
        P + "interface" to s.interfaceSounds,
        P + "requests" to s.requestSounds,
        P + "autopilot" to s.autopilotSounds,
        P + "alerts" to s.alertSounds,
        P + "volume" to s.volume,
        P + "haptics" to s.haptics,
        P + "strength" to s.strength.name,
        VERSION_KEY to VERSION,
    )

    /** Missing or malformed values read as the defaults. */
    fun read(get: (String) -> Any?): FeedbackSettings {
        val d = FeedbackSettings()
        fun bool(key: String, default: Boolean) = get(P + key) as? Boolean ?: default
        return FeedbackSettings(
            master = bool("master", d.master),
            sounds = bool("sounds", d.sounds),
            interfaceSounds = bool("interface", d.interfaceSounds),
            requestSounds = bool("requests", d.requestSounds),
            autopilotSounds = bool("autopilot", d.autopilotSounds),
            alertSounds = bool("alerts", d.alertSounds),
            volume = (get(P + "volume") as? Float ?: d.volume).takeIf { it.isFinite() }?.coerceIn(0f, 1f) ?: d.volume,
            haptics = bool("haptics", d.haptics),
            strength = HapticStrength.entries.firstOrNull { it.name == get(P + "strength") } ?: d.strength,
        )
    }
}

/** [FeedbackSettings] persisted in the `feedback` preferences. Read once, then served from memory. */
class FeedbackStore(private val prefs: SharedPreferences) {
    constructor(context: Context) : this(context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE))

    private val _settings = MutableStateFlow(FeedbackSettingsCodec.read { prefs.all[it] })
    val settings: StateFlow<FeedbackSettings> = _settings.asStateFlow()

    val current: FeedbackSettings get() = _settings.value

    fun update(change: FeedbackSettings.() -> FeedbackSettings) {
        val next = _settings.value.change()
        if (next == _settings.value) return
        _settings.value = next
        val edit = prefs.edit()
        for ((key, value) in FeedbackSettingsCodec.write(next)) {
            when (value) {
                is Boolean -> edit.putBoolean(key, value)
                is Float -> edit.putFloat(key, value)
                is Int -> edit.putInt(key, value)
                is String -> edit.putString(key, value)
            }
        }
        edit.apply()
    }

    private companion object {
        const val PREFS = "feedback"
    }
}
