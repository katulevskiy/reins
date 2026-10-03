package dev.rewarden.android.ui

import dev.rewarden.android.ui.common.untrusted
import org.junit.Assert.assertEquals
import org.junit.Test

class UntrustedTest {
    private fun c(code: Int) = code.toChar().toString()

    @Test
    fun `bidi controls are removed`() {
        assertEquals("evil.com", untrusted(c(0x202E) + "evil.com" + c(0x202C)))
        assertEquals("abc", untrusted("a" + c(0x202E) + "b" + c(0x2066) + "c" + c(0x200F)))
        assertEquals("x", untrusted(c(0x061C) + "x" + c(0x2069)))
    }

    @Test
    fun `control characters become spaces and edges are trimmed`() {
        assertEquals("a b", untrusted("a" + c(0) + "b"))
        assertEquals("a b", untrusted("a" + c(0x2028) + "b"))
        assertEquals("line1\nline2", untrusted("  line1\nline2 " + c(7)))
    }
}
