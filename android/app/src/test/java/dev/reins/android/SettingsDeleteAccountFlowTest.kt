package dev.reins.android

import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.state.SessionState
import dev.reins.android.ui.settings.deletionConfirmed
import dev.reins.core.CoreException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Settings > Session > Delete account: the sheet says what goes and wants the account's email typed; the core's
 * refusal shows in the sheet and changes nothing; a deletion ends on the welcome screen with the phone's state of the
 * account forgotten, as after signing out (but without the browser page).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class SettingsDeleteAccountFlowTest : FlowHarness() {
    private fun openSheet() {
        launch()
        tap("openSettings")
        tap("deleteAccount")
        awaitTag("deleteAccountSheet")
    }

    private fun type(text: String) {
        rule.onNodeWithTag("deleteAccountEmail").performTextReplacement(text)
        settle()
    }

    @Test
    fun theTypedEmailMustBeTheAccountsOwn() {
        assertTrue(deletionConfirmed("me@example.com", "me@example.com"))
        assertTrue(deletionConfirmed("  Me@Example.COM ", "me@example.com"))
        assertFalse(deletionConfirmed("", ""))
        assertFalse(deletionConfirmed("me@example.co", "me@example.com"))
        assertFalse(deletionConfirmed("DELETE", "me@example.com"))
    }

    @Test
    fun theSheetSaysWhatGoesAndDeletesOnlyOnceTheEmailIsTyped() {
        openSheet()
        assertTrue(showsText("It cannot be undone", substring = true))
        assertTrue(showsText("Your account and its vault on the server"))
        assertTrue(showsText("Type me@example.com to confirm."))
        rule.onNodeWithTag("confirmDeleteAccount").assertIsNotEnabled()
        type("someone@example.com")
        rule.onNodeWithTag("confirmDeleteAccount").assertIsNotEnabled()
        type(" Me@Example.com ")
        rule.onNodeWithTag("confirmDeleteAccount").assertIsEnabled()
        tap("confirmDeleteAccount")
        awaitTag("welcome")
        assertEquals(listOf(" Me@Example.com "), core.deletedAccounts.toList())
        assertNull(core.session)
        assertEquals(SessionState.SignedOut, container.state.session.value)
        assertFalse(container.state.approvalDevice.value)
        assertNull("no browser page: the account's sign-in went with it", nextStarted())
    }

    @Test
    fun aRefusalIsShownInTheSheetAndKeepsTheAccount() {
        core.deleteAccountError = CoreException.Invalid(
            "Another phone approves requests for this account. Delete the account from that phone, or make this phone the approval device first.",
        )
        openSheet()
        type("me@example.com")
        tap("confirmDeleteAccount")
        awaitTag("deleteAccountError")
        assertTrue(showsText("Delete the account from that phone", substring = true))
        assertNotNull(core.session)
        assertTrue(core.deletedAccounts.isEmpty())
        assertTrue(container.state.session.value is SessionState.SignedIn)
        core.deleteAccountError = null
        tap("confirmDeleteAccount")
        awaitTag("welcome")
        assertEquals(1, core.deletedAccounts.size)
    }

    @Test
    fun cancellingDeletesNothing() {
        openSheet()
        type("me@example.com")
        tap("cancelDeleteAccount")
        awaitGone("deleteAccountSheet")
        assertTrue(core.deletedAccounts.isEmpty())
        assertNotNull(core.session)
        assertTrue(has("signOut"))
    }
}
