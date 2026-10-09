package dev.reins.android

import android.content.Intent
import android.net.Uri
import android.os.Looper
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.ListenableWorker
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.TestListenableWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import dev.reins.android.platform.SsoRedirectActivity
import dev.reins.android.push.RegisterDeviceWorker
import dev.reins.android.ui.common.OTHER_APPROVAL_DEVICE
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AccountKeys
import dev.reins.core.CoreException
import java.time.Duration
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The server refuses to make this phone the approval device while another phone approves for the account and this one
 * brings no proof ([CoreException.OtherApprovalDevice]): the Unlock screen says so and offers the other phone's
 * approval or the recovery code, after which the registration goes through. The background re-registration stops on it.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class TakeoverFlowTest : FlowHarness() {
    private val callback = "com.reins2fa.app://sso-callback?code=c0de&state=${FakeCore.SSO_STATE}"

    @Before
    fun anotherPhoneApproves() {
        dev.reins.android.TestNativeKeys.install()
        core.registrations.clear()
        core.resetOnboarding()
        core.otherApprovalDevice = true
        core.session?.let { container.deviceStatus.selectAccount(it) }
    }

    private fun callbackIntent() = Intent(context, MainActivity::class.java)
        .setAction(SsoRedirectActivity.ACTION_SIGNED_IN)
        .setData(Uri.parse(callback))

    /** "Continue" to an account whose keys this phone opens, while another phone approves for it. */
    private fun signInRefused() {
        core.session = null
        core.ssoKeys = AccountKeys.UNLOCKED
        launch()
        tap("continue")
        rule.waitUntil(10_000) { container.ssoSignIn.pending() != null }
        relaunch(callbackIntent())
        awaitTag("takeoverText")
    }

    private fun pollOnce() {
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(2_100))
        settle()
    }

    @Test
    fun theRefusalIsTheTakeoverScreenWithTheTwoWays() {
        assertEquals(OTHER_APPROVAL_DEVICE, CoreException.OtherApprovalDevice().userMessage())
        signInRefused()
        rule.onNodeWithTag("takeoverText").assertTextEquals(
            "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code.",
        )
        assertTrue(has("askOtherPhone"))
        assertTrue(has("enterRecoveryCode"))
        assertFalse(has("setupComputer"))
        assertEquals(1, core.refusedRegistrations.get())
        assertTrue(core.registrations.isEmpty())
        assertFalse(container.state.approvalDevice.value)
        assertTrue(container.deviceStatus.needsTakeover())

        // A relaunch comes back to it.
        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("takeoverText")
    }

    @Test
    fun takingOverOffersNoVaultReset() {
        signInRefused()
        assertTrue(has("askOtherPhone"))
        assertFalse(has("resetVault"))
    }

    @Test
    fun theRecoveryCodeTakesOverAndTheSignInCarriesOn() {
        signInRefused()
        tap("enterRecoveryCode")
        awaitTag("recoveryCode")
        rule.onNodeWithTag("recoveryCode").performTextReplacement(FakeCore.RECOVERY_CODE)
        tap("unlockAccount")
        tap("recoveryRecorded")

        tap("recoveryCodeDone")
        awaitTag("setupComputer")
        assertEquals(FakeCore.RECOVERY_CODE, core.unlockAttempts.single())
        awaitCore { container.state.approvalDevice.value }
        assertEquals(1, core.registrations.size)
        assertFalse(container.deviceStatus.needsTakeover())
        assertFalse(container.state.approvalTakeover.value)
        assertNull(container.state.registrationError.value)
    }

    @Test
    fun theOtherPhonesApprovalTakesOver() {
        signInRefused()
        tap("askOtherPhone")
        awaitTag("joinCode")
        rule.onNodeWithTag("joinCode").assertTextEquals("482 193")
        repeat(3) { pollOnce() }
        tap("recoveryRecorded")

        tap("recoveryCodeDone")
        awaitTag("setupComputer")
        awaitCore { core.registrations.size == 1 }
        assertTrue(container.state.approvalDevice.value)
        assertFalse(container.deviceStatus.needsTakeover())
    }

    @Test
    fun aReplacedPhoneUsingThisPhoneAgainSeesTheSameScreen() {
        container.deviceStatus.setReplaced(true)
        launch()
        tap("openSettings")
        tap("registerPhone")
        awaitTag("takeoverText")
        assertTrue(core.registrations.isEmpty())

        // "Not now": back to the app, still not approving; Settings says why and offers it again.
        tap("takeoverLater")
        awaitGone("unlock")
        assertFalse(container.deviceStatus.needsTakeover())
        awaitText(OTHER_APPROVAL_DEVICE)
        assertTrue(container.deviceStatus.isReplaced())

        tap("registerPhone")
        tap("enterRecoveryCode")
        awaitTag("recoveryCode")
        rule.onNodeWithTag("recoveryCode").performTextReplacement(FakeCore.RECOVERY_CODE)
        tap("unlockAccount")
        awaitGone("unlock")
        awaitCore { container.state.approvalDevice.value }
        assertEquals(1, core.registrations.size)
        assertFalse(container.deviceStatus.isReplaced())
    }

    @Test
    fun theBackgroundRegistrationStopsOnTheRefusalAndAsksTheUser() {
        val result = runBlocking { TestListenableWorkerBuilder<RegisterDeviceWorker>(context).build().doWork() }
        assertEquals(ListenableWorker.Result.failure(), result)
        assertEquals(1, core.refusedRegistrations.get())
        assertEquals(OTHER_APPROVAL_DEVICE, container.state.registrationError.value)
        assertTrue(container.deviceStatus.needsTakeover())

        // Through WorkManager: one attempt, then a final failure instead of a retry.
        RegisterDeviceWorker.enqueue(context)
        val work = WorkManager.getInstance(context)
        val id = work.getWorkInfosForUniqueWork("register-device").get().single().id
        WorkManagerTestInitHelper.getTestDriver(context)!!.setAllConstraintsMet(id)
        // A CoroutineWorker finishes off the main thread: wait for it to end.
        val deadline = System.currentTimeMillis() + 10_000
        var state = work.getWorkInfoById(id).get()!!.state
        while (!state.isFinished && System.currentTimeMillis() < deadline) {
            Thread.sleep(50)
            shadowOf(Looper.getMainLooper()).idle()
            state = work.getWorkInfoById(id).get()!!.state
        }
        assertEquals(WorkInfo.State.FAILED, state)
        assertEquals(2, core.refusedRegistrations.get())

        launch()
        awaitTag("takeoverText")
    }
}
