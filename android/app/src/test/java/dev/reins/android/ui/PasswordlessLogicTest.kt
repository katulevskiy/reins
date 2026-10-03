package dev.rewarden.android.ui

import dev.rewarden.android.platform.AppNotifier
import dev.rewarden.android.push.PushPayload
import dev.rewarden.android.ui.nav.DeepLink
import dev.rewarden.android.ui.nav.SheetTarget
import dev.rewarden.android.ui.settings.recoveryCodeLines
import dev.rewarden.android.ui.signin.AccountRules
import dev.rewarden.android.ui.signin.awaitJoin
import dev.rewarden.core.CoreException
import dev.rewarden.core.JoinProgress
import dev.rewarden.core.PendingItem
import dev.rewarden.core.PendingKind
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class PasswordlessLogicTest {
    @Test
    fun `only the app's own sign-in address counts as the callback`() {
        assertTrue(AccountRules.isSsoCallback("com.reins2fa.app://sso-callback?code=abc&state=xyz"))
        assertTrue(AccountRules.isSsoCallback("com.reins2fa.app://sso-callback?error=access_denied&state=xyz"))
        assertFalse(AccountRules.isSsoCallback("com.reins2fa.app://mcp-oauth?code=abc&state=xyz"))
        assertFalse(AccountRules.isSsoCallback("https://sso-callback/?code=abc"))
        assertFalse(AccountRules.isSsoCallback("com.reins2fa.app://sso-callback.evil.example?code=abc"))
        assertFalse(AccountRules.isSsoCallback("com.reins2fa.app://user@sso-callback?code=abc"))
        assertFalse(AccountRules.isSsoCallback("not a url at all"))
        assertFalse(AccountRules.isSsoCallback(null))
        assertFalse(AccountRules.isSsoCallback("com.reins2fa.app://sso-callback?code=" + "a".repeat(9_000)))
    }

    @Test
    fun `the hosted server is recognised however it is typed`() {
        val hosted = "https://app.reins2fa.com"
        assertTrue(AccountRules.isDefaultServer("app.reins2fa.com", hosted))
        assertTrue(AccountRules.isDefaultServer("HTTPS://App.Reins2fa.com/", hosted))
        assertFalse(AccountRules.isDefaultServer("reins.example.com", hosted))
        assertFalse(AccountRules.isDefaultServer("https://", hosted))
    }

    @Test
    fun `the poll asks every interval until the other phone answers`() = runTest {
        val answers = ArrayDeque(listOf(JoinProgress.WAITING, JoinProgress.WAITING, JoinProgress.JOINED))
        var polls = 0
        val result = awaitJoin({ polls++; answers.removeFirst() }, { delay(2_000) })
        assertEquals(JoinProgress.JOINED, result)
        assertEquals(3, polls)
        assertEquals(6_000L, currentTime)
    }

    @Test
    fun `a network hiccup keeps waiting, a refusal or expiry ends it`() = runTest {
        var polls = 0
        val result = awaitJoin({
            polls++
            if (polls == 1) throw CoreException.Network("offline")
            JoinProgress.DENIED
        }, {})
        assertEquals(JoinProgress.DENIED, result)
        assertEquals(2, polls)
        assertEquals(JoinProgress.EXPIRED, awaitJoin({ JoinProgress.EXPIRED }, {}))
    }

    @Test
    fun `other failures end the wait`() = runTest {
        val failure = runCatching { awaitJoin({ throw CoreException.NotFound() }, {}) }.exceptionOrNull()
        assertTrue(failure is CoreException.NotFound)
    }

    @Test
    fun `the recovery code is shown four groups to a line`() {
        val code = "ABCD-EFGH-IJKL-MNOP-QRST-UVWX-YZ23-4567-ABCD-EFGH-IJKL-MNOP-QRST"
        assertEquals(
            listOf("ABCD EFGH IJKL MNOP", "QRST UVWX YZ23 4567", "ABCD EFGH IJKL MNOP", "QRST"),
            recoveryCodeLines(code),
        )
    }

    @Test
    fun `a join request opens its own sheet from a notification and a push`() {
        val link = DeepLink.parse(AppNotifier.ACTION_OPEN_ITEM, "join", "join_0123456789", AppNotifier.ACTION_OPEN_ITEM)
        assertEquals(DeepLink(PendingKind.JOIN, "join_0123456789"), link)
        val item = PendingItem(PendingKind.JOIN, "join_0123456789", "", "", 0, "", "Pixel 9", "join", 1u, "", null, null, "", "", null)
        assertEquals(SheetTarget.Join("join_0123456789"), link!!.resolve(listOf(item)))
        assertNull(DeepLink(PendingKind.JOIN, "other").resolve(listOf(item)))
        assertEquals(SheetTarget.Join("join_0123456789"), item.toTarget())
        assertEquals(PushPayload("join", "join_0123456789"), PushPayload.parse(mapOf("t" to "join", "id" to "join_0123456789")))
    }
}
