package dev.reins.android.push

import dev.reins.android.sync.ForegroundSync
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
        // The account's tag, when the server names it: exactly 16 hex digits.
        assertEquals(PushPayload("req", "x", "0123456789abcdef"), PushPayload.parse(mapOf("t" to "req", "id" to "x", "p" to "0123456789abcdef")))
        assertNull(PushPayload.parse(mapOf("t" to "req", "id" to "x", "p" to "0123")))
        assertNull(PushPayload.parse(mapOf("t" to "req", "id" to "x", "p" to "0123456789ABCDEF")))
    }

    @Test
    fun `circle pushes carry a kind of news and an id`() {
        assertEquals(PushPayload("circle", "review:rv_1-a"), PushPayload.parse(mapOf("t" to "circle", "id" to "review:rv_1-a")))
        assertEquals(PushPayload("circle", "alert:link-1"), PushPayload.parse(mapOf("t" to "circle", "id" to "alert:link-1")))
        assertEquals(PushPayload("circle", "news:x"), PushPayload.parse(mapOf("t" to "circle", "id" to "news:x")))
        assertNull(PushPayload.parse(mapOf("t" to "circle", "id" to "other:x")))
        assertNull(PushPayload.parse(mapOf("t" to "circle", "id" to "review:")))
        assertNull(PushPayload.parse(mapOf("t" to "circle", "id" to "review:../x")))
        assertNull(PushPayload.parse(mapOf("t" to "circle", "id" to "plain")))
        // Only a Circle's ids have a colon.
        assertNull(PushPayload.parse(mapOf("t" to "req", "id" to "review:x")))
    }

    @Test
    fun `the sync backoff doubles up to thirty seconds`() {
        assertEquals(listOf(1_000L, 2_000L, 4_000L, 8_000L, 16_000L, 30_000L, 30_000L),
            (0..6).map(ForegroundSync::backoffMillis))
    }
}
