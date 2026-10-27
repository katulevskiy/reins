package dev.reins.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AgeSignal
import dev.reins.android.platform.AgeSignals
import dev.reins.android.platform.AgeSignalsProvider
import dev.reins.android.platform.Foreground
import dev.reins.android.platform.PlayAgeSignals
import dev.reins.android.platform.ageSignalOf
import dev.reins.android.platform.toInput
import dev.reins.core.AgeSignalInput
import dev.reins.core.AgeSignalPlatform
import dev.reins.core.CircleRole
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The age seam of a guardianship's minor mark (Decisions v2 P7): a hint at most, and no signal never blocks. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class AgeSignalsTest : FlowHarness() {
    private val default = AgeSignalsProvider.factory

    @After
    fun restore() {
        AgeSignalsProvider.factory = default
    }

    private fun proposed() {
        core.circlePeopleList = listOf(
            TestData.circlePerson("g1", "mom@example.com", role = CircleRole.GUARDIAN, mine = true, scopeLine = "Purchases"),
        )
        core.guardianWards["g1"] = TestData.wardView(minorProposed = true)
        Foreground.autoPopup = false
        launch()
        tap("tabSettings")
        tap("circlesRow")
        tap("circleOpen:g1")
        awaitTag("guardianMinorCard")
    }

    @Test
    fun unavailableNeverBlocksSetup() {
        AgeSignalsProvider.factory = { AgeSignals { AgeSignal.Unavailable } }
        proposed()
        tap("guardianMinorAccept")
        awaitCore { core.guardianMinorAccepts.isNotEmpty() }
        // No signal: the ward's own acceptance counts.
        assertEquals(listOf("g1" to AgeSignalInput.Unavailable), core.guardianMinorAccepts.toList())
        awaitTag("guardianMinor")
        assertFalse(has("guardianMinorCard"))
    }

    @Test
    fun aMinorSignalGoesWithTheAcceptance() {
        AgeSignalsProvider.factory = { AgeSignals { AgeSignal.Minor(15) } }
        proposed()
        tap("guardianMinorAccept")
        awaitCore { core.guardianMinorAccepts.isNotEmpty() }
        assertEquals(AgeSignalInput.Minor(15u, AgeSignalPlatform.GOOGLE_PLAY), core.guardianMinorAccepts.single().second)
    }

    @Test
    fun anAdultSignalIsRefusedAndSaysWhy() {
        AgeSignalsProvider.factory = { AgeSignals { AgeSignal.Adult } }
        proposed()
        tap("guardianMinorAccept")
        awaitTag("guardianError")
        assertTrue(core.guardianMinorAccepts.isEmpty())
        awaitText("This account isn't a minor's.")
    }

    @Test
    fun ignoreLeavesItUnanswered() {
        proposed()
        tap("guardianMinorIgnore")
        awaitGone("guardianMinorCard")
        assertTrue(core.guardianMinorAccepts.isEmpty())
    }

    @Test
    fun playRangesMapToSignals() {
        assertEquals(AgeSignal.Minor(15), ageSignalOf(13, 15))
        assertEquals(AgeSignal.Minor(17), ageSignalOf(null, 17))
        assertEquals(AgeSignal.Adult, ageSignalOf(18, null))
        assertEquals(AgeSignal.Adult, ageSignalOf(21, 150))
        assertEquals(AgeSignal.Unavailable, ageSignalOf(null, null))
        assertEquals(AgeSignal.Unavailable, ageSignalOf(16, null))
        assertEquals(AgeSignalInput.Unavailable, AgeSignal.Unavailable.toInput())
        assertEquals(AgeSignalInput.Adult, AgeSignal.Adult.toInput())
        assertEquals(AgeSignalInput.Minor(null, AgeSignalPlatform.GOOGLE_PLAY), AgeSignal.Minor(null).toInput())
    }

    @Test
    fun withoutPlayItIsUnavailable() {
        // Robolectric has no Play Store: an error, or no answer in time, is no signal (never a crash, never a wait).
        val signal = runBlocking { PlayAgeSignals(context, timeoutMs = 500).check() }
        assertEquals(AgeSignal.Unavailable, signal)
    }
}
