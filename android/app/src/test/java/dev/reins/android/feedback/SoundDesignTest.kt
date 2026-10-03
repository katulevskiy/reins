package dev.reins.android.feedback

import dev.reins.android.platform.AppNotifier
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The cue table, the event vocabulary, and the files they name. */
class SoundDesignTest {
    private val raw = File("src/main/res/raw")

    @Test
    fun `every cue has a spec and its file is in the app`() {
        assertEquals(Cue.entries.size, CueTable.all.size)
        for (spec in CueTable.all) {
            assertTrue(spec.resource, spec.resource.matches(Regex("fx_[a-z_]+")))
            assertTrue(spec.gain in 0.1f..1f)
            assertTrue("${spec.cue} -> ${spec.resource}", File(raw, spec.resource + ".wav").isFile)
        }
    }

    @Test
    fun `only the quiet echoes and lockdown borrow another cue's file`() {
        val shared = CueTable.all.groupBy { it.resource }.filterValues { it.size > 1 }.values.flatten().map { it.cue }.toSet()
        assertEquals(setOf(Cue.Send, Cue.AutoApproved, Cue.Close, Cue.AutoDenied, Cue.Attention, Cue.Lockdown), shared)
        assertEquals(CueTable.all.map { it.resource }.toSet().size, CueTable.resources.size)
    }

    @Test
    fun `nothing unused ships`() {
        val shipped = raw.listFiles { f -> f.name.startsWith("fx_") }!!.map { it.name.removeSuffix(".wav") }.toSet()
        assertEquals(CueTable.resources.toSet(), shipped)
    }

    @Test
    fun `the chimes govern by their own switches`() {
        assertEquals("fx_chime_request", CueTable.spec(Cue.Request).resource)
        assertEquals("fx_chime_attention", CueTable.spec(Cue.Attention).resource)
        assertEquals(CueCategory.Requests, CueTable.spec(Cue.Request).category)
        assertEquals(CueCategory.Alerts, CueTable.spec(Cue.Attention).category)
        assertEquals(CueCategory.Autopilot, CueTable.spec(Cue.AutoApproved).category)
        assertEquals(CueCategory.Autopilot, CueTable.spec(Cue.AutoDenied).category)
    }

    @Test
    fun `what Autopilot decides is quieter and lighter than what the user does`() {
        assertTrue(CueTable.spec(Cue.AutoApproved).gain < CueTable.spec(Cue.Send).gain / 2)
        assertTrue(CueTable.spec(Cue.AutoDenied).gain < CueTable.spec(Cue.Close).gain)
        assertEquals(0, CueTable.spec(Cue.AutoApproved).priority)
        assertEquals(0, CueTable.spec(Cue.AutoDenied).priority)
    }

    @Test
    fun `every event has a sound or a haptic and the Autopilot events exist`() {
        for (e in Event.entries) assertTrue(e.name, e.haptic != null || e.cue != null)
        for (name in listOf("AutoApproved", "AutoDenied", "AutopilotOn", "AutopilotOff", "BypassOn", "BypassOff", "LockdownOn", "LockdownOff")) {
            assertNotNull(name, Event.entries.firstOrNull { it.name == name })
        }
        assertEquals(Event.AutopilotOn, Event.autopilotModeChanged(moreAutonomy = true))
        assertEquals(Event.AutopilotOff, Event.autopilotModeChanged(moreAutonomy = false))
        assertEquals(Event.ToggleOn, Event.toggle(true))
        assertEquals(Event.Close, Event.expand(false))
    }

    @Test
    fun `the main moments sound as designed`() {
        assertEquals(Haptic.Attention to Cue.Request, Event.RequestArrived.haptic to Event.RequestArrived.cue)
        assertEquals(Haptic.Confirm to Cue.Send, Event.Approved.haptic to Event.Approved.cue)
        assertEquals(Haptic.Deny to Cue.Close, Event.Denied.haptic to Event.Denied.cue)
        assertEquals(Haptic.Success to Cue.Done, Event.GrantCreated.haptic to Event.GrantCreated.cue)
        assertEquals(Haptic.Heavy to Cue.Delete, Event.Revoked.haptic to Event.Revoked.cue)
        assertEquals(Haptic.Success to Cue.UploadReady, Event.UploadApproved.haptic to Event.UploadApproved.cue)
        assertEquals(Haptic.Surge to Cue.Surge, Event.BypassOn.haptic to Event.BypassOn.cue)
    }

    @Test
    fun `the notification channels play the same chimes as the app`() {
        assertEquals(CueTable.spec(Cue.Request).resource, AppNotifier.Kind.Approvals.sound)
        assertEquals(CueCategory.Requests, AppNotifier.Kind.Approvals.category)
        for (kind in AppNotifier.Kind.entries) {
            val sound = kind.sound ?: continue // Autopilot's quiet channels never sound
            assertTrue(sound, File(raw, "$sound.wav").isFile)
            if (kind.category == CueCategory.Autopilot) {
                // What Autopilot decides by itself sounds like its in-app cue.
                assertEquals(CueTable.spec(Cue.AutoDenied).resource, sound)
            } else {
                assertTrue(sound in setOf("fx_chime_request", "fx_chime_attention"))
            }
        }
        assertTrue(AppNotifier.CHANNEL_VERSION >= 2) // the unversioned channels had no chime
    }

    @Test
    fun `the detent climbs and stays in range`() {
        var last = -100
        for (step in 0..4) {
            val st = DetentLadder.semitones(step)
            assertTrue("step $step", st > last)
            last = st
        }
        for (step in -5..60) assertTrue(DetentLadder.rate(step) in DetentLadder.MIN_RATE..DetentLadder.MAX_RATE)
        assertTrue(DetentLadder.rate(5) > DetentLadder.rate(4)) // the next octave continues upward
        assertEquals(DetentLadder.rate(40), DetentLadder.rate(80), 0f) // the top holds
    }
}
