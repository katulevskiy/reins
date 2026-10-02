package dev.rewarden.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class GitHostsFlowTest : FlowHarness() {
    private fun openService(id: String) {
        launch()
        tap("integrations")
        tap("service:$id")
    }

    private fun opened(): Intent {
        val intent = nextStarted()
        assertNotNull(intent)
        assertEquals(Intent.ACTION_VIEW, intent!!.action)
        return intent
    }

    @Test
    fun theIntegrationsListHasTheNewGitHosts() {
        launch()
        tap("integrations")
        awaitTag("service:gitlab")
        awaitTag("service:codeberg")
        awaitTag("service:bitbucket")
    }

    @Test
    fun gitlabOpensItsTokenPageFilledInAndTakesAPastedToken() {
        openService("gitlab")
        tap("openTokenPage")
        val url = opened().dataString!!
        assertTrue(url, Regex("https://gitlab\\.com/-/user_settings/personal_access_tokens\\?name=Rewarden-\\d{6}&scopes=read_api,read_repository,write_repository").matches(url))
        tap("pasteManually")
        rule.onNodeWithTag("connectSecret").assertIsNotEnabled()
        rule.onNodeWithTag("secret").performTextReplacement("glpat-abcdefghijklmnopqrst")
        tap("connectSecret")
        awaitCore { core.tokensAdded.isNotEmpty() }
        assertEquals("gitlab" to "glpat-abcdefghijklmnopqrst", core.tokensAdded.single())
        awaitTag("account:octo-cat")
    }

    @Test
    fun aGitlabTokenCopiedOnItsPageIsPickedUpOnReturn() {
        openService("gitlab")
        tap("openTokenPage")
        opened()
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("token", "glpat-Zx9_abcdefghijklmnopqr"))
        scenario!!.moveToState(Lifecycle.State.STARTED)
        scenario!!.moveToState(Lifecycle.State.RESUMED)
        awaitCore { core.tokensAdded.isNotEmpty() }
        assertEquals("gitlab" to "glpat-Zx9_abcdefghijklmnopqr", core.tokensAdded.single())
    }

    @Test
    fun codebergOpensItsApplicationsPage() {
        openService("codeberg")
        awaitText("read:user", substring = true)
        tap("openTokenPage")
        assertEquals("https://codeberg.org/user/settings/applications", opened().dataString)
        tap("pasteManually")
        rule.onNodeWithTag("secret").performTextReplacement("0123456789abcdef0123456789abcdef01234567")
        tap("connectSecret")
        awaitCore { core.tokensAdded.isNotEmpty() }
        assertEquals("codeberg" to "0123456789abcdef0123456789abcdef01234567", core.tokensAdded.single())
    }

    @Test
    fun bitbucketExplainsEmailColonTokenAndSendsItWhole() {
        openService("bitbucket")
        awaitText("email:token", substring = true)
        // The field is there at once: the email has to be typed anyway.
        awaitTag("secret")
        tap("openTokenPage")
        assertEquals("https://id.atlassian.com/manage-profile/security/api-tokens", opened().dataString)
        rule.onNodeWithTag("secret").performTextReplacement("me@example.com:ATATT3xFfGF0abc")
        tap("connectSecret")
        awaitCore { core.tokensAdded.isNotEmpty() }
        assertEquals("bitbucket" to "me@example.com:ATATT3xFfGF0abc", core.tokensAdded.single())
    }

    @Test
    fun githubKeepsItsOwnPage() {
        openService("github")
        awaitTag("openGithub")
        assertFalse(has("openTokenPage"))
    }
}
