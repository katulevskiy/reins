package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.state.SessionState
import dev.reins.android.ui.gmail.GmailViewModel
import dev.reins.android.ui.services.ServiceViewModel
import dev.reins.core.CoreException
import dev.reins.core.McpAddStep
import dev.reins.core.SessionInfo
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class IntegrationCompletionFlowTest : FlowHarness() {
    @Before fun resetConnectionCalls() {
        core.addedAccounts.clear()
        core.serviceAdded.clear()
        core.tokenAccountGate = null
    }
    private fun launchService(id: String) {
        launch()
        tap("tabConnections")
        openService(id)
    }

    @Test fun verifiedGoogleIdentityHasAnExplicitFinishAndAddAnotherDoesNotReconnect() {
        launchService("gmail")
        runBlocking { GmailViewModel(container).connect("Work@Gmail.com") }
        awaitTag("integrationConnected")
        rule.onNodeWithTag("connectedIdentity").assertTextContains("work@gmail.com")
        assertFalse(has("addAccount"))
        tap("connectionAddAnother")
        awaitTag("addAccount")
        assertEquals(listOf("Work@Gmail.com"), core.addedAccounts.toList())
        assertTrue(core.accounts.any { it.account == "work@gmail.com" })
    }

    @Test fun connectionSurvivesScreenReentryAndActivityRecreationUntilDone() {
        launchService("gmail")
        runBlocking { GmailViewModel(container).connect("new@gmail.com") }
        awaitTag("integrationConnected")
        pressBack()
        openService("gmail")
        awaitTag("integrationConnected")
        scenario!!.recreate()
        awaitTag("integrationConnected")
        tap("connectionDone")
        openService("gmail")
        awaitTag("addAccount")
        assertFalse(has("integrationConnected"))
        assertEquals(1, core.addedAccounts.size)
    }

    @Test fun verifiedTokenAccountFinishesWithoutKeepingTheSecretFormVisible() {
        launchService("github")
        tap("pasteManually")
        rule.onNodeWithTag("secret").performTextReplacement("ghp_fixture_token")
        tap("connectSecret")
        awaitTag("integrationConnected")
        rule.onNodeWithTag("connectedIdentity").assertTextContains("octo-cat")
        assertFalse(has("secret"))
        assertEquals(1, core.tokensAdded.size)
    }

    @Test fun rejectedTokenNeverShowsCompletion() {
        core.serviceFailure = CoreException.Invalid("Token rejected")
        launchService("github")
        tap("pasteManually")
        rule.onNodeWithTag("secret").performTextReplacement("bad_fixture")
        tap("connectSecret")
        awaitTag("accountError")
        assertFalse(has("integrationConnected"))
        assertTrue(container.state.connectedAccounts.value.isEmpty())
    }

    @Test fun telegramCodeAndDevicePermissionsUseVerifiedIdentities() {
        launchService("telegram")
        rule.onNodeWithTag("phone").performTextReplacement("+15550199")
        tap("sendCode")
        rule.onNodeWithTag("code").performTextReplacement("12345")
        tap("submitCode")
        awaitTag("integrationConnected")
        rule.onNodeWithTag("connectedIdentity").assertTextContains("+15550199")
        tap("connectionDone")
        openService("device_contacts")
        ServiceViewModel(container, "device_contacts").addDevice()
        awaitTag("integrationConnected")
        rule.onNodeWithTag("connectedIdentity").assertTextContains("this phone")
        assertFalse(has("connectionAddAnother"))
    }

    @Test fun cancelledGoogleConsentIsConsumedWithoutRegisteringAnAccount() {
        launch()
        val gmail = GmailViewModel(container)
        // Stand in for the external Google consent screen, without a network or Play services account.
        GmailViewModel::class.java.getDeclaredField("awaitingConsent").apply { isAccessible = true }
            .set(gmail, "cancelled@gmail.com" to container.state.accountEpoch.value)
        gmail.consentFinished(false)
        gmail.consentFinished(true)
        settle()
        assertTrue(core.addedAccounts.isEmpty())
        assertTrue(container.state.connectedAccounts.value.isEmpty())
    }

    @Test fun consentFromAnotherSessionCannotRegisterOrReportSuccess() {
        launch()
        val contacts = ServiceViewModel(container, "gcontacts")
        ServiceViewModel::class.java.getDeclaredField("awaitingConsent").apply { isAccessible = true }
            .set(contacts, "old@gmail.com" to container.state.accountEpoch.value)
        container.state.setSession(SessionState.SignedIn(SessionInfo("https://other.example", "other@example.com")))
        contacts.consentFinished(true)
        settle()
        assertTrue(core.serviceAdded.isEmpty())
        assertTrue(container.state.connectedAccounts.value.isEmpty())
    }

    @Test fun lateVerifiedResultDoesNotCreateCompletionInAnotherSession() {
        launch()
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        core.tokenAccountGate = gate
        val vm = ServiceViewModel(container, "github")
        vm.addSecret("fixture_token")
        awaitCore { core.tokensAdded.isNotEmpty() }
        container.state.setSession(SessionState.SignedIn(SessionInfo("https://other.example", "other@example.com")))
        gate.complete(Unit)
        awaitCore { !vm.busy.value }
        assertTrue(container.state.connectedAccounts.value.isEmpty())
        core.tokenAccountGate = null
    }

    @Test fun oldRemovalCannotDismissTheSameAccountCompletionInANewSession() {
        launch()
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        core.removeAccountGate = gate
        val vm = GmailViewModel(container)
        vm.remove("same@gmail.com")
        awaitCore { core.removedAccounts.contains("same@gmail.com") }
        try {
            container.state.setSession(SessionState.SignedIn(SessionInfo("https://other.example", "other@example.com")))
            container.state.accountConnected("gmail", "same@gmail.com", container.state.accountEpoch.value)
            gate.complete(Unit)
            awaitCore { !vm.busy.value }
            assertEquals("same@gmail.com", container.state.connectedAccounts.value["gmail"])
        } finally {
            gate.complete(Unit)
            core.removeAccountGate = null
        }
    }

    @Test fun removalAndSessionChangeClearOnlyTheirOwnCompletion() {
        launch()
        // Signing out below only starts a new account epoch from a signed-in session.
        awaitCore { container.state.session.value is SessionState.SignedIn }
        runBlocking { GmailViewModel(container).connect("new@gmail.com") }
        ServiceViewModel(container, "device_contacts").addDevice()
        awaitCore { container.state.connectedAccounts.value.size == 2 }
        GmailViewModel(container).remove("new@gmail.com")
        awaitCore { container.state.connectedAccounts.value["gmail"] == null }
        assertEquals("this phone", container.state.connectedAccounts.value["device_contacts"])
        val epoch = container.state.accountEpoch.value
        container.state.setSession(SessionState.SignedOut)
        container.state.accountConnected("gmail", "stale@gmail.com", epoch)
        container.state.serverConnected(TestData.mcpServer(), epoch)
        assertTrue(container.state.connectedAccounts.value.isEmpty())
        assertTrue(container.state.connectedServers.value.isEmpty())
    }

    @Test fun oldMcpCompletionCannotNavigateANewAddScreen() {
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        core.mcpAddGate = gate
        core.mcpNextAdd = McpAddStep.Added(TestData.mcpServer())
        launch()
        tap("tabConnections")
        tap("connect")
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.linear.app/mcp")
        tap("mcpAddSubmit")
        awaitCore { core.mcpAdds.isNotEmpty() }
        try {
            pressBack()
            tap("connect")
            tap("addMcp")
            gate.complete(Unit)
            awaitCore { container.state.connectedServers.value["linear"] != null }
            assertTrue(has("mcpUrl"))
            assertFalse(has("mcpDetail"))
            assertEquals(1, core.mcpAdds.size)
        } finally {
            gate.complete(Unit)
            core.mcpAddGate = null
        }
    }

    @Test fun addedMcpWithErrorNeverClaimsConnected() {
        core.mcpNextAdd = McpAddStep.Added(TestData.mcpServer(status = "error", error = "Tools unavailable"))
        launch()
        tap("tabConnections")
        tap("connect")
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.example.com/mcp")
        tap("mcpAddSubmit")
        awaitTag("mcpDetail")
        awaitTag("mcpServerError")
        assertFalse(has("integrationConnected"))
        assertTrue(container.state.connectedServers.value.isEmpty())
    }

    @Test fun verifiedMcpRemainsConnectedWhenSubsequentListRefreshFails() {
        core.mcpNextAdd = McpAddStep.Added(TestData.mcpServer())
        launch()
        tap("tabConnections")
        tap("connect")
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.linear.app/mcp")
        core.mcpListFailure = CoreException.Invalid("List refresh unavailable")
        try {
            tap("mcpAddSubmit")
            awaitTag("integrationConnected")
            settle()
            assertEquals(1, core.mcpAdds.size)
            assertFalse(has("mcpUrl"))
        } finally {
            core.mcpListFailure = null
        }
    }

    @Test fun usableMcpHasExplicitCompletionRetainedAcrossReentry() {
        core.mcpNextAdd = McpAddStep.Added(TestData.mcpServer())
        launch()
        tap("tabConnections")
        tap("connect")
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.linear.app/mcp")
        tap("mcpAddSubmit")
        awaitTag("integrationConnected")
        rule.onNodeWithTag("connectedIdentity").assertTextContains("Linear · mcp.linear.app")
        pressBack()
        tap("mcp:linear")
        awaitTag("integrationConnected")
        tap("connectionDone")
        assertFalse(has("integrationConnected"))
        assertEquals(1, core.mcpAdds.size)
    }
}
