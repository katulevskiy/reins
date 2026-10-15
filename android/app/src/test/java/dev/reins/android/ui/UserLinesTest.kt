package dev.reins.android.ui

import dev.reins.android.platform.GoogleAuthorizer
import dev.reins.android.ui.common.userMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** What the user reads is a short line; developer setup and failure detail go to the log, never the screen. */
class UserLinesTest {
    private val developerWords = listOf("SHA-1", "OAuth", "Google Cloud", "package", "API", "Exception", "code ")

    @Test
    fun `google setup problems read as a short line`() {
        val line = GoogleAuthorizer.unavailable("gmail")
        assertEquals("Gmail can't be connected from this version of Reins yet.", line)
        developerWords.forEach { assertFalse("\"$line\" says $it", line.contains(it)) }
        // The fix is still written down, for the log.
        assertTrue(GoogleAuthorizer.SETUP_DETAIL.contains("SHA-1"))
    }

    @Test
    fun `an unexpected error never names its class`() {
        val line = IllegalStateException("boom").userMessage()
        assertEquals("Something went wrong. Try again.", line)
        assertFalse(line.contains("IllegalStateException"))
    }
}
