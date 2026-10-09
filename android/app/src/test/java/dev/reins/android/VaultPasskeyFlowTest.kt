package dev.reins.android

import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.PasskeyJson
import dev.reins.android.platform.PasskeyResult
import dev.reins.android.platform.SsoRedirectActivity
import dev.reins.android.ui.common.PASSKEY_UNSUPPORTED
import dev.reins.android.ui.signin.NO_PASSKEY_HERE
import dev.reins.core.AccountKeys
import dev.reins.core.CoreException
import dev.reins.core.VaultPasskeyView
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The passkey that opens the vault: offered before the recovery code to an account without one, "Unlock with passkey"
 * on the Unlock screen of an account with one, and the list in Settings.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class VaultPasskeyFlowTest : FlowHarness() {
    private val callback = "com.reins2fa.app://sso-callback?code=c0de&state=${FakeCore.SSO_STATE}"

    @Before
    fun signedOut() {
        dev.reins.android.TestNativeKeys.install()
        core.session = null
        core.registrations.clear()
        core.resetOnboarding()
        context.getSharedPreferences("recovery-record", android.content.Context.MODE_PRIVATE).edit().clear().commit()
    }

    private fun callbackIntent() = Intent(context, MainActivity::class.java)
        .setAction(SsoRedirectActivity.ACTION_SIGNED_IN)
        .setData(Uri.parse(callback))

    /** "Continue", and the browser comes back signed in. */
    private fun signIn() {
        launch()
        tap("continue")
        rule.waitUntil(10_000) { container.ssoSignIn.pending() != null }
        relaunch(callbackIntent())
    }

    /** A new account, which has no vault passkey yet: the offer comes before the recovery code. */
    private fun signUpWithoutPasskey() {
        core.vaultPasskeys = emptyList()
        signIn()
        awaitTag("passkeyOffer")
    }

    private fun recordRecovery() {
        tap("recoveryRecorded")
        tap("recoveryCodeDone")
        awaitGone("recoveryRecorded")
    }

    // ---- the offer before the recovery code --------------------------------------------------------------------------

    @Test
    fun aNewAccountAddsAPasskeyThenRecordsTheCodeThenSetsUp() {
        signUpWithoutPasskey()
        assertTrue(showsText("Protect your vault with a passkey"))
        assertTrue(showsText("no other phone or recovery code", substring = true))
        assertFalse(has("recoveryRecorded"))
        assertEquals("Nothing polls before the recovery code is recorded", 0, core.syncStarts.get())

        tap("addPasskey")
        awaitTag("recoveryRecorded")
        val (name, id, prf) = core.passkeyAdds.single()
        assertEquals(Build.MODEL, name)
        assertArrayEquals(FakePasskeys.NEW_PASSKEY_ID, id)
        assertArrayEquals(FakeCore.PASSKEY_PRF, prf)
        assertEquals("me@example.com", passkeys.creates.single().userName)
        assertTrue("The PRF output came with the new passkey", passkeys.gets.isEmpty())
        assertFalse(container.state.vaultPasskeyOffer.value)

        recordRecovery()
        awaitTag("setup")
        awaitCore { core.registrations.size == 1 }
    }

    @Test
    fun aProviderThatGivesThePrfOnlyOnUseIsAskedForTheNewPasskey() {
        passkeys.createResult = FakePasskeys.made(prf = null)
        passkeys.getResult = FakePasskeys.made()
        signUpWithoutPasskey()
        tap("addPasskey")
        awaitTag("recoveryRecorded")
        assertArrayEquals(FakePasskeys.NEW_PASSKEY_ID, passkeys.gets.single().single())
        assertArrayEquals(FakeCore.PASSKEY_PRF, core.passkeyAdds.single().third)
    }

    @Test
    fun skippingGoesToTheRecoveryCodeAndStaysSkippedForThisSignIn() {
        signUpWithoutPasskey()
        tap("skipPasskey")
        awaitTag("recoveryRecorded")
        assertTrue(passkeys.creates.isEmpty())
        assertTrue(core.passkeyAdds.isEmpty())

        // Coming back to the app before the code is recorded does not offer it again.
        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("recoveryRecorded")
        assertFalse(has("passkeyOffer"))
        recordRecovery()
        awaitTag("setup")
    }

    @Test
    fun aPasswordManagerWithoutPrfSaysSoAndLetsYouGoOn() {
        passkeys.createResult = PasskeyResult.Unsupported
        signUpWithoutPasskey()
        tap("addPasskey")
        awaitTag("passkeyError")
        awaitText(PASSKEY_UNSUPPORTED)
        assertTrue(core.passkeyAdds.isEmpty())
        rule.onNodeWithTag("skipPasskey").assertIsEnabled()

        // A new passkey that turns out to give no PRF output when used is not added either.
        passkeys.createResult = FakePasskeys.made(prf = null)
        passkeys.getResult = PasskeyResult.Unsupported
        tap("addPasskey")
        awaitCore { passkeys.gets.size == 1 }
        awaitText(PASSKEY_UNSUPPORTED)
        assertTrue(core.passkeyAdds.isEmpty())

        tap("skipPasskey")
        awaitTag("recoveryRecorded")
        recordRecovery()
        awaitTag("setup")
    }

    @Test
    fun closingThePromptChangesNothing() {
        passkeys.createResult = PasskeyResult.Cancelled
        signUpWithoutPasskey()
        tap("addPasskey")
        awaitCore { passkeys.creates.size == 1 }
        assertTrue(has("passkeyOffer"))
        assertFalse(has("passkeyError"))
        rule.onNodeWithTag("addPasskey").assertIsEnabled()
    }

    @Test
    fun offlineTheOfferIsSkippedRatherThanBlocking() {
        core.vaultPasskeys = emptyList()
        core.vaultPasskeysError = CoreException.Network("offline")
        signIn()
        awaitTag("recoveryRecorded")
        assertFalse(has("passkeyOffer"))
    }

    @Test
    fun anAccountWithAPasskeyGoesStraightToTheRecoveryCode() {
        signIn()
        awaitTag("recoveryRecorded")
        assertFalse(has("passkeyOffer"))
    }

    // ---- "Unlock with passkey" ---------------------------------------------------------------------------------------

    private fun signInLocked() {
        core.ssoKeys = AccountKeys.LOCKED
        signIn()
        awaitTag("unlock")
    }

    @Test
    fun aLockedAccountWithAPasskeyUnlocksWithIt() {
        signInLocked()
        awaitTag("unlockWithPasskey")
        assertTrue(showsText("Unlock it with your passkey", substring = true))
        tap("unlockWithPasskey")
        // The account has its passkey already: the recovery code comes next, without the offer.
        awaitTag("recoveryRecorded")
        assertFalse(has("passkeyOffer"))
        assertArrayEquals(FakeCore.EXISTING_PASSKEY_ID, passkeys.gets.single().single())
        assertArrayEquals(FakeCore.EXISTING_PASSKEY_ID, core.passkeyUnlocks.single())
        recordRecovery()
        awaitTag("setup")
        awaitCore { core.registrations.size == 1 }
        assertFalse(container.deviceStatus.keysLocked())
    }

    @Test
    fun aLockedAccountWithoutPasskeysOffersTheOtherWays() {
        core.vaultPasskeys = emptyList()
        signInLocked()
        awaitTag("askOtherPhone")
        assertFalse(has("unlockWithPasskey"))
    }

    @Test
    fun closingThePasskeyPromptStaysOnTheUnlockScreen() {
        passkeys.getResult = PasskeyResult.Cancelled
        signInLocked()
        tap("unlockWithPasskey")
        awaitCore { passkeys.gets.size == 1 }
        assertFalse(has("unlockError"))
        rule.onNodeWithTag("unlockWithPasskey").assertIsEnabled()
        assertTrue(core.passkeyUnlocks.isEmpty())
        assertTrue(container.deviceStatus.keysLocked())
    }

    @Test
    fun aPasskeyThatDoesNotOpenTheVaultIsExplained() {
        passkeys.getResult = PasskeyResult.Passkey(FakeCore.EXISTING_PASSKEY_ID, ByteArray(32))
        signInLocked()
        tap("unlockWithPasskey")
        awaitTag("unlockError")
        awaitText("This passkey does not open this account's vault.")
        assertTrue(container.deviceStatus.keysLocked())

        passkeys.getResult = PasskeyResult.Unsupported
        tap("unlockWithPasskey")
        awaitText(NO_PASSKEY_HERE)
        assertTrue(has("enterRecoveryCode"))
    }

    // ---- Settings > Vault passkeys -----------------------------------------------------------------------------------

    private fun openVaultPasskeys() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.recoveryCode = FakeCore.RECOVERY_CODE
        launch()
        if (core.vaultPasskeys.isEmpty()) tap("skipPasskey")
        recordRecovery()
        tap("openSettings")
        awaitTag("vaultPasskeysRow")
    }

    private fun tag(id: ByteArray) = PasskeyJson.b64(id)

    @Test
    fun settingsListsRemovesAndAddsPasskeys() {
        openVaultPasskeys()
        awaitText("1 passkey")
        tap("vaultPasskeysRow")
        awaitTag("passkey:${tag(FakeCore.EXISTING_PASSKEY_ID)}")
        assertTrue(showsText("Pixel 8"))

        tap("removePasskey:${tag(FakeCore.EXISTING_PASSKEY_ID)}")
        awaitText("Remove this passkey?")
        rule.onNodeWithText("Remove").performClick()
        awaitTag("noPasskeys")
        assertArrayEquals(FakeCore.EXISTING_PASSKEY_ID, core.passkeyRemovals.single())

        tap("addPasskey")
        awaitTag("passkey:${tag(FakePasskeys.NEW_PASSKEY_ID)}")
        assertEquals(Build.MODEL, core.passkeyAdds.single().first)
        assertTrue(showsText(Build.MODEL))

        pressBack()
        awaitTag("vaultPasskeysRow")
        awaitText("1 passkey")
    }

    @Test
    fun settingsSaysWhenThereIsNoneAndAnUnsupportedProviderAddsNothing() {
        core.vaultPasskeys = emptyList()
        passkeys.createResult = PasskeyResult.Unsupported
        openVaultPasskeys()
        awaitText("None: add one to unlock on a new phone")
        tap("vaultPasskeysRow")
        awaitTag("noPasskeys")
        tap("addPasskey")
        awaitTag("passkeyError")
        awaitText(PASSKEY_UNSUPPORTED)
        assertTrue(core.passkeyAdds.isEmpty())
    }

    @Test
    fun anAccountWithAMasterPasswordHasNoPasskeyRow() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        launch()
        tap("openSettings")
        awaitCore { core.recoveryCodeReads.get() > 0 }
        assertFalse(has("vaultPasskeysRow"))
    }

    // ---- the WebAuthn JSON -------------------------------------------------------------------------------------------

    private val options = dev.reins.core.VaultPasskeyOptions(
        "app.reins2fa.com", "user-1".toByteArray(), "me@example.com", byteArrayOf(1, 2, 3), byteArrayOf(4, 5, 6),
        listOf(byteArrayOf(7, 8)),
    )

    @Test
    fun theCreationRequestAsksForAResidentKeyWithPrf() {
        val json = org.json.JSONObject(PasskeyJson.creation(options))
        assertEquals("app.reins2fa.com", json.getJSONObject("rp").getString("id"))
        assertEquals("Reins", json.getJSONObject("rp").getString("name"))
        assertEquals(PasskeyJson.b64("user-1".toByteArray()), json.getJSONObject("user").getString("id"))
        assertEquals("me@example.com", json.getJSONObject("user").getString("displayName"))
        assertEquals("AQID", json.getString("challenge"))
        assertEquals(-7, json.getJSONArray("pubKeyCredParams").getJSONObject(0).getInt("alg"))
        assertEquals(-257, json.getJSONArray("pubKeyCredParams").getJSONObject(1).getInt("alg"))
        assertEquals("Bwg", json.getJSONArray("excludeCredentials").getJSONObject(0).getString("id"))
        assertEquals("required", json.getJSONObject("authenticatorSelection").getString("residentKey"))
        assertEquals("required", json.getJSONObject("authenticatorSelection").getString("userVerification"))
        assertEquals("BAUG", json.getJSONObject("extensions").getJSONObject("prf").getJSONObject("eval").getString("first"))

        val get = org.json.JSONObject(PasskeyJson.assertion(options, listOf(byteArrayOf(7, 8))))
        assertEquals("app.reins2fa.com", get.getString("rpId"))
        assertEquals("Bwg", get.getJSONArray("allowCredentials").getJSONObject(0).getString("id"))
        assertEquals("required", get.getString("userVerification"))
        assertEquals("BAUG", get.getJSONObject("extensions").getJSONObject("prf").getJSONObject("eval").getString("first"))
    }

    @Test
    fun responsesGiveThePrfOutputOrSayTheProviderCannot() {
        val prf = ByteArray(32) { 3 }
        fun response(extensions: String) = """{"id":"Bwg","rawId":"Bwg","type":"public-key","clientExtensionResults":$extensions}"""

        val withResult = PasskeyJson.parse(response("""{"prf":{"enabled":true,"results":{"first":"${PasskeyJson.b64(prf)}"}}}"""), created = true)
        assertArrayEquals(byteArrayOf(7, 8), (withResult as PasskeyResult.Passkey).credentialId)
        assertArrayEquals(prf, withResult.prf)

        val onUse = PasskeyJson.parse(response("""{"prf":{"enabled":true}}"""), created = true)
        assertEquals(null, (onUse as PasskeyResult.Passkey).prf)

        assertEquals(PasskeyResult.Unsupported, PasskeyJson.parse(response("""{"prf":{"enabled":false}}"""), created = true))
        assertEquals(PasskeyResult.Unsupported, PasskeyJson.parse(response("{}"), created = true))
        assertEquals(PasskeyResult.Unsupported, PasskeyJson.parse(response("""{"prf":{}}"""), created = false))
        assertTrue(PasskeyJson.parse("not json", created = false) is PasskeyResult.Failed)
    }

    @Test
    fun aPasskeyAddedIsListedWithItsDate() {
        core.vaultPasskeys = listOf(VaultPasskeyView(byteArrayOf(5), "Galaxy S25", 1_700_000_000))
        openVaultPasskeys()
        tap("vaultPasskeysRow")
        awaitText("Galaxy S25")
        assertTrue(showsText("Added ", substring = true))
    }
}
