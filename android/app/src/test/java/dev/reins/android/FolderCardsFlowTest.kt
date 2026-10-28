package dev.reins.android

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasTestTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.FakeCore.FolderProposal
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Foreground
import dev.reins.core.FolderKindChoice
import dev.reins.core.FolderView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The folders a paired computer proposes for its signed policy: question cards in Activity (Trust after the owner
 * check, Not now), never a notification; and the computer's page lists them under Folders.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class FolderCardsFlowTest : FlowHarness() {
    private val code = FolderView("/home/me/code", FolderKindChoice.ROOT)
    /** Long, with spaces and a `*`: shown whole all the same. */
    private val scratch = FolderView("/home/me/Library/Mobile Documents/scratch areas for the build agents/very-long-folder-name/*", FolderKindChoice.SCRATCH)
    private val work = FolderView("/srv/work", FolderKindChoice.ROOT)

    private fun proposal(folder: FolderView, at: Long, id: String = "d1", computer: String = "M2A") = FolderProposal(id, computer, folder, at)

    private fun key(folder: FolderView, id: String = "d1") = "$id:${folder.kind.name}:${folder.path}"

    private fun deep(tag: String) = rule.onNode(hasTestTag(tag), useUnmergedTree = true)

    @Test
    fun aCardShowsThePathWholeAndTrustAsksTheOwner() {
        core.connections = listOf(TestData.computer("d1", "M2A"), TestData.connection())
        core.folderProposals = listOf(proposal(code, 1_700_000_050), proposal(scratch, 1_700_000_040))
        launch()
        awaitTag("folder:${key(code)}")
        deep("folderTitle:${key(code)}").assertTextEquals("Trust this folder?")
        deep("folderPath:${key(code)}").assertTextEquals("/home/me/code")
        deep("folderWho:${key(code)}").assertTextEquals("M2A · Projects")
        deep("folderPath:${key(scratch)}").assertTextEquals(scratch.path)
        deep("folderWho:${key(scratch)}").assertTextEquals("M2A · Scratch")
        // The details are behind the (?).
        assertFalse(has("folderHelpText:${key(code)}"))
        tap("folderHelp:${key(code)}")
        awaitTag("folderHelpText:${key(code)}")
        // Trust loosens: refused, the card comes back and nothing is signed.
        authResult = AuthResult.Cancelled
        tap("trustFolder:${key(code)}")
        awaitCore { prompts.get() == 1 }
        awaitTag("folder:${key(code)}")
        assertTrue(core.folderConfirmCalls.isEmpty())
        authResult = AuthResult.Success
        tap("trustFolder:${key(code)}")
        awaitCore { core.folderConfirmCalls.isNotEmpty() }
        assertEquals("d1" to code, core.folderConfirmCalls.single())
        assertEquals("Trust /home/me/code as projects", core.autoOwnerReasons.last())
        assertEquals("Trust /home/me/code as projects", promptTexts.last().second)
        awaitGone("folder:${key(code)}")
        assertTrue("the other card stays", has("folder:${key(scratch)}"))
    }

    @Test
    fun notNowDismissesWithoutTheOwnerCheck() {
        core.connections = listOf(TestData.computer("d1", "M2A"), TestData.connection())
        core.folderProposals = listOf(proposal(scratch, 1_700_000_040))
        launch()
        tap("notNowFolder:${key(scratch)}")
        awaitCore { core.folderDismissCalls.isNotEmpty() }
        assertEquals("d1" to scratch, core.folderDismissCalls.single())
        awaitGone("folderCards")
        assertEquals(0, prompts.get())
        assertTrue("nothing signed", core.folderConfirmCalls.isEmpty())
    }

    @Test
    fun folderCardsNeverNotify() {
        Foreground.focused = false
        core.connections = listOf(TestData.computer("d1", "M2A"), TestData.connection())
        core.folderProposals = listOf(proposal(code, 1_700_000_050))
        launch()
        awaitTag("folder:${key(code)}")
        // A new proposal arrives with the next read of the waiting items, still without a notification.
        core.folderProposals = core.folderProposals + proposal(work, 1_700_000_060)
        container.refreshAfterAnswer()
        awaitTag("folder:${key(work)}")
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        assertEquals(0, shadowOf(manager).allNotifications.size)
    }

    @Test
    fun aComputersPageListsTrustedFoldersToRemoveAndSuggestedOnesToTrust() {
        core.connections = listOf(TestData.computer("d1", "M2A"), TestData.connection())
        core.folderProposals = listOf(proposal(code, 1_700_000_050), proposal(scratch, 1_700_000_040), proposal(work, 1_700_000_030))
        core.confirmedFolders = mapOf("d1" to listOf(code))
        // Dismissed from Activity: still suggested on the page.
        core.dismissedFolders = setOf("d1" to work)
        launch()
        tap("tabConnections")
        tap("connection:d1")
        awaitTag("computerFolders")
        deep("trustedFolder:${key(code)}:path").assertTextEquals("/home/me/code")
        deep("trustedFolder:${key(code)}:kind").assertTextEquals("Projects")
        deep("suggestedFolder:${key(scratch)}:path").assertTextEquals(scratch.path)
        deep("suggestedFolder:${key(scratch)}:kind").assertTextEquals("Scratch · Suggested")
        assertTrue(has("suggestedFolder:${key(work)}"))
        assertFalse("a trusted folder is not suggested again", has("suggestedFolder:${key(code)}"))
        assertFalse("a trusted folder has no Trust", has("confirmFolder:${key(code)}"))
        // Removing only tightens: at once, no owner check.
        tap("removeFolder:${key(code)}")
        awaitCore { core.folderRemoveCalls.isNotEmpty() }
        assertEquals("d1" to code, core.folderRemoveCalls.single())
        assertEquals(0, prompts.get())
        awaitGone("trustedFolder:${key(code)}")
        // Still proposed by the computer: back among the suggested.
        awaitTag("suggestedFolder:${key(code)}")
        // Trusting a dismissed one asks the owner first.
        tap("confirmFolder:${key(work)}")
        awaitCore { core.folderConfirmCalls.isNotEmpty() }
        assertEquals("d1" to work, core.folderConfirmCalls.single())
        assertEquals(1, prompts.get())
        assertEquals("Trust /srv/work as projects", core.autoOwnerReasons.last())
        awaitTag("trustedFolder:${key(work)}")
        assertFalse(has("suggestedFolder:${key(work)}"))
    }

    @Test
    fun anAiAppsPageHasNoFolders() {
        core.connections = listOf(TestData.connection("c1", "Claude"))
        core.folderProposals = listOf(proposal(code, 1_700_000_050, id = "c1", computer = "Claude"))
        launch()
        tap("tabConnections")
        tap("connection:c1")
        awaitTag("connectionName")
        assertFalse(has("computerFolders"))
        assertEquals("never read for an AI app", 0, core.computerFoldersReads.get())
    }
}
