package dev.reins.android

import androidx.compose.ui.test.*
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.SessionState
import dev.reins.android.ui.permissions.AccountPermissionsViewModel
import dev.reins.core.*
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PrimaryPhoneSetupFlowTest : FlowHarness() {
    private fun legacyView() {
        core.permissionViews["gmail/me@gmail.com"] = PermissionFixtures.view("gmail", "me@gmail.com").let { view ->
            view.copy(availablePresets = listOf(AccountPermissionPreset.REVIEW_EVERY_REQUEST, AccountPermissionPreset.CUSTOM),
                capabilities = view.capabilities.map { it.copy(supportedModes = listOf(PermissionMode.ASK, PermissionMode.BLOCK)) })
        }
    }
    private fun open() {
        legacyView(); launch(); tap("tabConnections"); openService("gmail"); tap("accountPermissions:me@gmail.com")
        awaitTag("primarySetupOpen")
    }
    private fun dialog() { tap("primarySetupOpen"); awaitTag("primarySetupDialog") }

    @Test fun setupRequiresRecoveryBackupThenOwnerAndDoesNotGrantAnyAgentAction() {
        open(); dialog()
        rule.onNodeWithTag("primarySetupConfirm").performScrollTo().assertIsNotEnabled()
        assertTrue(showsText("whole Reins account", substring = true))
        tap("primarySetupBackup"); tap("primarySetupConfirm")
        awaitGone("primarySetupDialog")
        rule.onNodeWithTag("permissionPreset:READ_ASSIST").performScrollTo().assertIsEnabled()
        assertEquals(1, core.setupCreated.get()); assertEquals(1, prompts.get())
        assertTrue(core.savedPermissionDrafts.isEmpty()); assertTrue(core.delegationRecords.isEmpty())
    }

    @Test fun cancelDoesNotCreateAuthorityAndRequiresFreshBackupAcknowledgmentOnReentry() {
        open(); dialog(); tap("primarySetupBackup"); tap("primarySetupCancel"); dialog()
        rule.onNodeWithTag("primarySetupBackup").assertIsOff()
        assertEquals(0, core.setupCalls.get()); assertEquals(0, prompts.get())
    }

    @Test fun cancelledOwnerDoesNotCreateAuthority() {
        open(); dialog(); tap("primarySetupBackup"); authResult = AuthResult.Cancelled; tap("primarySetupConfirm")
        awaitCore { core.setupFinished.get() == 1 }
        assertEquals(0, core.setupCreated.get()); assertTrue(has("primarySetupDialog"))
    }

    @Test fun viewerAndRemovedPhonesGetGuidanceWithoutCreation() {
        legacyView(); core.setupRole = PhoneRole.VIEWER
        core.setupRoster = RosterSummary(1u, PhoneRole.VIEWER, listOf(RosterMemberView("other", "Work phone", true, false)), "fixture", emptyList())
        launch(); tap("tabConnections"); openService("gmail"); tap("accountPermissions:me@gmail.com")
        awaitTag("primarySetupGuidance"); assertFalse(has("primarySetupOpen"))
        assertTrue(showsText("Work phone", substring = true)); assertEquals(0, core.setupCalls.get())
        pressBack(); core.setupRole = PhoneRole.REMOVED; tap("accountPermissions:me@gmail.com")
        awaitTag("primarySetupGuidance"); assertFalse(has("primarySetupOpen")); assertEquals(0, core.setupCalls.get())
    }

    @Test fun typedProofIsOptionalSecureAndClearedAfterEachAttempt() {
        core.setupRequiresProof = true
        open(); dialog(); tap("primarySetupBackup"); tap("primarySetupConfirm")
        awaitTag("primarySetupError"); assertEquals(0, core.setupCreated.get())
        rule.onNodeWithText("Use a recovery code or master password").performScrollTo().performClick()
        rule.onNodeWithTag("primarySetupProof").performTextReplacement("synthetic-fixture-password")
        tap("primarySetupConfirm"); awaitGone("primarySetupDialog")
        assertTrue(core.setupProofUsed.get()); assertEquals(1, core.setupCreated.get())
    }

    @Test fun suppliedWhitespaceProofIsValidatedRatherThanUsingTheSavedSecret() {
        open(); dialog(); tap("primarySetupBackup")
        rule.onNodeWithText("Use a recovery code or master password").performScrollTo().performClick()
        rule.onNodeWithTag("primarySetupProof").performTextReplacement("        ")
        tap("primarySetupConfirm"); awaitTag("primarySetupError")
        assertTrue(core.setupProofUsed.get()); assertEquals(0, core.setupCreated.get())
        // The proof field was cleared after submission; an explicit retry can use the locally retained secret.
        tap("primarySetupConfirm"); awaitGone("primarySetupDialog")
        assertFalse(core.setupProofUsed.get()); assertEquals(1, core.setupCreated.get())
    }

    @Test fun roleChangedDuringOwnerPromptCannotBeTakenOver() {
        legacyView(); launch(); awaitTag("tabConnections")
        val editor = AccountPermissionsViewModel(container, "gmail", "me@gmail.com")
        val setup = editor.primaryPhoneSetup(); setup.check(); awaitCore { setup.ui.value.canCreate }
        setup.enable(true, null, Authenticator { _, _ -> core.setupRole = PhoneRole.VIEWER; AuthResult.Success }) {}
        awaitCore { core.setupFinished.get() == 1 }
        assertEquals(0, core.setupCreated.get()); assertEquals(PhoneRole.VIEWER, core.setupRole)
    }

    @Test fun delayedCorePreparationCannotUseAnOldOwnerPromptInANewAccount() {
        legacyView(); launch(); awaitTag("tabConnections")
        val editor = AccountPermissionsViewModel(container, "gmail", "me@gmail.com")
        val setup = editor.primaryPhoneSetup(); setup.check(); awaitCore { setup.ui.value.canCreate }
        val gate = CompletableDeferred<Unit>(); core.setupAuthorizedGate = gate
        val auth = java.util.concurrent.atomic.AtomicInteger()
        setup.enable(true, "synthetic-shared-password", Authenticator { _, _ -> auth.incrementAndGet(); AuthResult.Success }) {}
        awaitCore { core.setupCalls.get() == 1 }
        container.state.setSession(SessionState.SignedIn(SessionInfo("https://other.example", "other@example.com")))
        gate.complete(Unit); awaitCore { core.setupFinished.get() == 1 }
        assertEquals(0, auth.get()); assertEquals(0, core.setupCreated.get())
    }

    @Test fun navigationCancellationStopsAnOwnerPromptAlreadyInProgress() {
        legacyView(); launch(); awaitTag("tabConnections")
        val editor = AccountPermissionsViewModel(container, "gmail", "me@gmail.com")
        val setup = editor.primaryPhoneSetup(); setup.check(); awaitCore { setup.ui.value.canCreate }
        val entered = CompletableDeferred<Unit>(); val answer = CompletableDeferred<Unit>()
        setup.enable(true, null, Authenticator { _, _ -> entered.complete(Unit); answer.await(); AuthResult.Success }) {}
        awaitCore { entered.isCompleted }; setup.cancelAttempt(); answer.complete(Unit)
        awaitCore { core.setupFinished.get() == 1 }; assertEquals(0, core.setupCreated.get())
    }

    @Test fun accountReplacementClearsDialogProofAndBackupConfirmation() {
        open(); dialog(); tap("primarySetupBackup")
        rule.onNodeWithText("Use a recovery code or master password").performScrollTo().performClick()
        rule.onNodeWithTag("primarySetupProof").performTextReplacement("synthetic-old-account-proof")
        val replacement = SessionInfo("https://other.example", "other@example.com")
        core.session = replacement
        container.state.setSession(SessionState.SignedIn(replacement))
        awaitGone("primarySetupDialog")
        // Account replacement clears all old lists; stand in for the new session's verified refresh.
        container.state.setServices(core.catalogue); container.state.setAccounts(core.accounts)
        tap("tabConnections"); openService("gmail"); tap("accountPermissions:me@gmail.com"); awaitTag("primarySetupOpen"); dialog()
        rule.onNodeWithTag("primarySetupBackup").assertIsOff()
        rule.onNodeWithTag("primarySetupProof").assertDoesNotExist()
        rule.onNodeWithTag("primarySetupConfirm").performScrollTo().assertIsNotEnabled()
        assertEquals(0, core.setupCreated.get())
    }

    @Suppress("DEPRECATION")
    @Test fun largeTextKeepsBackupAndConfirmationReachable() {
        val resources = context.resources
        val previous = android.content.res.Configuration(resources.configuration)
        resources.updateConfiguration(android.content.res.Configuration(previous).apply { fontScale = 1.6f }, resources.displayMetrics)
        try {
            open(); dialog()
            rule.onNodeWithTag("primarySetupConfirm").performScrollTo().assertIsNotEnabled()
            tap("primarySetupBackup"); tap("primarySetupConfirm"); awaitGone("primarySetupDialog")
            assertEquals(1, core.setupCreated.get())
        } finally { resources.updateConfiguration(previous, resources.displayMetrics) }
    }

    @Test fun lostReplyChecksExistingPrimaryWithoutDuplicatingAuthority() {
        open(); dialog(); tap("primarySetupBackup"); core.setupLostReply = true; tap("primarySetupConfirm")
        awaitTag("primarySetupError"); assertEquals(1, core.setupCreated.get())
        tap("primarySetupConfirm"); awaitGone("primarySetupDialog")
        assertEquals(1, core.setupCalls.get()); assertEquals(1, core.setupCreated.get())
    }
}
