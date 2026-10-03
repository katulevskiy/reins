package dev.rewarden.android.ui

import dev.rewarden.android.ui.signin.AccountRules
import dev.rewarden.android.ui.signin.Strength
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AccountRulesTest {
    private val good = "correct horse battery"

    @Test
    fun `a new account needs an email, 12 characters, matching passwords and the Terms`() {
        assertEquals("Enter your email address.", AccountRules.createProblem("", good, good, true))
        assertEquals("Enter your email address.", AccountRules.createProblem("me@example", good, good, true))
        assertEquals(
            "The master password needs at least 12 characters.",
            AccountRules.createProblem("me@example.com", "elevenchars", "elevenchars", true),
        )
        assertEquals("The passwords don't match.", AccountRules.createProblem("me@example.com", good, "$good!", true))
        assertEquals("The passwords don't match.", AccountRules.createProblem("me@example.com", good, "", true))
        assertEquals("Accept the Terms to continue.", AccountRules.createProblem("me@example.com", good, good, false))
        assertNull(AccountRules.createProblem("me@example.com", good, good, true))
        assertNull(AccountRules.createProblem("  me@example.com ", "twelve chars", "twelve chars", true))
    }

    @Test
    fun `emails are checked loosely`() {
        assertTrue(AccountRules.isEmail("me@example.com"))
        assertTrue(AccountRules.isEmail("first.last+tag@sub.example.co.uk"))
        assertFalse(AccountRules.isEmail("me@@example.com"))
        assertFalse(AccountRules.isEmail("@example.com"))
        assertFalse(AccountRules.isEmail("me@.com"))
        assertFalse(AccountRules.isEmail("me@example."))
        assertFalse(AccountRules.isEmail("me @example.com"))
    }

    @Test
    fun `strength follows length, variety and repetition`() {
        assertEquals(Strength.Weak, AccountRules.strength(""))
        assertEquals(Strength.Weak, AccountRules.strength("Sh0rt!"))
        assertEquals(Strength.Weak, AccountRules.strength("aaaaaaaaaaaaaaaaaaaaaaaa"))
        assertEquals(Strength.Weak, AccountRules.strength("abababababab"))
        assertEquals(Strength.Fair, AccountRules.strength("elevenchars1"))
        assertEquals(Strength.Fair, AccountRules.strength("onlylowercaseletters".take(15)))
        assertEquals(Strength.Strong, AccountRules.strength("Tr0ub4dor&3xyzQ!"))
        assertEquals(Strength.Strong, AccountRules.strength("correct horse battery staple"))
    }

    @Test
    fun `the hint says what to do`() {
        assertEquals("Use at least 12 characters.", AccountRules.strengthHint("short"))
        assertEquals("Too repetitive. Mix in other characters.", AccountRules.strengthHint("abababababab"))
        assertEquals("Hard to guess. Remember it, or write it down.", AccountRules.strengthHint("correct horse battery staple"))
    }

    @Test
    fun `server addresses get https when they have no scheme`() {
        assertEquals("https://app.reins2fa.com", AccountRules.serverUrl("https://app.reins2fa.com/"))
        assertEquals("https://reins.example.com", AccountRules.serverUrl(" reins.example.com "))
        assertEquals("http://10.0.2.2:8000", AccountRules.serverUrl("http://10.0.2.2:8000"))
        assertEquals("https://example.com/reins", AccountRules.serverUrl("HTTPS://example.com/reins"))
        assertNull(AccountRules.serverUrl("https://"))
        assertNull(AccountRules.serverUrl(""))
        assertNull(AccountRules.serverUrl("ftp://example.com"))
        assertNull(AccountRules.serverUrl("https://exa mple.com"))
    }

    @Test
    fun `the MCP address is the server's mcp endpoint`() {
        assertEquals("https://app.reins2fa.com/mcp", AccountRules.mcpUrl("https://app.reins2fa.com"))
        assertEquals("https://reins.example.com/mcp", AccountRules.mcpUrl("https://reins.example.com/"))
        assertEquals("app.reins2fa.com", AccountRules.displayHost("https://app.reins2fa.com/"))
    }
}
