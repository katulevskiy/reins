package dev.rewarden.android.feedback

/**
 * Anything pressable answers with a default tap, unless the moment already has its own feedback. The default is
 * deferred a few ms; if a haptic (or a cue) was requested explicitly around the same press, that one is the answer and
 * the default stays out of its way. Haptic and cue are claimed separately, so opening a sheet (an explicit Open cue)
 * still keeps the tap's haptic.
 */
class ClaimTracker(private val clock: () -> Long) {
    private var hapticAt = Long.MIN_VALUE / 2
    private var cueAt = Long.MIN_VALUE / 2
    private var quietAt = Long.MIN_VALUE / 2

    fun claimHaptic() {
        hapticAt = clock()
    }

    fun claimCue() {
        cueAt = clock()
    }

    /** Was an explicit haptic requested from [WINDOW_BEFORE_MS] before [releasedAt] until now? */
    fun hapticClaimed(releasedAt: Long): Boolean = hapticAt >= releasedAt - WINDOW_BEFORE_MS

    fun cueClaimed(releasedAt: Long): Boolean = cueAt >= releasedAt - WINDOW_BEFORE_MS

    /** Was any cue requested (or a close silenced) in the last [windowMs]? */
    fun cueWithin(windowMs: Long): Boolean = clock() - maxOf(cueAt, quietAt) < windowMs

    /** A choice was made in a dialog: the dialog closing right after is the choice's doing, not a sound of its own. */
    fun quiet() {
        quietAt = clock()
    }

    companion object {
        /** Explicit feedback this far before the release still counts (the click handler runs just ahead of it). */
        const val WINDOW_BEFORE_MS = 100L

        /** How long the default waits for an explicit answer. */
        const val DEFER_MS = 40L
    }
}

/** What `pressable` and the sheet and dialog helpers talk to; the real engine implements it next to [Feedback]. */
interface TapFeedback {
    /** A press was released inside its target: answer with the default tap unless the moment claims it. */
    fun defaultTap()

    /**
     * [cue] unless some cue was requested in the last [windowMs]: a sheet or dialog closing right after the action it
     * hosted already has that action's sound, and the close would only muddy it.
     */
    fun cueUnlessRecent(cue: Cue, windowMs: Long = 250)

    /** A choice was made: the close that follows stays silent. */
    fun quietClose()
}
