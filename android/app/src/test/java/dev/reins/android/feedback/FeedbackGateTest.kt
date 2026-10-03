package dev.reins.android.feedback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FakeEnv(
    override var active: Boolean = true,
    override var hasVibrator: Boolean = true,
) : FeedbackEnvironment

class CountingEnv : FeedbackEnvironment {
    var reads = 0
    override val active: Boolean get() = true.also { reads++ }
    override val hasVibrator: Boolean get() = true.also { reads++ }
}

class FeedbackGateTest {
    private var now = 10_000L
    private var settings = FeedbackSettings()
    private val env = FakeEnv()
    private val gate = FeedbackGate({ settings }, env, { now })

    private fun why(d: Decision) = (d as? Decision.Skip)?.why

    @Test
    fun `by default everything plays`() {
        assertEquals(Decision.Play, gate.haptic(Haptic.Confirm))
        assertEquals(Decision.Play, gate.cue(Cue.Send))
    }

    @Test
    fun `haptics need their switch and a vibrator`() {
        settings = settings.copy(haptics = false)
        assertEquals(Skipped.HapticsOff, why(gate.haptic(Haptic.Confirm)))
        settings = FeedbackSettings()
        env.hasVibrator = false
        assertEquals(Skipped.NoVibrator, why(gate.haptic(Haptic.Confirm)))
    }

    @Test
    fun `the master off silences everything whatever else is on`() {
        settings = FeedbackSettings(master = false) // every other switch still on
        assertEquals(Skipped.MasterOff, why(gate.haptic(Haptic.Confirm)))
        assertEquals(Skipped.MasterOff, why(gate.haptic(Haptic.Error, preview = true)))
        for (cue in Cue.entries) assertEquals(cue.name, Skipped.MasterOff, why(gate.cue(cue)))
        assertEquals(Skipped.MasterOff, why(gate.cue(Cue.Request, preview = true)))
        assertEquals(false, settings.soundsOn)
        assertEquals(false, settings.hapticsOn)
        for (category in CueCategory.entries) assertEquals(false, settings.allows(category))
    }

    @Test
    fun `with the master on the switches below decide`() {
        settings = FeedbackSettings(sounds = false)
        assertEquals(Skipped.SoundsOff, why(gate.cue(Cue.Tap)))
        assertEquals(Decision.Play, gate.haptic(Haptic.Confirm)) // haptics are their own switch
        settings = FeedbackSettings(haptics = false)
        assertEquals(Skipped.HapticsOff, why(gate.haptic(Haptic.Confirm)))
        assertEquals(Decision.Play, gate.cue(Cue.Tap))
    }

    @Test
    fun `the gate knows only whether the app is active and has a vibrator, not the phone's own sound settings`() {
        assertEquals(Decision.Play, gate.cue(Cue.Tap))
        assertEquals(Decision.Play, gate.haptic(Haptic.Confirm))
        assertEquals(
            setOf("active", "hasVibrator"),
            FeedbackEnvironment::class.java.methods.map { it.name.removePrefix("get").replaceFirstChar(Char::lowercase) }.toSet(),
        )
    }

    @Test
    fun `nothing plays in the background or with the screen off`() {
        env.active = false
        assertEquals(Skipped.Inactive, why(gate.haptic(Haptic.Error)))
        assertEquals(Skipped.Inactive, why(gate.cue(Cue.Request)))
    }

    @Test
    fun `each kind of sound has its own switch`() {
        settings = FeedbackSettings(interfaceSounds = false)
        assertEquals(Skipped.CategoryOff, why(gate.cue(Cue.Tap)))
        assertEquals(Skipped.CategoryOff, why(gate.cue(Cue.Send)))
        assertEquals(Decision.Play, gate.cue(Cue.Request)) // requests are independent of interface sounds

        now += 1000
        settings = FeedbackSettings(requestSounds = false)
        assertEquals(Skipped.CategoryOff, why(gate.cue(Cue.Request)))
        assertEquals(Decision.Play, gate.cue(Cue.Attention))
        now += 1000
        settings = FeedbackSettings(alertSounds = false)
        assertEquals(Skipped.CategoryOff, why(gate.cue(Cue.Attention)))
        assertEquals(Decision.Play, gate.cue(Cue.AutoApproved))
        now += 1000
        settings = FeedbackSettings(autopilotSounds = false)
        assertEquals(Skipped.CategoryOff, why(gate.cue(Cue.AutoApproved)))
        assertEquals(Skipped.CategoryOff, why(gate.cue(Cue.AutoDenied)))
        assertEquals(Decision.Play, gate.cue(Cue.Tap))
    }

    @Test
    fun `a preview ignores the category switches and rate limits but not the master or sounds switch`() {
        settings = FeedbackSettings(interfaceSounds = false, requestSounds = false)
        assertEquals(Decision.Play, gate.cue(Cue.Request, preview = true))
        assertEquals(Decision.Play, gate.cue(Cue.Request, preview = true))
        settings = FeedbackSettings(sounds = false)
        assertEquals(Skipped.SoundsOff, why(gate.cue(Cue.Tap, preview = true)))
    }

    @Test
    fun `the same cue never stacks`() {
        assertEquals(Decision.Play, gate.cue(Cue.Tap))
        now += 30
        assertEquals(Skipped.Rate, why(gate.cue(Cue.Tap)))
        now += 40
        assertEquals(Decision.Play, gate.cue(Cue.Tap))
    }

    @Test
    fun `the same haptic is coalesced`() {
        assertEquals(Decision.Play, gate.haptic(Haptic.Tick))
        now += 20
        assertEquals(Skipped.Rate, why(gate.haptic(Haptic.Tick)))
        now += 60
        assertEquals(Decision.Play, gate.haptic(Haptic.Tick))
    }

    @Test
    fun `light haptics do not chatter across kinds`() {
        assertEquals(Decision.Play, gate.haptic(Haptic.Tick))
        now += 10
        assertEquals(Skipped.Rate, why(gate.haptic(Haptic.Select))) // inside the global gap
        now += 30
        assertEquals(Decision.Play, gate.haptic(Haptic.Select))
    }

    @Test
    fun `important haptics cut through light ones`() {
        assertEquals(Decision.Play, gate.haptic(Haptic.Tick))
        now += 5
        assertEquals(Decision.Play, gate.haptic(Haptic.Error))
        now += 5
        assertEquals(Skipped.Rate, why(gate.haptic(Haptic.Tick)))
    }

    @Test
    fun `a burst of light haptics is capped`() {
        var played = 0
        repeat(60) {
            if (gate.haptic(if (it % 2 == 0) Haptic.Tick else Haptic.Select) == Decision.Play) played++
            now += 30
        }
        // 60 events over 1.8 s, 12 per second at most.
        assertTrue("played $played", played <= FeedbackGate.LIGHT_HAPTICS_PER_WINDOW * 2)
        assertTrue(played > 0)
    }

    @Test
    fun `streams are capped and a more important cue still gets in`() {
        for (cue in listOf(Cue.Request, Cue.Done, Cue.UploadReady, Cue.Reconnected)) {
            assertEquals(cue.name, Decision.Play, gate.cue(cue))
            now += 30
        }
        assertEquals(Skipped.Streams, why(gate.cue(Cue.Tap)))
        assertEquals(Decision.Play, gate.cue(Cue.Attention))
        now += 2000
        assertEquals(Decision.Play, gate.cue(Cue.Tap))
    }

    @Test
    fun `the gate reads the environment a bounded number of times per decision`() {
        val counting = CountingEnv()
        val g = FeedbackGate({ settings }, counting, { now })
        g.cue(Cue.Tap)
        assertTrue("reads=${counting.reads}", counting.reads <= 2)
        counting.reads = 0
        now += 1000
        g.haptic(Haptic.Confirm)
        assertTrue("reads=${counting.reads}", counting.reads <= 2)
    }
}
