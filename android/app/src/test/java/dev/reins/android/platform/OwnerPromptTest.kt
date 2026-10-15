package dev.reins.android.platform

import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The core's owner check over the phone's prompt: only a pass now confirms, with the core's reason shown. */
class OwnerPromptTest {
    private fun prompt(result: () -> AuthResult, asked: MutableList<Pair<String, String>> = mutableListOf()) =
        OwnerPrompt({ title, subtitle -> asked += title to subtitle; result() }, "Turn on Overnight for every AI", EmptyCoroutineContext)

    @Test
    fun `a pass confirms, with the core's reason as the subtitle`() = runBlocking {
        val asked = mutableListOf<Pair<String, String>>()
        val owner = prompt({ AuthResult.Success }, asked)
        assertTrue(owner.confirm("Start Overnight for 8 h"))
        assertEquals(listOf("Turn on Overnight for every AI" to "Start Overnight for 8 h"), asked)
        assertEquals(AuthResult.Success, owner.last)
    }

    @Test
    fun `a cancel, no screen lock, or an error refuses`() = runBlocking {
        val cancelled = prompt({ AuthResult.Cancelled })
        assertFalse(cancelled.confirm("Start Overnight for 1 h"))
        assertEquals(AuthResult.Cancelled, cancelled.last)
        val none = prompt({ AuthResult.Unavailable })
        assertFalse(none.confirm("Start Overnight for 1 h"))
        assertEquals(AuthResult.Unavailable, none.last)
        val broken = prompt({ throw IllegalStateException("no activity") })
        assertFalse(broken.confirm("Start Overnight for 1 h"))
        assertEquals(AuthResult.Cancelled, broken.last)
    }
}
