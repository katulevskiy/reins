package dev.reins.android

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.TextLayoutResult
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.SecretsRevoked
import dev.reins.core.SecretsScopeChoice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Reins Secrets on the phone: environments released for a command, values saved from the computer, revoking. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class SecretsFlowTest : FlowHarness() {
    private val longCommand =
        "bash -c 'set -euo pipefail; export NODE_ENV=development; npm ci --prefer-offline --no-audit && " +
            "npm run build -- --mode development && node scripts/seed.js --reset --users 25 && npm run dev -- --port 3000 --host 0.0.0.0'"

    private fun layoutOf(tag: String): TextLayoutResult {
        val results = mutableListOf<TextLayoutResult>()
        rule.onNodeWithTag(tag).fetchSemanticsNode().config[SemanticsActions.GetTextLayoutResult].action!!.invoke(results)
        return results.single()
    }

    private fun hasUnmerged(tag: String): Boolean {
        settle()
        return rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    // ---- an environment for a command ---------------------------------------------------------------------------------

    @Test
    fun anEnvReleaseShowsTheWholeCommandAndTheVariablesNames() {
        openRequest(TestData.envReleaseView(TestData.envRelease(command = longCommand, purpose = "Seed and run the dev server")))
        awaitTag("envRelease")
        rule.onNodeWithTag("what").assertTextEquals("Use 3 variables of shop · dev")
        rule.onNodeWithTag("envRunner").assertTextEquals("You")
        assertTrue(has("envPerson"))
        rule.onNodeWithTag("envCommand").assertTextEquals(longCommand)
        // Never cut: it wraps over as many lines as it needs.
        val command = layoutOf("envCommand")
        assertFalse(command.hasVisualOverflow)
        assertTrue(command.lineCount > 2)
        assertFalse(has("envWarning"))
        rule.onNodeWithTag("envWhere").assertTextEquals("shop · dev")
        assertTrue(has("envVar:DATABASE_URL") && has("envVar:STRIPE_KEY") && has("envVar:SESSION_SECRET"))
        rule.onNodeWithTag("envRoot").assertTextEquals("/home/me/code/shop")
        rule.onNodeWithTag("envLease").assertTextEquals("15 minutes")
        rule.onNodeWithTag("envPurpose").assertTextContains("Seed and run the dev server", substring = true)
        rule.onNodeWithTag("approve").assertTextEquals("Approve")
        assertTrue(has("lifetime:HOUR"))
    }

    @Test
    fun anAgentRunningAProgramFromAnOddPlaceIsFlagged() {
        val release = TestData.envRelease(
            runner = "agent:claude-code", runnerLabel = "Claude Code", programWarning = "In a temporary folder",
            program = "/tmp/build-8841/node_modules/.bin/npm", command = "/tmp/build-8841/node_modules/.bin/npm run dev",
        )
        openRequest(TestData.envReleaseView(release))
        awaitTag("envWarning")
        rule.onNodeWithTag("envWarning").assertTextEquals("In a temporary folder")
        rule.onNodeWithTag("envProgram").assertTextEquals("/tmp/build-8841/node_modules/.bin/npm")
        rule.onNodeWithTag("envRunner").assertTextEquals("Claude Code")
        assertTrue(has("envAgent"))
        tap("help:programWarning")
        awaitTag("helpBubble")
    }

    @Test
    fun everyVariableSaysSoAndManyNamesFoldIntoAPage() {
        openRequest(TestData.envReleaseView(TestData.envRelease(allVariables = true)))
        awaitTag("envVariables")
        rule.onNodeWithTag("envVariables").assertTextEquals("Every variable")
        rule.onNodeWithTag("what").assertTextContains("every variable", substring = true)
    }

    @Test
    fun manyVariablesFoldIntoAPage() {
        val names = (1..12).map { "VAR_$it" }
        openRequest(TestData.envReleaseView(TestData.envRelease(variables = names)))
        awaitTag("envVariablesMore")
        rule.onNodeWithTag("envVariablesMore").assertTextEquals("+4")
        assertFalse(has("envVar:VAR_9"))
        tap("envVariablesMore")
        awaitTag("envVariablesAll")
        rule.onNodeWithTag("envVariablesAll").assertTextContains("VAR_12", substring = true)
    }

    @Test
    fun onceRemembersNothing() {
        openRequest(TestData.envReleaseView())
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun rememberingCoversTheExactCommandByDefault() {
        openRequest(TestData.envReleaseView())
        tap("lifetime:DAY")
        assertFalse(has("envScopeNote"))
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(86_400uL, standing.durationSecs)
        assertEquals(SecretsScopeChoice("exact", false), standing.scope.secrets)
    }

    @Test
    fun aWiderCommandPatternAndEveryVariableArePickedInTheSheet() {
        openRequest(TestData.envReleaseView())
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        awaitTag("rememberSheet")
        tap("moreToggle")
        awaitTag("envCommands")
        assertTrue(has("envCommand:exact") && has("envCommand:prefix:1"))
        tap("envCommand:prefix:2")
        tap("envAllVariables")
        tap("rememberConfirm")
        awaitGone("rememberSheet")
        awaitTag("envScopeNote")
        rule.onNodeWithTag("envScopeNote").assertTextContains("npm run dev * · every variable", substring = true)
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(3_600uL, standing.durationSecs)
        assertEquals(SecretsScopeChoice("prefix:2", true), standing.scope.secrets)
    }

    @Test
    fun anEnvironmentAskedEveryTimeOffersNoRemember() {
        openRequest(TestData.envReleaseView(TestData.envRelease(env = "production", alwaysAsk = true, lease = 60u)))
        awaitTag("noStanding")
        assertFalse(has("lifetime:HOUR"))
        assertFalse(has("lifetimeCustom"))
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun aCommandThatCannotBeRememberedOffersNoRemember() {
        openRequest(TestData.envReleaseView(TestData.envRelease(commands = emptyList())))
        awaitTag("noStanding")
        assertFalse(has("lifetime:DAY"))
        assertFalse(has("alwaysAskToggle"))
    }

    @Test
    fun aPrintedValueIsAskedEveryTime() {
        openRequest(TestData.envReleaseView(TestData.envRelease(variables = listOf("STRIPE_KEY"), print = true, command = "reins secrets get STRIPE_KEY")))
        awaitTag("noStanding")
        rule.onNodeWithTag("what").assertTextEquals("Print a value from shop · dev")
        rule.onNodeWithTag("askDetails", useUnmergedTree = true).assertTextEquals("STRIPE_KEY")
        rule.onNodeWithTag("envLease").assertTextEquals("Printed")
        assertFalse(has("alwaysAskToggle"))
    }

    @Test
    fun askEveryTimeIsSavedAndStopsRemembering() {
        openRequest(TestData.envReleaseView())
        tap("lifetime:DAY")
        tap("alwaysAskToggle")
        awaitCore { core.alwaysAsk[("shop" to "dev")] == true }
        awaitTag("noStanding")
        assertFalse(has("lifetime:DAY"))
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    // ---- values from the computer -------------------------------------------------------------------------------------

    @Test
    fun aStoreShowsItsCheckDigitsAndWhatChangesNeverValues() {
        openRequest(TestData.secretsStoreView())
        awaitTag("storeCheck")
        rule.onNodeWithTag("storeCheck").assertContentDescriptionEquals("4821")
        rule.onNodeWithTag("what").assertTextEquals("Save 9 secrets in your vault")
        rule.onNodeWithTag("storeEnvName:0", useUnmergedTree = true).assertTextEquals("shop · dev")
        assertFalse(hasUnmerged("storeNew:0"))
        assertTrue(hasUnmerged("storeNew:1"))
        rule.onNodeWithTag("storeCount:0:added").assertContentDescriptionEquals("2 added")
        rule.onNodeWithTag("storeCount:0:removed").assertContentDescriptionEquals("1 removed")
        rule.onNodeWithTag("storeCount:1:copied").assertContentDescriptionEquals("4 copied")
        assertFalse(has("storeCount:1:removed"))
        assertFalse(hasUnmerged("storeNames:0:renamed"))
        tap("storeEnv:0")
        rule.waitUntil(10_000) { hasUnmerged("storeNames:0:renamed") }
        rule.onNodeWithTag("storeNames:0:renamed", useUnmergedTree = true).assertTextEquals("DB_URL → DATABASE_URL")
        rule.onNodeWithTag("storeNames:0:added", useUnmergedTree = true).assertTextEquals("SENTRY_DSN, OPENAI_API_KEY")
        rule.onNodeWithTag("storeSsh:0").assertTextEquals("Deploy key")
        rule.onNodeWithTag("storeSshFp:0").assertTextEquals("SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8")
        rule.onNodeWithTag("storeToken:0:host").assertTextEquals("github.com")
        rule.onNodeWithTag("storeToken:0:scopes").assertTextEquals("repo, workflow, read:org")
        // One approval, asked every time, and not shown as a danger.
        awaitTag("noStanding")
        assertFalse(has("onceWarning"))
        rule.onNodeWithTag("approve").assertTextEquals("Save")
        tap("help:check:storeCheck")
        awaitText("Compare with the terminal", substring = true)
    }

    @Test
    fun savingAStoreSendsNoStanding() {
        openRequest(TestData.secretsStoreView())
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("req44", core.approvals.single().first)
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun aTokenShowsItsHostScopesAndCheckDigits() {
        openRequest(TestData.tokenStoreView())
        awaitTag("tokenCheck")
        rule.onNodeWithTag("tokenCheck").assertContentDescriptionEquals("7302")
        rule.onNodeWithTag("token:host").assertTextEquals("github.com")
        rule.onNodeWithTag("token:scopes").assertTextEquals("repo, workflow, read:org")
        rule.onNodeWithTag("what").assertTextContains("GitHub token", substring = true)
    }

    // ---- revoking on every computer ---------------------------------------------------------------------------------------

    private fun confirmRevoke() {
        rule.onAllNodes(hasText("Revoke")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
    }

    @Test
    fun aSecretsGrantIsRevokedOnAllComputersAndSaysWhichAreNotToldYet() {
        core.grants = listOf(TestData.secretsGrant())
        core.secretsRevokeAnswer = SecretsRevoked(1u, 2u, listOf("work-laptop"))
        launch()
        tap("tabTrust")
        tap("grant:s1")
        tap("revokeEverywhere")
        confirmRevoke()
        awaitCore { core.secretsRevoked.toList() == listOf<Pair<String?, String?>>("shop" to "dev") }
        awaitTag("notDelivered:0")
        rule.onNodeWithTag("notDelivered:0").assertTextEquals("Not yet delivered to work-laptop")
        rule.onNodeWithTag("revokeTold").assertTextEquals("Revoked · 2 computers told")
    }

    @Test
    fun otherGrantsHaveNoRevokeOnAllComputers() {
        core.grants = listOf(TestData.grant("g1"))
        launch()
        tap("tabTrust")
        tap("grant:g1")
        awaitTag("revoke")
        assertFalse(has("revokeEverywhere"))
    }

    @Test
    fun anEnvReleaseInActivityCanBeRevokedOnAllComputers() {
        core.activity = listOf(TestData.entry(7, action = "write", outcome = "released", service = "vault", op = "env_release"))
        core.entryApprovals[7L] = TestData.envReleaseView().copy(waitUntil = null)
        launch()
        tap("entry:7")
        awaitTag("entryApproval")
        awaitTag("envRelease")
        assertFalse("read-only: nothing to change", has("alwaysAskToggle"))
        tap("revokeEverywhere")
        confirmRevoke()
        awaitCore { core.secretsRevoked.toList() == listOf<Pair<String?, String?>>("shop" to "dev") }
        awaitTag("revokeTold")
        assertFalse(has("notDelivered:0"))
    }
}
