package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.state.AppState
import dev.reins.android.ui.nav.SheetTarget
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Only what waits for the owner pops up; what a mode or Autopilot decided closes and stays gone. */
class AskGateTest {
    @Test
    fun `the next ask is the first waiting one that has not popped up`() {
        val waiting = listOf(TestData.pending("r1"), TestData.pending("r2"))
        assertEquals("r1", AskGate.next(waiting, emptySet())?.id)
        assertEquals("r2", AskGate.next(waiting, setOf("r1"))?.id)
        assertNull(AskGate.next(waiting, setOf("r1", "r2")))
        assertNull("nothing waits for the owner: nothing pops up", AskGate.next(emptyList(), emptySet()))
    }

    @Test
    fun `an ask open on a request a mode decided closes, any other stays`() {
        assertTrue(AskGate.closes(SheetTarget.Approval("r1"), "r1"))
        assertTrue("an upload a bypass approved too", AskGate.closes(SheetTarget.Upload("b1"), "b1"))
        assertFalse(AskGate.closes(SheetTarget.Approval("r2"), "r1"))
        assertFalse(AskGate.closes(null, "r1"))
    }

    @Test
    fun `a request decided by a mode leaves the list and a list read before the decision does not bring it back`() {
        val state = AppState()
        state.setPending(listOf(TestData.pending("r1"), TestData.pending("r2")))
        state.markAutoDecided("r1")
        assertEquals(listOf("r2"), state.pending.value.map { it.id })
        // A refresh that read the core's list a moment before the bypass approved r1.
        state.setPending(listOf(TestData.pending("r1"), TestData.pending("r2")))
        assertEquals(listOf("r2"), state.pending.value.map { it.id })
        assertNull("never pops up again", AskGate.next(state.pending.value, setOf("r2")))
    }
}
