package dev.reins.android

import androidx.compose.ui.test.*
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.android.ui.gmail.GmailViewModel
import dev.reins.android.ui.permissions.AccountPermissionsViewModel
import dev.reins.android.state.SessionState
import kotlinx.coroutines.CompletableDeferred
import dev.reins.core.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class PermissionControlsFlowTest : FlowHarness() {
    private fun existing() {
        launch(); tap("tabConnections"); openService("gmail"); tap("accountPermissions:me@gmail.com")
        awaitTag("permissionPreset:CUSTOM")
    }
    private fun chooseRead() {
        tap("permissionPreset:READ_ASSIST")
        awaitTag("permissionAgent:c1")
    }
    @Test fun laterPreservesVerifiedAccountAndDoesNotSaveOrRepeatOAuth() {
        launch(); tap("tabConnections"); openService("gmail")
        runBlocking { GmailViewModel(container).connect("new@gmail.com") }
        awaitTag("integrationConnected")
        awaitTag("permissionPreset:REVIEW_EVERY_REQUEST")
        tap("connectionDone")
        assertTrue(core.accounts.any { it.account == "new@gmail.com" })
        assertEquals(1, core.addedAccounts.size)
        assertTrue(core.savedPermissionDrafts.isEmpty())
    }
    @Test fun explicitAdmissionAndPresetReachTheExactCoreDraftAfterOwnerReview() {
        existing(); chooseRead(); tap("permissionAgent:c2"); tap("permissionsPreview")
        awaitTag("permissionPreviewSummary")
        rule.onNodeWithTag("permissionPreviewHistory").performScrollTo().assertTextEquals("A history comparison is unavailable.")
        tap("permissionsSave"); awaitTag("permissionsSaved")
        val draft = core.savedPermissionDrafts.single()
        assertTrue(draft.grantAccountAccess)
        assertEquals(setOf("c1", "c2"), draft.agentIds.toSet())
        assertEquals(AccountPermissionPreset.READ_ASSIST, draft.preset)
        assertEquals(PermissionMode.ASK, draft.rules.first { it.capability == PermissionCapability.MAIL_SEND }.mode)
        assertEquals(1, prompts.get())
    }
    @Test fun reviewListsExactAgentsAdmissionOperationsAndLimitsBeforeOwnerApproval() {
        existing(); chooseRead(); tap("permissionAgent:c2"); tap("permissionsPreview")
        awaitTag("permissionReviewDetails")
        for (text in listOf("Claude Code", "Personal agent", "c1", "c2", "Personal agent can discover this account.", "Search mail: Allow automatically", "Read mail: Allow automatically", "Send mail: Review by default", "At most 20 further uses for each automatic operation.")) {
            rule.onNodeWithText(text).performScrollTo().assertIsDisplayed()
        }
        assertTrue(core.savedPermissionDrafts.isEmpty())
        assertEquals(0, prompts.get())
    }
    @Test fun emptyAgentSelectionTruthfullyAppliesReviewAndBlockDefaultsToAllAgents() {
        existing(); tap("permissionAgent:c1"); tap("permissionMode:MAIL_SEND:BLOCK")
        awaitTag("permissionAllAgents")
        tap("permissionsPreview")
        rule.onNodeWithText("All agents (including future connections)").performScrollTo().assertIsDisplayed()
        rule.onAllNodesWithText("All requests of this operation in this account, from all agents including future connections.")[0].performScrollTo().assertIsDisplayed()
        tap("permissionsSave"); awaitTag("permissionsSaved")
        val draft = core.savedPermissionDrafts.single()
        assertTrue(draft.agentIds.isEmpty()); assertFalse(draft.grantAccountAccess)
        assertEquals(PermissionMode.BLOCK, draft.rules.first { it.capability == PermissionCapability.MAIL_SEND }.mode)
        assertTrue(draft.rules.any { it.mode == PermissionMode.ASK })
        assertTrue(draft.rules.none { it.mode == PermissionMode.AUTO })
    }
    @Test fun cancelledOwnerDoesNotSaveAndKeepsTheDraftEditable() {
        existing(); chooseRead(); tap("permissionsPreview"); authResult = AuthResult.Cancelled
        tap("permissionsSave"); awaitTag("permissionsPreview")
        assertTrue(core.savedPermissionDrafts.isEmpty())
        assertFalse(has("permissionsSaved"))
    }
    @Test fun optionalSaveFailureLeavesAccountConnectedAndRetainsCurrentPolicy() {
        existing(); chooseRead(); tap("permissionsPreview")
        core.permissionFailure = CoreException.Invalid("Could not save permissions")
        tap("permissionsSave"); awaitTag("permissionsError")
        rule.onNodeWithText("Could not save permissions", substring = true).assertExists()
        assertTrue(core.accounts.any { it.account == "me@gmail.com" })
        assertTrue(core.savedPermissionDrafts.isEmpty())
        assertEquals(PermissionMode.ASK, runBlocking { core.accountPermissions("gmail", "me@gmail.com") }.capabilities.first().configuredMode)
    }
    @Test fun concurrentPolicyEditRequiresExplicitReload() {
        existing(); chooseRead()
        core.permissionViews["gmail/me@gmail.com"] = PermissionFixtures.view("gmail", "me@gmail.com", 2u)
        tap("permissionsPreview"); awaitTag("permissionsChanged")
        rule.onNodeWithTag("permissionsPreview").performScrollTo().assertIsNotEnabled()
        assertTrue(core.savedPermissionDrafts.isEmpty())
        tap("permissionsReload"); awaitTag("permissionPreset:CUSTOM")
        chooseRead(); tap("permissionsPreview"); awaitTag("permissionPreviewSummary")
    }
    @Test fun recipientEntrySelectsOnlyTheCoreCanonicalOption() {
        existing(); tap("permissionMode:MAIL_SEND:AUTO"); tap("permissionScope:MAIL_SEND")
        awaitTag("permissionScopeQuery")
        rule.onNodeWithTag("permissionScopeQuery").performTextReplacement("FRIEND@example.com")
        awaitTag("scopeOption:recipient:friend@example.com")
        tap("scopeOption:recipient:friend@example.com"); tap("permissionScopeDone")
        rule.onNodeWithText("Expire in 1 hour").performScrollTo().performClick()
        rule.onNodeWithTag("permissionUses").performScrollTo().performTextReplacement("7")
        tap("permissionsPreview")
        rule.onNodeWithText("recipient:friend@example.com").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Every To and Cc address must match these recipients.").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("At most 7 further uses for each automatic operation.").performScrollTo().assertIsDisplayed()
        tap("permissionsSave"); awaitTag("permissionsSaved")
        val send = core.savedPermissionDrafts.single().rules.first { it.capability == PermissionCapability.MAIL_SEND }
        assertEquals(listOf("recipient:friend@example.com"), send.scope.ids)
        assertEquals(PermissionScopeKind.SELECTED_RECIPIENTS, send.scope.kind)
    }
    @Test fun unsupportedIntegrationShowsEffectivePolicyWithoutOneWayChanges() {
        launch(); tap("tabConnections"); openService("github")
        // Existing connected synthetic account is added through the real finishing path.
        runBlocking { container.state.accountConnected(AccountView("github", "octo-cat", 1), container.state.accountEpoch.value) }
        awaitTag("permissionsReadOnly")
        assertTrue(showsText("Existing permissions, grants and Auto behavior remain in effect", substring = true))
        rule.onNodeWithTag("permissionPreset:CUSTOM").assertDoesNotExist()
        rule.onNodeWithTag("permissionsPreview").assertDoesNotExist()
        rule.onNodeWithTag("accountDelegate").assertDoesNotExist()
        tap("connectionDone")
        assertTrue(core.savedPermissionDrafts.isEmpty())
        assertTrue(core.delegationRecords.isEmpty())
        assertEquals(0, prompts.get())
    }
    @Test fun temporaryPermissionHasExplicitIssuePauseResumeAndRevoke() {
        existing(); tap("accountDelegate"); awaitTag("delegationCapability:MAIL_SEARCH")
        tap("permissionsPreview"); tap("permissionsSave"); awaitTag("permissionsSaved")
        assertEquals(1, core.delegationRecords.size)
        tap("permissionsDone"); pressBack(); pressBack(); tap("tabTrust"); tap("trustPermissions")
        awaitTag("delegation:temporary-0")
        tap("pauseDelegation:temporary-0"); rule.onNode(hasText("Pause") and hasAnyAncestor(isDialog()), useUnmergedTree = true).performClick()
        awaitTag("resumeDelegation:temporary-0")
        tap("resumeDelegation:temporary-0"); rule.onNode(hasText("Resume") and hasAnyAncestor(isDialog()), useUnmergedTree = true).performClick()
        awaitTag("pauseDelegation:temporary-0")
        tap("revokeDelegation:temporary-0"); rule.onNode(hasText("Revoke") and hasAnyAncestor(isDialog()), useUnmergedTree = true).performClick()
        rule.waitUntil(10_000) { core.delegationRecords.single().state == DelegationState.REVOKED }
        assertEquals(4, prompts.get())
    }

    @Test fun oldSessionSaveCannotConfirmOrPublishSuccessAfterLogout() {
        launch(); awaitTag("tabConnections")
        val editor = AccountPermissionsViewModel(container, "gmail", "me@gmail.com")
        awaitCore { editor.ui.value.draft != null && !editor.ui.value.busy }
        editor.preset(AccountPermissionPreset.READ_ASSIST)
        awaitCore { editor.ui.value.draft?.preset == AccountPermissionPreset.READ_ASSIST && !editor.ui.value.busy }
        editor.preview(); awaitCore { editor.ui.value.preview != null && !editor.ui.value.busy }
        val gate = CompletableDeferred<Unit>(); core.permissionSaveGate = gate
        editor.save(dev.reins.android.platform.Authenticator { _, _ -> AuthResult.Success })
        awaitCore { core.permissionSaveCalls.get() == 1 }
        val previousEpoch = container.state.accountEpoch.value
        container.state.setSession(SessionState.SignedOut)
        assertNotEquals(previousEpoch, container.state.accountEpoch.value)
        gate.complete(Unit)
        rule.waitForIdle()
        assertFalse(editor.ui.value.saved)
        assertTrue(core.savedPermissionDrafts.isEmpty())
    }
    @Test fun leavingAndReturningCannotAuthorizeAnOldOwnerPromptOrReplaceTheReloadedDraft() {
        launch(); awaitTag("tabConnections")
        val editor = AccountPermissionsViewModel(container, "gmail", "me@gmail.com")
        awaitCore { editor.ui.value.draft != null && !editor.ui.value.busy }
        editor.preset(AccountPermissionPreset.READ_ASSIST)
        awaitCore { editor.ui.value.draft?.preset == AccountPermissionPreset.READ_ASSIST && !editor.ui.value.busy }
        editor.preview(); awaitCore { editor.ui.value.preview != null && !editor.ui.value.busy }
        val started = CompletableDeferred<Unit>(); val answer = CompletableDeferred<Unit>(); val finished = CompletableDeferred<Unit>()
        editor.save(dev.reins.android.platform.Authenticator { _, _ ->
            started.complete(Unit)
            answer.await()
            finished.complete(Unit)
            AuthResult.Success
        })
        awaitCore { started.isCompleted }
        editor.deactivate(); editor.activate(); editor.reload()
        awaitCore { editor.ui.value.draft?.preset == AccountPermissionPreset.CUSTOM && !editor.ui.value.busy }
        answer.complete(Unit)
        awaitCore { finished.isCompleted }
        rule.waitForIdle()
        assertFalse(editor.ui.value.saved)
        assertEquals(AccountPermissionPreset.CUSTOM, editor.ui.value.draft?.preset)
        assertTrue(core.savedPermissionDrafts.isEmpty())
    }
    @Test fun latePickerResultCannotReplaceANewerQueryOrReopenClosedOptions() {
        launch(); awaitTag("tabConnections")
        val editor = AccountPermissionsViewModel(container, "gmail", "me@gmail.com")
        awaitCore { editor.ui.value.draft != null && !editor.ui.value.busy }
        val gate = CompletableDeferred<Unit>(); core.permissionScopeGates["slow@example.com"] = gate
        editor.scopeOptions(PermissionCapability.MAIL_SEND, "slow@example.com")
        editor.scopeOptions(PermissionCapability.MAIL_SEND, "new@example.com")
        awaitCore { editor.ui.value.options?.options?.firstOrNull()?.label == "new@example.com" }
        gate.complete(Unit); rule.waitForIdle()
        assertEquals("new@example.com", editor.ui.value.options?.options?.first()?.label)
        editor.closeOptions(); assertNull(editor.ui.value.options)
    }
    @Test fun pickerNeverOffersAnOptionBelongingToAnotherAccount() {
        existing()
        val kind = PermissionScopeKind.SELECTED_RECIPIENTS
        core.permissionScopeOverride = PermissionScopeOptions(listOf(kind), listOf(
            PermissionScopeOption("recipient:friend@example.com", "friend@example.com", "", "gmail", "me@gmail.com", kind),
            PermissionScopeOption("recipient:other@example.com", "other@example.com", "", "gmail", "other@gmail.com", kind)), true, null)
        tap("permissionMode:MAIL_SEND:AUTO"); tap("permissionScope:MAIL_SEND")
        awaitTag("scopeOption:recipient:friend@example.com")
        assertFalse(has("scopeOption:recipient:other@example.com"))
    }

    @Test fun unverifiedUsageDisablesPositiveChoicesButAllowsReviewToBeSaved() {
        core.permissionViews["gmail/me@gmail.com"] = PermissionFixtures.view("gmail", "me@gmail.com").copy(usageVerified = false, used = 999u, remainingUses = 888u)
        existing()
        rule.onNodeWithTag("permissionPreset:READ_ASSIST").performScrollTo().assertIsNotEnabled()
        rule.onNodeWithTag("permissionMode:MAIL_READ:AUTO").performScrollTo().assertIsNotEnabled()
        assertTrue(showsText("Usage and remaining quota are unavailable.", substring = true))
        tap("permissionPreset:REVIEW_EVERY_REQUEST"); tap("permissionsPreview"); tap("permissionsSave"); awaitTag("permissionsSaved")
        assertTrue(core.savedPermissionDrafts.single().rules.none { it.mode == PermissionMode.AUTO })
    }
    @Test fun temporaryDurationChoicesUseTheCoreDelegationBound() {
        core.permissionViews["gmail/me@gmail.com"] = PermissionFixtures.view("gmail", "me@gmail.com").copy(delegationMaximumDurationSecs = 3600u)
        existing(); tap("accountDelegate"); awaitTag("delegationCapability:MAIL_SEARCH")
        assertTrue(showsText("Expire in 1 hour"))
        assertFalse(showsText("Expire in 1 day"))
        assertFalse(showsText("Expire in 7 days"))
    }
    @Test fun unknownUsageDoesNotShowFallbackCountersOrOfferResume() {
        runBlocking {
            val issued = core.issueDelegation(core.delegationDraft("gmail", "me@gmail.com"), object : OwnerCheck { override suspend fun confirm(reason: String) = true })
            core.delegationRecords.clear(); core.delegationRecords += issued.copy(state = DelegationState.PAUSED, usageVerified = false, used = 999u, unresolved = 888u)
        }
        launch(); tap("tabTrust"); tap("trustPermissions"); awaitTag("delegation:temporary-0")
        rule.onNodeWithTag("resumeDelegation:temporary-0").performScrollTo().assertIsNotEnabled()
        rule.onNodeWithTag("revokeDelegation:temporary-0").performScrollTo().assertIsEnabled()
        assertTrue(showsText("Usage and quota unavailable", substring = true))
        assertFalse(showsText("999", substring = true)); assertFalse(showsText("888", substring = true))
    }

    @Test fun customizingAPresetKeepsItsCoreRulesLimitsAndExplicitAgents() {
        existing(); chooseRead(); tap("permissionAgent:c2"); tap("permissionPreset:CUSTOM")
        tap("permissionsPreview"); tap("permissionsSave"); awaitTag("permissionsSaved")
        val draft = core.savedPermissionDrafts.single()
        assertEquals(AccountPermissionPreset.CUSTOM, draft.preset)
        assertEquals(PermissionMode.AUTO, draft.rules.first { it.capability == PermissionCapability.MAIL_READ }.mode)
        assertEquals(20uL, draft.maxUses)
        assertTrue(draft.agentIds.contains("c2")); assertTrue(draft.grantAccountAccess)
    }
}
