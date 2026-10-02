package dev.rewarden.android.push

import dev.rewarden.android.sync.ForegroundSync
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PushTest {
    @Test
    fun `only the exact payload shape is accepted`() {
        assertEquals(PushPayload("req", "abc_DEF-123"), PushPayload.parse(mapOf("t" to "req", "id" to "abc_DEF-123")))
        assertEquals(PushPayload("pair", "x"), PushPayload.parse(mapOf("t" to "pair", "id" to "x")))
        assertEquals(PushPayload("replaced", "x"), PushPayload.parse(mapOf("t" to "replaced", "id" to "x")))
        assertNull(PushPayload.parse(mapOf("t" to "other", "id" to "x")))
        assertNull(PushPayload.parse(mapOf("t" to "req")))
        assertNull(PushPayload.parse(mapOf("id" to "x")))
        assertNull(PushPayload.parse(mapOf("t" to "req", "id" to "../etc")))
        assertNull(PushPayload.parse(mapOf("t" to "req", "id" to "a".repeat(129))))
        assertNull(PushPayload.parse(mapOf("t" to "req", "id" to "")))
    }

    @Test
    fun `the sync backoff doubles up to thirty seconds`() {
        assertEquals(listOf(1_000L, 2_000L, 4_000L, 8_000L, 16_000L, 30_000L, 30_000L),
            (0..6).map(ForegroundSync::backoffMillis))
    }
}
