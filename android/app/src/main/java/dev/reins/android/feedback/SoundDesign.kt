package dev.reins.android.feedback

import kotlin.math.pow

/**
 * One sound cue as the engine plays it: which file, how loud relative to the set, how important it is when sounds
 * compete, and which in-app switch governs it. Files are `res/raw/fx_<name>.wav`, mono 16-bit 48 kHz, mastered with
 * [CueTable.ASSET_BOOST] of headroom and trimmed to start sounding within 1 ms (they come from Zeron's sound set,
 * `docs/sound-design` there). Two cues may share a file at different levels.
 */
data class CueSpec(
    val cue: Cue,
    /** `res/raw` resource name, without extension. */
    val resource: String,
    val category: CueCategory,
    /** Level trim against the rest of the set, 0..1 (before the user's volume). */
    val gain: Float,
    /** 0 interface, 1 action, 2 event, 3 alert. Higher wins when streams are scarce. */
    val priority: Int,
    /** The same cue is never stacked within this many ms. */
    val minGapMs: Long = 60,
    /** Approximate length, used to count streams still sounding. */
    val approxMs: Long = 120,
)

object CueTable {
    /**
     * Every `fx_*.wav` is mastered this much hotter (+6 dB) than the default level, to give the volume slider its
     * headroom: `SoundPool` volume cannot exceed 1.0, so "twice as loud at 100%" lives in the file and the default
     * volume plays it at half (see [volume]).
     */
    const val ASSET_BOOST = 2f

    /** SoundPool volume (0..1) for [spec] at user gain [userGain] (0..[FeedbackSettings.MAX_GAIN]). */
    fun volume(spec: CueSpec, userGain: Float): Float = (userGain * spec.gain / ASSET_BOOST).coerceIn(0f, 1f)

    fun spec(cue: Cue): CueSpec = specs[cue.ordinal]

    private val specs: List<CueSpec> by lazy { Cue.entries.map(::build) }

    private fun build(cue: Cue): CueSpec = when (cue) {
        // The chimes, 2-4 dB above the interface set. The notification channels play the same files.
        Cue.Request -> CueSpec(cue, "fx_chime_request", CueCategory.Requests, 1f, 2, 400, 600)
        Cue.Attention -> CueSpec(cue, "fx_chime_attention", CueCategory.Alerts, 1f, 3, 400, 650)
        Cue.Done -> CueSpec(cue, "fx_chime_done", CueCategory.Interface, 0.8f, 2, 400, 520)

        // Longer action cues (about 2 dB hotter than the interface set, so trimmed).
        Cue.Send -> CueSpec(cue, "fx_send", CueCategory.Interface, 0.8f, 1, 120, 160)
        Cue.UploadReady -> CueSpec(cue, "fx_upload_ready", CueCategory.Interface, 0.8f, 2, 250, 300)
        Cue.Reconnected -> CueSpec(cue, "fx_reconnected", CueCategory.Interface, 0.8f, 2, 400, 450)
        Cue.Undo -> CueSpec(cue, "fx_undo", CueCategory.Interface, 0.8f, 1, 120, 250)

        // Interface cues: subliminal, matched in loudness.
        Cue.Tap -> CueSpec(cue, "fx_tap", CueCategory.Interface, 1f, 0, 60, 60)
        Cue.Select -> CueSpec(cue, "fx_select", CueCategory.Interface, 1f, 0, 60, 80)
        Cue.ToggleOn -> CueSpec(cue, "fx_toggle_on", CueCategory.Interface, 1f, 1, 80, 90)
        Cue.ToggleOff -> CueSpec(cue, "fx_toggle_off", CueCategory.Interface, 1f, 1, 80, 90)
        Cue.Open -> CueSpec(cue, "fx_open", CueCategory.Interface, 1f, 1, 120, 120)
        Cue.Close -> CueSpec(cue, "fx_close", CueCategory.Interface, 1f, 1, 120, 120)
        Cue.Detent -> CueSpec(cue, "fx_detent", CueCategory.Interface, 1f, 0, 60, 60)
        Cue.Delete -> CueSpec(cue, "fx_delete", CueCategory.Interface, 1f, 1, 120, 120)
        Cue.Copy -> CueSpec(cue, "fx_copy", CueCategory.Interface, 1f, 1, 120, 120)
        Cue.Error -> CueSpec(cue, "fx_error", CueCategory.Interface, 1f, 3, 250, 180)
        Cue.Refresh -> CueSpec(cue, "fx_refresh", CueCategory.Interface, 1f, 1, 200, 120)

        // Autopilot modes answer the switch, so they are interface sounds. Surge swells for half a second and ranks as
        // an action (a later tap does not cut it).
        Cue.Surge -> CueSpec(cue, "fx_surge", CueCategory.Interface, 1f, 1, 400, 490)
        Cue.Zip -> CueSpec(cue, "fx_zip", CueCategory.Interface, 1f, 0, 100, 85)
        Cue.FastOn -> CueSpec(cue, "fx_fast_on", CueCategory.Interface, 0.7f, 1, 300, 195)
        Cue.FastOff -> CueSpec(cue, "fx_fast_off", CueCategory.Interface, 0.8f, 1, 200, 100)
        Cue.Lockdown -> CueSpec(cue, "fx_chime_attention", CueCategory.Interface, 0.8f, 2, 400, 650)

        // What Autopilot decides by itself: the user's own approve and deny, far quieter, and the lightest priority.
        Cue.AutoApproved -> CueSpec(cue, "fx_send", CueCategory.Autopilot, 0.3f, 0, 250, 160)
        Cue.AutoDenied -> CueSpec(cue, "fx_close", CueCategory.Autopilot, 0.5f, 0, 250, 120)
    }

    val all: List<CueSpec> get() = specs

    /** Each file once, for loading. */
    val resources: List<String> get() = specs.map { it.resource }.distinct()
}

/**
 * The Detent cue climbs a major-pentatonic ladder (the family's key), played by resampling one 880 Hz tick: step 0 sits
 * a fourth below the reference and each step walks up the scale, so dragging a slider "plays" it. Rates stay inside
 * SoundPool's 0.5..2.0 range; beyond the top the ladder holds.
 */
object DetentLadder {
    private val degrees = intArrayOf(0, 2, 4, 7, 9)
    const val BASE_SEMITONES = -7
    const val MAX_SEMITONES = 12
    const val MIN_RATE = 0.5f
    const val MAX_RATE = 2.0f

    fun semitones(step: Int): Int {
        val octave = Math.floorDiv(step, degrees.size)
        val degree = Math.floorMod(step, degrees.size)
        return (BASE_SEMITONES + 12 * octave + degrees[degree]).coerceIn(-12, MAX_SEMITONES)
    }

    fun rate(step: Int): Float = 2.0.pow(semitones(step) / 12.0).toFloat().coerceIn(MIN_RATE, MAX_RATE)
}
