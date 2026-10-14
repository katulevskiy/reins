package dev.reins.android.ui

import dev.reins.android.ui.grants.grantOrigin
import org.junit.Assert.assertEquals
import org.junit.Test

class GrantOriginTest {
    @Test
    fun `a grant says where it came from, a bypass one that it ends with the bypass`() {
        assertEquals("Approved by Bypass; ends with it", grantOrigin("bypass", "Claude"))
        assertEquals("Approved by Bypass; ends with it", grantOrigin("bypass:c1", "Claude"))
        assertEquals("Your starting rule, when you connected Claude", grantOrigin("starter", "Claude"))
        assertEquals("Chosen while approving a request", grantOrigin("approval", "Claude"))
        assertEquals("Chosen while approving a request", grantOrigin("bypassing", "Claude"))
    }
}
