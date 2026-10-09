package dev.reins.android

import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onLast
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.core.AccountView
import dev.reins.core.VaultItemKind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The vault on the phone (Integrations > Password vault > Open the vault): the list and its search, an item's secrets
 * only after the screen lock, the names the desktop app uses, adding an API key, making an SSH key, editing, deleting.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class VaultFlowTest : FlowHarness() {
    @Before
    fun vaultConnected() {
        core.resetVault()
        core.accounts = core.accounts + AccountView("vault", "me@example.com", 1_700_000_000)
    }

    private fun openVault() {
        launch()
        tap("integrations")
        tap("service:vault")
        tap("openVault")
        awaitTag("vaultItem:openai")
    }

    private fun type(tag: String, text: String) {
        awaitTag(tag)
        rule.onNodeWithTag(tag).performTextInput(text)
        settle()
    }

    @Test
    fun theListShowsEveryItemAndSearches() {
        openVault()
        assertTrue(has("vaultItem:github") && has("vaultItem:deploy") && has("vaultItem:visa"))
        awaitText("This phone's key for reins vault add: 4821-9930-1274")
        type("vaultSearch", "git")
        awaitGone("vaultItem:openai")
        assertTrue(has("vaultItem:github"))
        type("vaultSearch", "zzz")
        awaitTag("vaultNoMatch")
    }

    @Test
    fun anEmptyVaultSaysWhatToAddAndHowTheComputerNamesIt() {
        core.vaultDetails = emptyList()
        launch()
        tap("integrations")
        tap("service:vault")
        tap("openVault")
        awaitTag("vaultEmpty")
        assertTrue(showsText("vault:OpenAI/password", substring = true))
    }

    @Test
    fun aSecretShowsOnlyAfterTheScreenLockAndTheReferenceIsGiven() {
        openVault()
        tap("vaultItem:openai")
        awaitTag("vaultField:password")
        assertTrue(has("vaultUse:vault:OpenAI/password"))
        assertFalse(showsText("sk-test-0000"))
        authResult = AuthResult.Cancelled
        tap("vaultReveal:password")
        settle()
        assertFalse(showsText("sk-test-0000"))
        assertTrue(core.vaultRevealed.isEmpty())
        authResult = AuthResult.Success
        tap("vaultReveal:password")
        awaitText("sk-test-0000")
        assertEquals(listOf("openai/password"), core.vaultRevealed.toList())
        tap("vaultReveal:password")
        rule.waitUntil(10_000) { !showsText("sk-test-0000") }
    }

    @Test
    fun withoutAScreenLockSecretsStayHidden() {
        openVault()
        tap("vaultItem:github")
        authResult = AuthResult.Unavailable
        tap("vaultCopy:password")
        awaitTag("vaultItemError")
        assertTrue(core.vaultRevealed.isEmpty())
    }

    @Test
    fun anApiKeyIsAddedWithItsName() {
        openVault()
        tap("vaultAdd")
        tap("newItem:ApiKey")
        type("vaultName", "Groq")
        assertTrue(showsText("Use it as vault:Groq/password"))
        type("vaultInput:password", "gsk_123")
        tap("vaultSave")
        awaitCore { core.vaultCreated.isNotEmpty() }
        val input = core.vaultCreated.single()
        assertEquals(VaultItemKind.LOGIN, input.kind)
        assertEquals("Groq", input.name)
        assertEquals(listOf("password" to "gsk_123"), input.fields.map { it.key to it.value })
        awaitTag("vaultUse:vault:Groq/password")
    }

    @Test
    fun anSshKeyIsMadeOnThePhoneAndItsPublicHalfIsShown() {
        openVault()
        tap("vaultAdd")
        tap("newItem:SshKey")
        type("vaultName", "Laptop")
        tap("vaultGenerate")
        awaitTag("sshMade")
        assertTrue(showsText("ssh-ed25519 AAAA", substring = true))
        assertTrue(has("vaultShare:public_key"))
        assertTrue("made by the core, not typed in", core.vaultCreated.isEmpty())
    }

    @Test
    fun anEditSendsOnlyWhatChanged() {
        openVault()
        tap("vaultItem:github")
        tap("vaultEdit")
        awaitTag("vaultInput:username")
        rule.onNodeWithTag("vaultInput:username").performTextReplacement("octo-cat")
        tap("vaultSave")
        awaitCore { core.vaultUpdated.isNotEmpty() }
        val (id, input) = core.vaultUpdated.single()
        assertEquals("github", id)
        assertEquals("the password was left as it was", listOf("username" to "octo-cat"), input.fields.map { it.key to it.value })
    }

    @Test
    fun changesNeedTheScreenLock() {
        openVault()
        tap("vaultAdd")
        tap("newItem:ApiKey")
        type("vaultName", "Groq")
        type("vaultInput:password", "gsk_123")
        authResult = AuthResult.Cancelled
        tap("vaultSave")
        settle()
        assertTrue("nothing is saved without the screen lock", core.vaultCreated.isEmpty())
        authResult = AuthResult.Unavailable
        tap("vaultSave")
        awaitTag("vaultFormError")
        assertTrue(core.vaultCreated.isEmpty())
        pressBack()
        pressBack()
        tap("vaultItem:visa")
        authResult = AuthResult.Cancelled
        tap("vaultDelete")
        awaitText("Delete Visa?")
        rule.onAllNodes(hasText("Delete")).onLast().performClick()
        settle()
        assertTrue("nothing is deleted without the screen lock", core.vaultDeleted.isEmpty())
    }

    @Test
    fun deletingAsksFirst() {
        openVault()
        tap("vaultItem:visa")
        tap("vaultDelete")
        awaitText("Delete Visa?")
        assertTrue(core.vaultDeleted.isEmpty())
        // The dialog's own Delete button comes after the page's.
        rule.onAllNodes(hasText("Delete")).onLast().performClick()
        awaitCore { core.vaultDeleted.toList() == listOf("visa") }
        awaitTag("vaultItem:openai")
        assertFalse(has("vaultItem:visa"))
    }
}
