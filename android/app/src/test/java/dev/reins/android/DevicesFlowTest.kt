package dev.reins.android

import androidx.compose.ui.test.onLast
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.CoreException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Settings > Devices: this phone (the approval device, its key for `reins vault add`), the other devices signed in to
 * the account with a way to sign a lost phone out, and the computers, which open their own page.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class DevicesFlowTest : FlowHarness() {
    @Before
    fun devices() {
        core.resetDevices()
        core.connections = listOf(TestData.connection(), TestData.computer())
    }

    private fun openDevices() {
        launch()
        tap("openSettings")
        tap("devicesRow")
        awaitTag("thisDevice")
    }

    @Test
    fun thisPhoneTheOthersAndTheComputersAreListed() {
        openDevices()
        awaitText("Your approval device", substring = true)
        assertTrue(showsText("Key for reins vault add: 4821-9930-1274", substring = true))
        assertTrue(has("device:d-old") && has("device:d-web"))
        assertFalse("this phone is not offered for signing out", has("signOutDevice:d-this"))
        assertTrue(showsText("Android · last seen", substring = true))
        assertTrue(has("deviceComputer:d1"))
        assertFalse("an AI app is not a computer", has("deviceComputer:c1"))
        tap("deviceComputer:d1")
        awaitTag("disconnect")
    }

    @Test
    fun aLostPhoneIsSignedOutWithTheRecoveryCodeTypedThen() {
        openDevices()
        tap("signOutDevice:d-old")
        awaitTag("signOutDialog")
        assertTrue(showsText("Sign out Pixel 7?"))
        assertTrue("two phones may share a name: the dialog says which", showsText("Android · signed in", substring = true))
        rule.onNodeWithTag("signOutProof").performTextInput("wrong code")
        tap("confirmSignOut")
        awaitTag("signOutError")
        assertTrue(core.signedOutDevices.isEmpty())
        rule.onNodeWithTag("signOutProof").performTextReplacement(FakeCore.RECOVERY_CODE)
        tap("confirmSignOut")
        awaitCore { core.signedOutDevices.toList() == listOf("d-old") }
        awaitGone("signOutDialog")
        // The phone signed out may still keep the recovery code: a new one is offered right away.
        core.recoveryCode = FakeCore.RECOVERY_CODE
        core.rotations.set(0)
        tap("rotateAfterSignOut")
        awaitText("Make a new recovery code?")
        rule.onAllNodes(androidx.compose.ui.test.hasText("Make a new code")).onLast().performClick()
        awaitCore { core.rotations.get() == 1 }
        awaitTag("recoveryRecorded")
        awaitTag("devicesMessage")
        assertTrue(showsText("Pixel 7 is signed out", substring = true))
        awaitGone("device:d-old")
        assertEquals(listOf("d-this", "d-web"), core.devices.map { it.id })
    }

    @Test
    fun aPhoneThatIsNotTheApprovalDeviceIsToldWhichOneIs() {
        core.devicesError = CoreException.Server(403u, "not the approval device")
        openDevices()
        awaitTag("devicesError")
        assertTrue(showsText("Devices are listed and signed out from your approval phone", substring = true))
    }
}
