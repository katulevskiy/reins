package dev.reins.android.feedback

import kotlin.math.log10
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FeedbackSettingsTest {
    @Test
    fun `everything defaults on at half volume`() {
        val d = FeedbackSettings()
        assertTrue(d.master && d.sounds && d.interfaceSounds && d.requestSounds && d.autopilotSounds && d.alertSounds && d.haptics)
        assertEquals(HapticStrength.Standard, d.strength)
        assertEquals(0.5f, d.volume, 0f)
        assertEquals(1f, d.gain, 1e-6f)
    }

    @Test
    fun `settings round-trip through their stored form`() {
        val s = FeedbackSettings(
            master = false, sounds = false, interfaceSounds = false, requestSounds = false, autopilotSounds = false,
            alertSounds = false, volume = 0.4f, haptics = false, strength = HapticStrength.Strong,
        )
        val stored = FeedbackSettingsCodec.write(s)
        assertEquals(s, FeedbackSettingsCodec.read { stored[it] })
        assertEquals(FeedbackSettingsCodec.VERSION, stored[FeedbackSettingsCodec.VERSION_KEY])
    }

    @Test
    fun `missing or malformed values read as the defaults`() {
        assertEquals(FeedbackSettings(), FeedbackSettingsCodec.read { null })
        val bad = mapOf<String, Any>("feedback.volume" to Float.NaN, "feedback.strength" to "Thunder", "feedback.sounds" to "yes")
        assertEquals(FeedbackSettings(), FeedbackSettingsCodec.read { bad[it] })
        val loud = mapOf<String, Any>("feedback.volume" to 9f)
        assertEquals(1f, FeedbackSettingsCodec.read { loud[it] }.volume, 0f)
    }

    @Test
    fun `category switches are independent under the master`() {
        val s = FeedbackSettings(requestSounds = false)
        assertEquals(false, s.allows(CueCategory.Requests))
        assertEquals(true, s.allows(CueCategory.Alerts))
        assertEquals(true, s.allows(CueCategory.Autopilot))
        assertEquals(true, s.allows(CueCategory.Interface))
        assertEquals(false, FeedbackSettings(sounds = false).allows(CueCategory.Interface))
        for (category in CueCategory.entries) assertEquals(false, FeedbackSettings(master = false).allows(category))
        assertEquals(false, FeedbackSettings(master = false).hapticsOn)
        assertEquals(false, FeedbackSettings(haptics = false).hapticsOn)
        assertEquals(true, FeedbackSettings().soundsOn && FeedbackSettings().hapticsOn)
    }

    @Test
    fun `the full slider is six decibels louder than the default`() {
        val max = FeedbackSettings(volume = 1f).gain
        assertEquals(FeedbackSettings.MAX_GAIN, max, 1e-6f)
        assertEquals(6.02, 20 * log10(max.toDouble()), 0.01)
        assertEquals(0f, FeedbackSettings(volume = 0f).gain, 0f)
        assertEquals(FeedbackSettings.MAX_GAIN, FeedbackSettings(volume = 7f).gain, 1e-6f) // clamped
    }

    @Test
    fun `the volume curve is monotone, continuous, and squared below the default`() {
        var last = -1f
        for (i in 0..100) {
            val g = FeedbackSettings.gainFor(i / 100f)
            assertTrue("$i%", g > last || i == 0)
            last = g
        }
        assertEquals(0.25f, FeedbackSettings.gainFor(0.25f), 1e-6f) // slider 25% = -12 dB
        assertEquals(FeedbackSettings.gainFor(0.5f), FeedbackSettings.gainFor(0.5001f), 1e-3f) // no jump at the join
        val step = 20 * log10(FeedbackSettings.gainFor(0.75f).toDouble() / FeedbackSettings.gainFor(0.5f))
        assertEquals(3.01, step, 0.01) // equal dB steps above the default
    }

    @Test
    fun `SoundPool volume never exceeds one and the default plays the files at half`() {
        val spec = CueTable.spec(Cue.Tap)
        assertEquals(0.5f, CueTable.volume(spec, FeedbackSettings().gain), 1e-6f)
        assertEquals(1f, CueTable.volume(spec, FeedbackSettings(volume = 1f).gain), 1e-6f)
        for (s in CueTable.all) for (i in 0..100) assertTrue(CueTable.volume(s, FeedbackSettings.gainFor(i / 100f)) <= 1f)
        val fast = CueTable.spec(Cue.FastOn)
        assertEquals(0.5f * fast.gain, CueTable.volume(fast, FeedbackSettings().gain), 1e-6f) // a trim is kept
    }

    @Test
    fun `the master round-trips and defaults on`() {
        val stored = FeedbackSettingsCodec.write(FeedbackSettings(master = false))
        assertEquals(false, FeedbackSettingsCodec.read { stored[it] }.master)
        assertEquals(true, FeedbackSettingsCodec.read { null }.master)
    }
}
