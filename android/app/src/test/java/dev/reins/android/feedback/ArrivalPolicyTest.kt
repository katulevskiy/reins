package dev.reins.android.feedback

import android.app.NotificationManager
import android.media.AudioManager
import org.junit.Assert.assertEquals
import org.junit.Test

/** What a request arriving while the app is open gets: the chime and haptic once per request and per burst. */
class ArrivalPolicyTest {
    private var now = 100_000L
    private val policy = ArrivalPolicy { now }
    private val on = FeedbackSettings()

    private fun arrive(id: String, needsUser: Boolean = true, settings: FeedbackSettings = on, ringer: Ringer = Ringer.Normal) =
        policy.decide(id, needsUser, settings, ringer)

    @Test
    fun `a new request that needs the user chimes and buzzes`() {
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("req1"))
    }

    @Test
    fun `what Autopilot, a bypass or a grant answered gets nothing, then or later`() {
        assertEquals(ArrivalOutcome.Nothing, arrive("auto1", needsUser = false))
        now += 10_000
        assertEquals(ArrivalOutcome.Nothing, arrive("auto1"))
        // and it does not hold back the next real one
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("req2"))
    }

    @Test
    fun `a burst chimes once and its requests stay announced`() {
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("a"))
        for (id in listOf("b", "c", "d")) {
            now += 400
            assertEquals(id, ArrivalOutcome.Nothing, arrive(id))
        }
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.Nothing, arrive("c"))
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("e"))
    }

    @Test
    fun `at most one chime per window, counted from the last one`() {
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("a"))
        now += ArrivalPolicy.COALESCE_MS - 1
        assertEquals(ArrivalOutcome.Nothing, arrive("b"))
        now += 1
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("c"))
    }

    @Test
    fun `an update to a request already shown stays quiet`() {
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("req1"))
        now += 60_000
        assertEquals(ArrivalOutcome.Nothing, arrive("req1"))
    }

    @Test
    fun `Sound when open off keeps the haptic only`() {
        assertEquals(ArrivalOutcome.HapticOnly, arrive("a", settings = FeedbackSettings(soundWhenOpen = false)))
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.Nothing, arrive("b", settings = FeedbackSettings(soundWhenOpen = false, haptics = false)))
    }

    @Test
    fun `the sound switches above it and the master decide too`() {
        assertEquals(ArrivalOutcome.HapticOnly, arrive("a", settings = FeedbackSettings(requestSounds = false)))
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.HapticOnly, arrive("b", settings = FeedbackSettings(sounds = false)))
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.ChimeOnly, arrive("c", settings = FeedbackSettings(haptics = false)))
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.Nothing, arrive("d", settings = FeedbackSettings(master = false)))
    }

    @Test
    fun `a silent arrival does not start a burst window`() {
        assertEquals(ArrivalOutcome.Nothing, arrive("a", settings = FeedbackSettings(master = false)))
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("b"))
    }

    @Test
    fun `vibrate mode only buzzes, silent mode does nothing`() {
        assertEquals(ArrivalOutcome.HapticOnly, arrive("a", ringer = Ringer.Vibrate))
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.Nothing, arrive("b", ringer = Ringer.Silent))
    }

    @Test
    fun `the ringer follows the phone's mode and Do Not Disturb`() {
        val all = NotificationManager.INTERRUPTION_FILTER_ALL
        assertEquals(Ringer.Normal, Ringer.of(AudioManager.RINGER_MODE_NORMAL, all))
        assertEquals(Ringer.Vibrate, Ringer.of(AudioManager.RINGER_MODE_VIBRATE, all))
        assertEquals(Ringer.Silent, Ringer.of(AudioManager.RINGER_MODE_SILENT, all))
        assertEquals(Ringer.Normal, Ringer.of(AudioManager.RINGER_MODE_NORMAL, NotificationManager.INTERRUPTION_FILTER_UNKNOWN))
        for (dnd in listOf(
            NotificationManager.INTERRUPTION_FILTER_PRIORITY,
            NotificationManager.INTERRUPTION_FILTER_ALARMS,
            NotificationManager.INTERRUPTION_FILTER_NONE,
        )) {
            assertEquals(Ringer.Silent, Ringer.of(AudioManager.RINGER_MODE_NORMAL, dnd))
            assertEquals(Ringer.Silent, Ringer.of(AudioManager.RINGER_MODE_VIBRATE, dnd))
        }
    }

    @Test
    fun `old ids are forgotten after a while, so memory stays bounded`() {
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("first"))
        repeat(ArrivalPolicy.REMEMBERED) { arrive("other$it") }
        now += ArrivalPolicy.COALESCE_MS
        assertEquals(ArrivalOutcome.ChimeAndHaptic, arrive("first"))
    }
}
