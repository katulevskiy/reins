package dev.reins.android.ui

import dev.reins.android.design.serviceName
import dev.reins.android.ui.mcp.isMcpRedirect
import dev.reins.android.ui.services.isSignInPage
import dev.reins.android.ui.services.isXRedirect
import dev.reins.android.ui.services.needsAgain
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class XSignInLogicTest {
    @Test
    fun `only the app's own X redirect finishes an X sign-in`() {
        assertTrue(isXRedirect("com.reins2fa.app://x-oauth?state=s&code=c"))
        assertTrue(isXRedirect("com.reins2fa.app://x-oauth?error=access_denied&state=s"))
        assertTrue(isXRedirect("com.reins2fa.app://x-oauth/?state=s&code=c"))
        assertFalse(isXRedirect(null))
        assertFalse(isXRedirect(""))
        assertFalse(isXRedirect("com.reins2fa.app://mcp-oauth?state=s&code=c"))
        assertFalse(isXRedirect("com.reins2fa.app://other?code=c"))
        assertFalse(isXRedirect("com.reins2fa.app://x-oauth.evil.com?state=s&code=c"))
        assertFalse(isXRedirect("com.reins2fa.app://evil.com@x-oauth?code=c"))
        assertFalse(isXRedirect("com.reins2fa.app://x-oauth:443?code=c"))
        assertFalse(isXRedirect("https://x-oauth?state=s&code=c"))
        assertFalse(isXRedirect("https://evil.example.com/x-oauth?state=s&code=c"))
        assertFalse(isXRedirect("evil.app://x-oauth?state=s&code=c"))
        assertFalse(isXRedirect("com.reins2fa.app://x-oauth?code=" + "a".repeat(9_000)))
        assertFalse(isXRedirect("not a uri at all ::"))
    }

    @Test
    fun `the MCP and X redirects never stand in for each other`() {
        assertFalse(isMcpRedirect("com.reins2fa.app://x-oauth?state=s&code=c"))
        assertTrue(isMcpRedirect("com.reins2fa.app://mcp-oauth?state=s&code=c"))
    }

    @Test
    fun `only an https page is opened for an X sign-in`() {
        assertTrue(isSignInPage("https://x.com/i/oauth2/authorize?client_id=a&state=s"))
        assertFalse(isSignInPage("http://x.com/i/oauth2/authorize"))
        assertFalse(isSignInPage("intent://evil#Intent;end"))
        assertFalse(isSignInPage("javascript:alert(1)"))
        assertFalse(isSignInPage("file:///sdcard/page.html"))
        assertFalse(isSignInPage("https:///no-host"))
        assertFalse(isSignInPage("not a url ::"))
    }

    @Test
    fun `X has its own words`() {
        assertEquals("X", serviceName("x"))
        assertEquals("Signed out", needsAgain("oauth"))
    }
}
