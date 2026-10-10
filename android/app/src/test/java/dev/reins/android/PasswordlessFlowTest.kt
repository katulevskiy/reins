package dev.reins.android

import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Looper
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Foreground
import dev.reins.android.platform.SsoPurpose
import dev.reins.android.platform.SsoRedirectActivity
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.AccountKeys
import dev.reins.core.CoreException
import dev.reins.core.JoinProgress
import java.time.Duration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Signing in without a password ("Continue" in a Custom Tab, back through `com.reins2fa.app://sso-callback`), the Unlock
 * screen of a phone whose account keys are on another phone, the approval device's side of "Add another phone", and
 * the recovery code in Settings.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PasswordlessFlowTest : FlowHarness() {
    private val server get() = AccountRules.serverUrl(BuildConfig.DEFAULT_SERVER)!!
    private val callback = "com.reins2fa.app://sso-callback?code=c0de&state=${FakeCore.SSO_STATE}"

    @Before
    fun signedOut() {
        dev.reins.android.TestNativeKeys.install()
        core.session = null
        core.registrations.clear()
        core.resetOnboarding()
        context.getSharedPreferences("recovery-record", android.content.Context.MODE_PRIVATE).edit().clear().commit()
    }

    /** What SsoRedirectActivity hands MainActivity when the browser comes back. */
    private fun callbackIntent(uri: String = callback) = Intent(context, MainActivity::class.java)
        .setAction(SsoRedirectActivity.ACTION_SIGNED_IN)
        .setData(Uri.parse(uri))

    /** "Continue" on the welcome page, until its page was handed to the browser. */
    private fun tapContinue() {
        launch()
        tap("continue")
        rule.waitUntil(10_000) { container.ssoSignIn.pending() != null }
    }

    /** Signs in with "Continue" to an account whose keys are on another phone: the Unlock screen. */
    private fun signInLocked() {
        core.ssoKeys = AccountKeys.LOCKED
        tapContinue()
        relaunch(callbackIntent())
        awaitTag("unlock")
    }

    @Test
    fun signingOutOfLockedAccountAlsoEndsTheBrowserSession() {
        signInLocked()
        val app = shadowOf(context as android.app.Application)
        while (app.nextStartedActivity != null) { /* discard the initial sign-in browser intent */ }
        core.browserLogoutUrl = "https://api.workos.com/user_management/sessions/logout?session_id=session_locked"
        tap("unlockSignOut")
        awaitTag("welcome")
        assertEquals(core.browserLogoutUrl, app.nextStartedActivity?.data?.toString())
        assertNull(core.session)
        assertNull(container.ssoSignIn.pending())
    }

    /** Lets the join poll's pauses pass (the main looper's clock only moves when told to). */
    private fun pollOnce() {
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(2_100))
        settle()
    }

    private fun recordRecovery() {
        awaitTag("recoveryRecorded")
        rule.onNodeWithTag("recoveryCodeDone").assertIsNotEnabled()
        tap("recoveryRecorded")

        tap("recoveryCodeDone")
        awaitGone("recoveryRecorded")
    }

    @Test
    fun recoveryRecordingIsRequiredAndSurvivesARestart() {
        tapContinue()
        relaunch(callbackIntent())
        awaitTag("recoveryRecorded")
        assertFalse(has("setup"))
        assertEquals("Recovery setup must finish before foreground polling starts", 0, core.syncStarts.get())
        rule.onNodeWithTag("recoveryCodeDone").assertIsNotEnabled()
        tap("copyRecoveryCode")
        rule.onNodeWithTag("recoveryCodeDone").assertIsNotEnabled()
        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("recoveryRecorded")
        recordRecovery()
        awaitTag("setup")
        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("setup")
        assertFalse(has("recoveryRecorded"))
    }

    @Test
    fun signingOutAndInWhileTheActivityIsStartedRestartsForegroundSync() {
        val info = dev.reins.core.SessionInfo(server, "me@example.com")
        core.session = info
        launch()
        awaitCore { core.syncStarts.get() > 0 }
        container.state.setSession(dev.reins.android.state.SessionState.SignedOut)
        awaitTag("welcome")
        val stopped = core.syncStarts.get()
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(300))
        settle()
        assertEquals(stopped, core.syncStarts.get())
        container.state.setSession(dev.reins.android.state.SessionState.SignedIn(info))
        awaitCore { core.syncStarts.get() > stopped }
    }

    @Test
    fun aPollRefusedBeforeThePhoneRegisteredKeepsPolling() {
        // Right after a sign-in the first poll can reach the server before the phone registers as the approval device.
        core.syncRefusals.set(1)
        core.session = dev.reins.core.SessionInfo(server, "me@example.com")
        launch()
        awaitCore { core.syncStarts.get() >= 3 }
        assertFalse(container.state.deviceReplaced.value)
    }

    // ---- "Continue" --------------------------------------------------------------------------------------------------

    @Test
    fun continueOpensTheSignInPageAndTheCallbackSignsInANewAccount() {
        tapContinue()
        assertEquals(listOf(server), core.ssoBegins.toList())
        val opened = nextStarted()
        assertNotNull(opened)
        assertEquals("$server/identity/connect/authorize?state=${FakeCore.SSO_STATE}", opened!!.dataString)

        relaunch(callbackIntent())
        recordRecovery()
        awaitTag("setup")
        assertEquals(listOf(server, callback, FakeCore.SSO_STATE, FakeCore.SSO_VERIFIER), core.ssoFinishes.single())
        awaitCore { container.state.approvalDevice.value }
        assertEquals(1, core.registrations.size)
        // Each start is used once.
        assertNull(container.ssoSignIn.pending())
        assertFalse(container.deviceStatus.keysLocked())
    }

    @Test
    fun anAccountThisPhoneOpensFinishesLikeAPasswordSignIn() {
        core.ssoKeys = AccountKeys.UNLOCKED
        tapContinue()
        relaunch(callbackIntent())
        recordRecovery()
        awaitTag("setup")
        awaitCore { core.registrations.size == 1 }
        assertFalse(has("unlock"))
    }

    @Test
    fun aCallbackNobodyWaitsForIsIgnored() {
        launch(callbackIntent())
        awaitTag("welcome")
        settle()
        assertTrue(core.ssoFinishes.isEmpty())
    }

    @Test
    fun somethingThatIsNotTheCallbackIsNotSent() {
        tapContinue()
        relaunch(callbackIntent("https://evil.example.com/sso-callback?code=x&state=${FakeCore.SSO_STATE}"))
        awaitTag("welcome")
        settle()
        assertTrue(core.ssoFinishes.isEmpty())
        // The sign-in still waits for the real callback.
        assertNotNull(container.ssoSignIn.pending())
    }

    @Test
    fun aCallbackForAnotherSignInIsRefusedInPlainWords() {
        tapContinue()
        relaunch(callbackIntent("com.reins2fa.app://sso-callback?code=c0de&state=other"))
        awaitText("The sign-in did not come back as expected. Try again.")
        assertTrue(has("continue"))
        assertTrue(core.registrations.isEmpty())
    }

    @Test
    fun aServerThatCannotStartTheSignInSaysWhy() {
        core.ssoBeginError = CoreException.Network("offline")
        launch()
        tap("continue")
        awaitTag("signInError")
        awaitText("Can't reach the server. Check your connection and try again.")
        assertNull(container.ssoSignIn.pending())
        assertNull(nextStarted())
    }

    @Test
    fun anotherServerSignsInThroughItsOwnPage() {
        launch()
        tap("useAnotherServer")
        awaitTag("server")
        rule.onNodeWithTag("server").performTextReplacement("reins.example.com")
        tap("continue")
        rule.waitUntil(10_000) { container.ssoSignIn.pending() != null }
        assertEquals(listOf("https://reins.example.com"), core.ssoBegins.toList())
        relaunch(callbackIntent())
        recordRecovery()
        awaitTag("setup")
        assertEquals("https://reins.example.com", core.ssoFinishes.single()[0])
    }

    @Test
    fun theRedirectActivityHandsTheCallbackToTheAppAndCloses() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(callback)).setClass(context, SsoRedirectActivity::class.java)
        val activity = org.robolectric.Robolectric.buildActivity(SsoRedirectActivity::class.java, view).create().get()
        val forwarded = nextStarted()
        assertNotNull(forwarded)
        assertEquals(MainActivity::class.java.name, forwarded!!.component?.className)
        assertEquals(SsoRedirectActivity.ACTION_SIGNED_IN, forwarded.action)
        assertEquals(callback, forwarded.dataString)
        assertTrue(forwarded.flags and Intent.FLAG_ACTIVITY_CLEAR_TOP != 0)
        assertTrue(forwarded.flags and Intent.FLAG_ACTIVITY_SINGLE_TOP != 0)
        assertTrue(activity.isFinishing)
    }

    @Test
    fun theManifestRoutesTheCallbackToTheRedirectActivity() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(callback)).addCategory(Intent.CATEGORY_BROWSABLE)
        val match = context.packageManager.queryIntentActivities(view, 0)
        assertEquals(listOf(SsoRedirectActivity::class.java.name), match.map { it.activityInfo.name })
    }

    // ---- keys on another phone -------------------------------------------------------------------------------------

    @Test
    fun aLockedAccountShowsUnlockAndNeverRegistersThisPhone() {
        signInLocked()
        rule.onNodeWithTag("unlockText").assertTextContains("me@example.com", substring = true)
        assertTrue(container.deviceStatus.keysLocked())
        assertFalse(has("setup"))

        // A relaunch comes back to it, still without taking the approval role from the other phone.
        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("unlock")
        settle()
        assertTrue(core.registrations.isEmpty())
        assertFalse(container.state.approvalDevice.value)
    }

    @Test
    fun askingTheOtherPhoneShowsTheCodeAndPollsUntilItApproves() {
        signInLocked()
        tap("askOtherPhone")
        awaitTag("joinCode")
        rule.onNodeWithTag("joinCode").assertTextEquals("482 193")
        assertEquals(listOf(Build.MODEL), core.joinBegins.toList())
        assertTrue(showsText("Check that your other phone shows"))
        assertTrue(core.registrations.isEmpty())
        repeat(3) { pollOnce() }
        recordRecovery()
        awaitTag("setup")
        assertEquals(3, core.joinPolls.get())
        awaitCore { core.registrations.size == 1 }
        assertFalse(container.deviceStatus.keysLocked())
    }

    @Test
    fun aRefusedRequestSaysSoAndCanBeAskedAgain() {
        core.joinWaits = 0
        core.joinAnswer = JoinProgress.DENIED
        signInLocked()
        tap("askOtherPhone")
        awaitTag("joinCode")
        pollOnce()
        awaitTag("joinEnded")
        awaitText("Your other phone said no", substring = true)
        rule.onNodeWithTag("askOtherPhone").assertIsEnabled()
        assertTrue(core.registrations.isEmpty())
        assertTrue(container.deviceStatus.keysLocked())
    }

    @Test
    fun anExpiredRequestSaysSo() {
        core.joinWaits = 0
        core.joinAnswer = JoinProgress.EXPIRED
        signInLocked()
        tap("askOtherPhone")
        awaitTag("joinCode")
        pollOnce()
        awaitText("Your other phone did not answer in time. Ask again.")
    }

    @Test
    fun cancellingWithdrawsTheRequest() {
        core.joinWaits = Int.MAX_VALUE
        signInLocked()
        tap("askOtherPhone")
        awaitTag("joinCode")
        tap("cancelJoin")
        awaitCore { core.joinCancels.get() == 1 }
        awaitTag("askOtherPhone")
        val polls = core.joinPolls.get()
        repeat(2) { pollOnce() }
        assertEquals(polls, core.joinPolls.get())
    }

    @Test
    fun noOtherPhoneIsExplained() {
        core.joinBeginError = CoreException.Invalid("No other phone approves for this account yet. Use the recovery code.")
        signInLocked()
        tap("askOtherPhone")
        awaitText("No other phone approves for this account yet. Use the recovery code.")
        assertTrue(has("enterRecoveryCode"))
    }

    @Test
    fun theRecoveryCodeUnlocksAndAWrongOneIsExplained() {
        signInLocked()
        tap("enterRecoveryCode")
        awaitTag("recoveryCode")
        rule.onNodeWithTag("unlockAccount").assertIsNotEnabled()
        rule.onNodeWithTag("recoveryCode").performTextReplacement("AAAA-BBBB")
        tap("unlockAccount")
        awaitTag("unlockError")
        awaitText("That is neither the recovery code nor the master password.")
        assertTrue(container.deviceStatus.keysLocked())

        val typed = FakeCore.RECOVERY_CODE.lowercase().replace('-', ' ')
        rule.onNodeWithTag("recoveryCode").performTextReplacement(typed)
        tap("unlockAccount")
        recordRecovery()
        awaitTag("setup")
        assertEquals(typed, core.unlockAttempts.last())
        awaitCore { core.registrations.size == 1 }
        assertFalse(container.deviceStatus.keysLocked())
    }

    @Test
    fun signingOutFromUnlockGoesBackToTheWelcome() {
        signInLocked()
        tap("unlockSignOut")
        awaitTag("welcome")
        assertNull(core.session)
        assertFalse(container.deviceStatus.keysLocked())
        assertTrue(core.registrations.isEmpty())
    }

    // ---- "Lost both? Reset the vault" --------------------------------------------------------------------------------

    /** From the locked Unlock screen to the reset's sign-in page handed to the browser. */
    private fun startReset() {
        signInLocked()
        while (nextStarted() != null) { /* the first sign-in's page */ }
        tap("resetVault")
        awaitTag("confirmReset")
        tap("confirmReset")
        rule.waitUntil(10_000) { container.ssoSignIn.pending()?.purpose == SsoPurpose.Reset }
    }

    @Test
    fun aLockedAccountOffersTheResetAndBackLeavesItAlone() {
        signInLocked()
        tap("resetVault")
        awaitTag("confirmReset")
        assertTrue(showsText("Deletes the vault", substring = true))
        assertTrue(showsText("AIs connect again", substring = true))
        assertTrue(showsText("To confirm, sign in again with this account."))
        tap("resetBack")
        awaitTag("askOtherPhone")
        assertFalse(has("confirmReset"))
        assertTrue(core.ssoBegins.size == 1 && core.resets.isEmpty())
        assertTrue(container.deviceStatus.keysLocked())
    }

    @Test
    fun resettingSignsInAgainAndEndsSignedInWithTheNewCodeToRecord() {
        startReset()
        assertEquals(listOf(server, server), core.ssoBegins.toList())
        assertEquals("$server/identity/connect/authorize?state=${FakeCore.SSO_STATE}", nextStarted()?.dataString)
        // The tab was closed without signing in: the reset waits as it was.
        settle()
        assertTrue(has("confirmReset"))
        rule.onNodeWithTag("resetBack").assertIsEnabled()
        assertTrue(core.resets.isEmpty())

        relaunch(callbackIntent())
        awaitTag("recoveryRecorded")
        assertEquals(FakeCore.RESET_RECOVERY_CODE, container.state.recoveryToRecord.value)
        assertTrue(showsText("HV3N Q8RT ZL2K M7WD", substring = true))
        recordRecovery()
        awaitTag("setup")
        assertEquals(listOf(server, callback, FakeCore.SSO_STATE, FakeCore.SSO_VERIFIER), core.resets.single())
        assertEquals("Only the first sign-in went to ssoFinish", 1, core.ssoFinishes.size)
        assertFalse(container.deviceStatus.keysLocked())
        assertFalse(container.state.keysLocked.value)
        assertNull(container.ssoSignIn.pending())
        awaitCore { core.registrations.size == 1 }
    }

    @Test
    fun aResetCallbackGoesToResetAccountAlsoFromTheRestoredRecord() {
        startReset()
        // What a restarted app reads back: the start, and that it is a reset.
        val restored = dev.reins.android.platform.SsoSignIn(context).pending()
        assertEquals(SsoPurpose.Reset, restored?.purpose)
        assertEquals(server, restored?.server)

        // The sign-in screen's side never takes a reset's callback; the Unlock screen's does.
        val signIn = dev.reins.android.platform.SsoSignIn(context)
        assertTrue(signIn.deliver(callback))
        assertNull(signIn.take(SsoPurpose.SignIn))
        assertEquals(callback, signIn.callback.value)
        assertNotNull(signIn.pending())

        // A fresh Unlock screen (the activity was recreated) finishes it with resetAccount, not ssoFinish.
        relaunch(callbackIntent())
        awaitTag("recoveryRecorded")
        assertEquals(1, core.resets.size)
        assertEquals(1, core.ssoFinishes.size)
    }

    @Test
    fun aRefusedResetSaysWhyAndLeavesTheAccountLocked() {
        val reason = "You signed in as other@example.com. Sign in as me@example.com to reset its vault."
        core.resetError = CoreException.Invalid(reason)
        startReset()
        relaunch(callbackIntent())
        awaitTag("unlockError")
        awaitText(reason)
        assertTrue(has("confirmReset"))
        assertEquals(1, core.resets.size)
        assertTrue(container.deviceStatus.keysLocked())
        assertTrue(core.registrations.isEmpty())
        assertFalse(has("recoveryRecorded"))
    }

    // ---- the approval device: another phone asks -------------------------------------------------------------------

    private fun openJoin() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.pending = listOf(TestData.joinItem())
        core.joins["join1"] = TestData.joinView()
        launch(link("join", "join1"))
        awaitTag("joinCode")
    }

    @Test
    fun aWaitingPhoneIsListedLikeAPairing() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.pending = listOf(TestData.joinItem())
        Foreground.autoPopup = false
        launch()
        awaitTag("pending:join1")
        assertTrue(showsText("Add Pixel 9?"))
    }

    @Test
    fun approvingAnotherPhoneAsksForBiometricsFirst() {
        openJoin()
        rule.onNodeWithTag("joinTitle").assertTextEquals("Add Pixel 9?")
        rule.onNodeWithTag("joinCode").assertTextEquals("482 193")
        tap("approve")
        awaitCore { core.joinAnswers.isNotEmpty() }
        assertEquals("join1" to true, core.joinAnswers.single())
        assertEquals(1, prompts.get())
        awaitText("Pixel 9 can open your account now. It approves from now on once it's set up.")
        assertFalse(has("sheet"))
    }

    @Test
    fun cancelledBiometricsAnswerNothing() {
        authResult = AuthResult.Cancelled
        openJoin()
        tap("approve")
        awaitCore { prompts.get() == 1 }
        assertTrue(core.joinAnswers.isEmpty())
        assertTrue(has("joinCode"))
    }

    @Test
    fun denyingAnotherPhoneNeedsNoBiometrics() {
        openJoin()
        tap("deny")
        awaitCore { core.joinAnswers.isNotEmpty() }
        assertEquals("join1" to false, core.joinAnswers.single())
        assertEquals(0, prompts.get())
    }

    // ---- Settings > Account ------------------------------------------------------------------------------------------

    @Test
    fun anAccountWithAMasterPasswordHasNoRecoveryCodeRow() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        launch()
        tap("openSettings")
        tap("help:Account")
        awaitText("To add another phone, sign in on it; this phone asks you to approve it.")
        awaitCore { core.recoveryCodeReads.get() > 0 }
        assertFalse(has("recoveryCodeRow"))
    }

    @Test
    fun theRecoveryCodeIsShownAfterBiometricsAndCanBeCopied() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.recoveryCode = FakeCore.RECOVERY_CODE
        launch()
        recordRecovery()
        tap("openSettings")
        authResult = AuthResult.Cancelled
        tap("recoveryCodeRow")
        awaitCore { prompts.get() == 1 }
        assertFalse(has("recoveryCodeSheet"))

        authResult = AuthResult.Success
        tap("recoveryCodeRow")
        awaitTag("recoveryCodeSheet")
        assertEquals(2, prompts.get())
        assertTrue(showsText("ABCD EFGH IJKL MNOP", substring = true))
        assertTrue(showsText("Write it down", substring = true))
        tap("recoveryCode")
        val clip = context.getSystemService(ClipboardManager::class.java).primaryClip
        assertEquals(FakeCore.RECOVERY_CODE, clip?.getItemAt(0)?.text?.toString())
        tap("recoveryCodeDone")
        awaitGone("recoveryCodeSheet")
    }
}
