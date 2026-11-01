package dev.reins.android

import android.content.Intent
import android.net.Uri
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.XRedirectActivity
import dev.reins.core.CoreException
import dev.reins.core.ServiceView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class XFlowTest : FlowHarness() {
    private val redirect = "com.reins2fa.app://x-oauth?state=st4te&code=abc123"

    private fun redirectIntent(uri: String = redirect) = Intent(context, MainActivity::class.java)
        .setAction(XRedirectActivity.ACTION_SIGNED_IN)
        .setData(Uri.parse(uri))

    private fun launchX() {
        launch()
        tap("tabConnections")
        openService("x")
    }

    @Test
    fun theIntegrationsListHasX() {
        launch()
        tap("tabConnections")
        tap("addService")
        awaitTag("service:x")
    }

    @Test
    fun signingInOpensXsPageAndTheRedirectAddsTheAccount() {
        launchX()
        tap("oauthSignIn")
        awaitCore { core.oauthCalls.isNotEmpty() }
        assertEquals(listOf("begin", "x"), core.oauthCalls.first())
        var opened: Intent? = null
        rule.waitUntil(10_000) { nextStarted()?.also { opened = it } != null }
        assertEquals(Intent.ACTION_VIEW, opened!!.action)
        assertEquals(core.oauthAuthorizeUrl, opened!!.dataString)
        assertEquals("x", container.oauthSignIn.pending())

        // The browser comes back through the redirect activity, which hands the address to the app.
        relaunch(redirectIntent())
        awaitCore { core.oauthCalls.size == 2 }
        assertEquals(listOf("finish", "x", redirect), core.oauthCalls[1])
        awaitTag("connectedIdentity")
        rule.onNodeWithTag("connectedIdentity").assertTextEquals("reins_test")
        assertNull(container.oauthSignIn.pending())
    }

    @Test
    fun aFailedSignInIsExplainedOnXsPage() {
        container.oauthSignIn.begin("x")
        core.serviceFailure = CoreException.Invalid("X did not allow it: access_denied")
        launch(redirectIntent("com.reins2fa.app://x-oauth?error=access_denied&state=st4te"))
        awaitTag("signInError")
        awaitText("X did not allow it", substring = true)
        // Signing in again is offered.
        awaitTag("oauthSignIn")
    }

    @Test
    fun aRedirectNobodyWaitsForIsIgnored() {
        launch(redirectIntent())
        awaitTag("tabConnections")
        settle()
        assertTrue(core.oauthCalls.isEmpty())
    }

    @Test
    fun aRedirectIsUsedOnlyOnce() {
        container.oauthSignIn.begin("x")
        launch(redirectIntent())
        awaitCore { core.oauthCalls.size == 1 }
        relaunch(redirectIntent())
        awaitTag("tabConnections")
        settle()
        assertEquals(1, core.oauthCalls.size)
    }

    @Test
    fun somethingThatIsNotTheRedirectIsNotSent() {
        container.oauthSignIn.begin("x")
        launch(redirectIntent("com.reins2fa.app://x-oauth.evil.com?state=st4te&code=x"))
        awaitTag("tabConnections")
        settle()
        relaunch(redirectIntent("https://evil.example.com/x-oauth?state=st4te&code=x"))
        awaitTag("tabConnections")
        settle()
        assertTrue(core.oauthCalls.isEmpty())
        // The sign-in still waits for the real redirect.
        assertEquals("x", container.oauthSignIn.pending())
    }

    @Test
    fun aSignInPageThatIsNotHttpsIsNeverOpened() {
        core.oauthAuthorizeUrl = "http://x.com/i/oauth2/authorize?state=st4te"
        launchX()
        tap("oauthSignIn")
        awaitTag("accountError")
        assertNull(nextStarted())
        assertNull(container.oauthSignIn.pending())
    }

    @Test
    fun withoutAClientIdXSaysWhy() {
        core.catalogue = core.catalogue.map {
            if (it.service == "x") ServiceView("x", "X", "oauth", false, "This build has no X client id.", emptyList()) else it
        }
        launchX()
        awaitTag("unavailable")
        awaitText("This build has no X client id.")
        assertFalse(has("oauthSignIn"))
    }

    @Test
    fun theRedirectActivityHandsTheAddressToTheAppAndCloses() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(redirect)).setClass(context, XRedirectActivity::class.java)
        val activity = org.robolectric.Robolectric.buildActivity(XRedirectActivity::class.java, view).create().get()
        val forwarded = nextStarted()
        assertNotNull(forwarded)
        assertEquals(MainActivity::class.java.name, forwarded!!.component?.className)
        assertEquals(XRedirectActivity.ACTION_SIGNED_IN, forwarded.action)
        assertEquals(redirect, forwarded.dataString)
        assertTrue(forwarded.flags and Intent.FLAG_ACTIVITY_CLEAR_TOP != 0)
        assertTrue(forwarded.flags and Intent.FLAG_ACTIVITY_SINGLE_TOP != 0)
        assertTrue(activity.isFinishing)
    }

    @Test
    fun theRedirectActivityForwardsNothingElse() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse("com.reins2fa.app://x-oauth.evil.com?code=x")).setClass(context, XRedirectActivity::class.java)
        val activity = org.robolectric.Robolectric.buildActivity(XRedirectActivity::class.java, view).create().get()
        assertNull(nextStarted())
        assertTrue(activity.isFinishing)
    }

    @Test
    fun theManifestRoutesTheRedirectToTheRedirectActivity() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(redirect)).addCategory(Intent.CATEGORY_BROWSABLE)
        val match = context.packageManager.queryIntentActivities(view, 0)
        assertEquals(listOf(XRedirectActivity::class.java.name), match.map { it.activityInfo.name })
    }
}
