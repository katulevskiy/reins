package dev.rewarden.android.feedback

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TapFallbackTest {
    private var now = 1_000L
    private val claims = ClaimTracker { now }

    @Test
    fun `an explicit answer around the press keeps the default tap away`() {
        val released = now
        assertFalse(claims.cueClaimed(released))
        assertFalse(claims.hapticClaimed(released))
        now += 5
        claims.claimCue() // e.g. a sheet opening with its Open cue
        now += ClaimTracker.DEFER_MS
        assertTrue(claims.cueClaimed(released))
        assertFalse("the tap's haptic stays", claims.hapticClaimed(released))
    }

    @Test
    fun `feedback just before the release counts, older feedback does not`() {
        claims.claimHaptic()
        now += ClaimTracker.WINDOW_BEFORE_MS - 1
        assertTrue(claims.hapticClaimed(now))
        now += 2
        assertFalse(claims.hapticClaimed(now))
    }

    @Test
    fun `a close right after a cue or a choice stays quiet`() {
        claims.claimCue()
        now += 100
        assertTrue(claims.cueWithin(250))
        now += 200
        assertFalse(claims.cueWithin(250))
        claims.quiet()
        now += 100
        assertTrue(claims.cueWithin(250))
    }
}
