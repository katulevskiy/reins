package dev.rewarden.android

import android.content.Intent
import android.net.Uri
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.platform.McpRedirectActivity
import dev.rewarden.core.CoreException
import dev.rewarden.core.McpAddStep
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class McpFlowTest : FlowHarness() {
    private val redirect = "com.reins2fa.app://mcp-oauth?code=abc123&state=st4te"

    private fun redirectIntent(uri: String = redirect) = Intent(context, MainActivity::class.java)
        .setAction(McpRedirectActivity.ACTION_SIGNED_IN)
        .setData(Uri.parse(uri))

    private fun openIntegrations() {
        launch()
        tap("integrations")
        awaitTag("addMcp")
    }

    // ---- the list -------------------------------------------------------------------------------------------------

    @Test
    fun theIntegrationsScreenListsMcpServersWithHostStatusAndTools() {
        core.mcp = listOf(
            TestData.mcpServer(),
            TestData.mcpServer("notion", "Notion", "https://mcp.notion.com/mcp?key=secret", status = "needs_sign_in", tools = emptyList()),
            TestData.mcpServer("broken", "Broken", "https://broken.example.com/mcp", status = "error", error = "The server did not answer", tools = emptyList()),
        )
        openIntegrations()
        awaitTag("mcp:linear")
        rule.onNodeWithTag("mcp:linear").assertTextContains("Linear")
        rule.onNodeWithTag("mcp:linear").assertTextContains("mcp.linear.app", substring = true)
        rule.onNodeWithTag("mcp:linear").assertTextContains("4 tools", substring = true)
        rule.onNodeWithTag("mcpStatus:linear").assertTextContains("Connected")
        rule.onNodeWithTag("mcpStatus:notion").assertTextContains("Needs sign-in")
        rule.onNodeWithTag("mcpStatus:broken").assertTextContains("Error")
        // The address is shown without its query, which could carry a key.
        assertFalse(showsText("secret", substring = true))
    }

    @Test
    fun withoutServersTheSectionSaysHowToAddOne() {
        openIntegrations()
        awaitTag("noMcp")
    }

    // ---- adding ---------------------------------------------------------------------------------------------------

    @Test
    fun aServerThatNeedsNoSignInIsAddedAndItsToolsAreShown() {
        core.mcpNextAdd = McpAddStep.Added(TestData.mcpServer())
        openIntegrations()
        tap("addMcp")
        awaitTag("mcpUrl")
        rule.onNodeWithTag("mcpAddSubmit").assertIsNotEnabled()
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.linear.app/mcp")
        rule.onNodeWithTag("mcpName").performTextReplacement("Linear")
        rule.onNodeWithTag("mcpAddSubmit").assertIsEnabled()
        tap("mcpAddSubmit")
        awaitTag("mcpDetail")
        assertEquals(listOf("https://mcp.linear.app/mcp" to "Linear"), core.mcpAdds.toList())
        awaitTag("tool:search_issues")
        assertTrue(core.mcpTokenAdds.isEmpty())
    }

    @Test
    fun anEmptyNameIsNotSent() {
        openIntegrations()
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("  https://mcp.linear.app/mcp  ")
        tap("mcpAddSubmit")
        awaitCore { core.mcpAdds.isNotEmpty() }
        assertEquals("https://mcp.linear.app/mcp" to null, core.mcpAdds.single())
    }

    @Test
    fun aTokenAddsTheServerWithItAndTheFieldIsEmptied() {
        openIntegrations()
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.example.com/mcp")
        rule.onNodeWithTag("mcpToken").performTextReplacement("tok_123")
        tap("mcpAddSubmit")
        awaitTag("mcpDetail")
        assertEquals(listOf(listOf("https://mcp.example.com/mcp", "tok_123", null)), core.mcpTokenAdds.toList())
        assertTrue(core.mcpAdds.isEmpty())
    }

    @Test
    fun aFailureIsShownInPlainWords() {
        core.mcpFailure = CoreException.Invalid("That address is not an MCP server")
        openIntegrations()
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://example.com")
        tap("mcpAddSubmit")
        awaitTag("mcpError")
        awaitText("That address is not an MCP server")
    }

    // ---- signing in -----------------------------------------------------------------------------------------------

    @Test
    fun aServerThatNeedsSignInOpensItsPageAndTheRedirectFinishesIt() {
        core.mcpNextAdd = McpAddStep.NeedsSignIn("linear", "https://linear.app/oauth/authorize?client_id=x&state=st4te")
        openIntegrations()
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.linear.app/mcp")
        tap("mcpAddSubmit")
        awaitTag("mcpSigningIn")
        val opened = nextStarted()
        assertNotNull(opened)
        assertEquals(Intent.ACTION_VIEW, opened!!.action)
        assertEquals("https://linear.app/oauth/authorize?client_id=x&state=st4te", opened.dataString)

        // The browser comes back through the redirect activity, which hands the address to the app.
        relaunch(redirectIntent())
        awaitCore { core.mcpSignIns.isNotEmpty() }
        assertEquals("linear" to redirect, core.mcpSignIns.single())
        awaitTag("mcpDetail")
        awaitTag("tool:create_issue")
        rule.onNodeWithTag("mcpStatus").assertTextContains("Connected")
    }

    @Test
    fun aRedirectNobodyWaitsForIsIgnored() {
        launch(redirectIntent())
        awaitTag("integrations")
        settle()
        assertTrue(core.mcpSignIns.isEmpty())
    }

    @Test
    fun aRedirectIsUsedOnlyOnce() {
        core.mcp = listOf(TestData.mcpServer(status = "needs_sign_in", tools = emptyList()))
        container.mcpSignIn.begin("linear")
        launch(redirectIntent())
        awaitCore { core.mcpSignIns.size == 1 }
        relaunch(redirectIntent())
        awaitTag("integrations")
        settle()
        assertEquals(1, core.mcpSignIns.size)
    }

    @Test
    fun aFailedSignInIsExplained() {
        core.mcp = listOf(TestData.mcpServer(status = "needs_sign_in", tools = emptyList()))
        container.mcpSignIn.begin("linear")
        core.mcpFailure = CoreException.Invalid("The sign-in was cancelled")
        launch(redirectIntent())
        awaitTag("mcpDetail")
        awaitTag("mcpNotice")
        awaitText("The sign-in was cancelled", substring = true)
    }

    @Test
    fun somethingThatIsNotTheRedirectIsNotSent() {
        core.mcp = listOf(TestData.mcpServer(status = "needs_sign_in", tools = emptyList()))
        container.mcpSignIn.begin("linear")
        launch(redirectIntent("https://evil.example.com/mcp-oauth?code=x"))
        awaitTag("integrations")
        settle()
        assertTrue(core.mcpSignIns.isEmpty())
        // The sign-in still waits for the real redirect.
        assertEquals("linear", container.mcpSignIn.pending())
    }

    @Test
    fun theRedirectActivityHandsTheAddressToTheAppAndCloses() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(redirect)).setClass(context, McpRedirectActivity::class.java)
        val activity = org.robolectric.Robolectric.buildActivity(McpRedirectActivity::class.java, view).create().get()
        val forwarded = nextStarted()
        assertNotNull(forwarded)
        assertEquals(MainActivity::class.java.name, forwarded!!.component?.className)
        assertEquals(McpRedirectActivity.ACTION_SIGNED_IN, forwarded.action)
        assertEquals(redirect, forwarded.dataString)
        assertTrue(forwarded.flags and Intent.FLAG_ACTIVITY_CLEAR_TOP != 0)
        assertTrue(forwarded.flags and Intent.FLAG_ACTIVITY_SINGLE_TOP != 0)
        assertTrue(activity.isFinishing)
    }

    @Test
    fun theManifestRoutesTheRedirectToTheRedirectActivity() {
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(redirect)).addCategory(Intent.CATEGORY_BROWSABLE)
        val match = context.packageManager.queryIntentActivities(view, 0)
        assertEquals(listOf(McpRedirectActivity::class.java.name), match.map { it.activityInfo.name })
    }

    @Test
    fun aServerWhoseSignInEndedCanSignInAgainFromItsPage() {
        core.mcp = listOf(TestData.mcpServer(status = "needs_sign_in", tools = emptyList()))
        core.mcpNextRefresh = McpAddStep.NeedsSignIn("linear", "https://linear.app/oauth/authorize?again=1")
        openIntegrations()
        tap("mcp:linear")
        tap("mcpSignIn")
        awaitCore { core.mcpRefreshes.isNotEmpty() }
        rule.waitUntil(10_000) { nextStarted()?.dataString == "https://linear.app/oauth/authorize?again=1" }
        assertEquals("linear", container.mcpSignIn.pending())
    }

    @Test
    fun anAuthorizePageThatIsNotWebIsNeverOpened() {
        core.mcpNextAdd = McpAddStep.NeedsSignIn("linear", "intent://evil#Intent;end")
        openIntegrations()
        tap("addMcp")
        rule.onNodeWithTag("mcpUrl").performTextReplacement("https://mcp.linear.app/mcp")
        tap("mcpAddSubmit")
        awaitTag("mcpError")
        assertNull(nextStarted())
    }

    // ---- one server -----------------------------------------------------------------------------------------------

    @Test
    fun theDetailShowsEachToolWithWhatItDoes() {
        core.mcp = listOf(TestData.mcpServer())
        openIntegrations()
        tap("mcp:linear")
        awaitTag("tool:search_issues")
        awaitTag("badge:search_issues:readOnly")
        awaitTag("badge:create_issue:changes")
        awaitTag("badge:delete_issue:changes")
        awaitTag("badge:delete_issue:asksEveryTime")
        awaitTag("badge:export_project:heavy")
        assertFalse(has("badge:search_issues:changes"))
        assertFalse(has("badge:create_issue:asksEveryTime"))
    }

    @Test
    fun theHeavyToggleTellsTheCore() {
        core.mcp = listOf(TestData.mcpServer())
        openIntegrations()
        tap("mcp:linear")
        tap("heavy:create_issue")
        awaitCore { core.mcpHeavy.isNotEmpty() }
        assertEquals(Triple("linear", "create_issue", true), core.mcpHeavy.single())
        awaitTag("badge:create_issue:heavy")
        tap("heavy:export_project")
        awaitCore { core.mcpHeavy.size == 2 }
        assertEquals(Triple("linear", "export_project", false), core.mcpHeavy.last())
    }

    @Test
    fun refreshAsksTheServerAgain() {
        core.mcp = listOf(TestData.mcpServer())
        openIntegrations()
        tap("mcp:linear")
        tap("mcpRefresh")
        awaitCore { core.mcpRefreshes.toList() == listOf("linear") }
    }

    @Test
    fun anErrorOfTheServerIsShownOnItsPage() {
        core.mcp = listOf(TestData.mcpServer(status = "error", error = "The server answered 500", tools = emptyList()))
        openIntegrations()
        tap("mcp:linear")
        awaitText("The server answered 500")
        awaitTag("noTools")
    }

    @Test
    fun removingAsksFirstAndThenGoesBackToTheList() {
        core.mcp = listOf(TestData.mcpServer())
        openIntegrations()
        tap("mcp:linear")
        tap("mcpRemove")
        awaitText("Remove Linear?")
        rule.onNodeWithText("Cancel").performClick()
        settle()
        assertTrue(core.mcpRemoved.isEmpty())
        tap("mcpRemove")
        awaitText("Remove Linear?")
        rule.onNodeWithText("Remove").performClick()
        awaitCore { core.mcpRemoved.toList() == listOf("linear") }
        awaitTag("noMcp")
        assertFalse(has("mcpDetail"))
    }

    // ---- titles -----------------------------------------------------------------------------------------------------

    @Test
    fun aWaitingCallNamesTheToolAndTheServer() {
        core.mcp = listOf(TestData.mcpServer())
        core.pending = listOf(TestData.pending("req30", "write", 1u, service = "mcp:linear", account = null, op = "create_issue"))
        launch()
        awaitTag("pending:req30")
        awaitText("Claude: Create issue")
        awaitText("Linear")
    }
}
