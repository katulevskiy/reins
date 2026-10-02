package dev.rewarden.android.ui

import dev.rewarden.android.TestData
import dev.rewarden.android.design.McpNames
import dev.rewarden.android.design.serviceName
import dev.rewarden.android.push.PushPayload
import dev.rewarden.android.ui.approval.leaseLabel
import dev.rewarden.android.ui.approval.sshTarget
import dev.rewarden.android.ui.common.fileSize
import dev.rewarden.android.ui.common.fullTitle
import dev.rewarden.android.ui.common.isTextFile
import dev.rewarden.android.ui.common.shortSha
import dev.rewarden.android.ui.mcp.ToolBadge
import dev.rewarden.android.ui.mcp.isMcpRedirect
import dev.rewarden.android.ui.mcp.mcpHost
import dev.rewarden.android.ui.mcp.mcpStatusLabel
import dev.rewarden.android.ui.mcp.toolBadges
import dev.rewarden.android.ui.mcp.toolCount
import dev.rewarden.android.ui.mcp.webPage
import dev.rewarden.android.ui.nav.DeepLink
import dev.rewarden.android.ui.nav.SheetTarget
import dev.rewarden.android.ui.services.GitHosts
import dev.rewarden.core.PendingKind
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NewFeaturesLogicTest {
    @After
    fun forget() {
        McpNames.update(emptyList())
    }

    // ---- MCP ------------------------------------------------------------------------------------------------------

    @Test
    fun `a server is shown by its host, never with a query`() {
        assertEquals("mcp.linear.app", mcpHost("https://mcp.linear.app/mcp"))
        assertEquals("mcp.notion.com", mcpHost("https://mcp.notion.com/mcp?key=secret"))
        assertEquals("localhost:8080", mcpHost("http://localhost:8080/mcp"))
        assertEquals("not a url", mcpHost("not a url"))
    }

    @Test
    fun `statuses and tool counts read as words`() {
        assertEquals("Connected", mcpStatusLabel("ok"))
        assertEquals("Needs sign-in", mcpStatusLabel("needs_sign_in"))
        assertEquals("Error", mcpStatusLabel("error"))
        assertEquals("No tools", toolCount(0))
        assertEquals("1 tool", toolCount(1))
        assertEquals("4 tools", toolCount(4))
    }

    @Test
    fun `each tool says whether it changes things, is asked every time, and is heavy`() {
        val (search, create, delete, export) = TestData.mcpTools()
        assertEquals(listOf(ToolBadge.ReadOnly), toolBadges(search))
        assertEquals(listOf(ToolBadge.Changes), toolBadges(create))
        assertEquals(listOf(ToolBadge.Changes, ToolBadge.AsksEveryTime), toolBadges(delete))
        assertEquals(listOf(ToolBadge.ReadOnly, ToolBadge.Heavy), toolBadges(export))
        assertEquals("Read only", ToolBadge.ReadOnly.label)
        assertEquals("Changes things", ToolBadge.Changes.label)
        assertEquals("Asks every time", ToolBadge.AsksEveryTime.label)
    }

    @Test
    fun `only the app's own redirect finishes a sign-in`() {
        assertTrue(isMcpRedirect("com.reins2fa.app://mcp-oauth?code=a&state=b"))
        assertTrue(isMcpRedirect("com.reins2fa.app://mcp-oauth/?error=access_denied&state=b"))
        assertFalse(isMcpRedirect(null))
        assertFalse(isMcpRedirect("https://evil.example.com/mcp-oauth?code=a"))
        assertFalse(isMcpRedirect("com.reins2fa.app://other?code=a"))
        assertFalse(isMcpRedirect("com.reins2fa.app://mcp-oauth.evil.com?code=a"))
        assertFalse(isMcpRedirect("com.reins2fa.app://mcp-oauth?code=" + "a".repeat(9_000)))
    }

    @Test
    fun `only web pages are opened for a sign-in`() {
        assertTrue(webPage("https://linear.app/oauth/authorize?x=1"))
        assertTrue(webPage("http://localhost:3000/authorize"))
        assertFalse(webPage("intent://evil#Intent;end"))
        assertFalse(webPage("javascript:alert(1)"))
        assertFalse(webPage("file:///sdcard/x"))
    }

    @Test
    fun `an mcp service is named after its server`() {
        assertEquals("MCP server", serviceName("mcp:linear"))
        McpNames.update(listOf(TestData.mcpServer()))
        assertEquals("Linear", serviceName("mcp:linear"))
        assertEquals("Claude: Create issue", fullTitle("Claude", "write", 1, "mcp:linear", op = "create_issue"))
        assertEquals("Claude: Search issues", fullTitle("Claude", "read", 1, "mcp:linear", op = "search_issues"))
        assertEquals("Claude: Use unknown_tool", fullTitle("Claude", "write", 1, "mcp:linear", op = "unknown_tool"))
        assertEquals("Claude: Use a tool of Linear", fullTitle("Claude", "write", 1, "mcp:linear", op = ""))
    }

    // ---- the new services and uploads ------------------------------------------------------------------------------

    @Test
    fun `new services have their names`() {
        assertEquals("GitLab", serviceName("gitlab"))
        assertEquals("Codeberg", serviceName("codeberg"))
        assertEquals("Bitbucket", serviceName("bitbucket"))
        assertEquals("Desktop app", serviceName("desktop"))
        assertEquals("Files", serviceName("files"))
    }

    @Test
    fun `an upload reads as sharing a file`() {
        assertEquals("Claude: Share a file", fullTitle("Claude", "upload", 1, "files"))
    }

    @Test
    fun `files are described by size, short hash and kind`() {
        assertEquals("812 bytes", fileSize(812u))
        assertEquals("18.0 KB", fileSize(18_432u))
        assertEquals("1.4 MB", fileSize(1_468_006u))
        assertEquals("2.0 GB", fileSize(2_147_483_648u))
        assertEquals("9f86d081884c7d65…0a08", shortSha("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"))
        assertEquals("abc", shortSha("abc"))
        assertTrue(isTextFile("text/plain"))
        assertTrue(isTextFile("text/csv; charset=utf-8"))
        assertTrue(isTextFile("application/json"))
        assertFalse(isTextFile("application/pdf"))
        assertFalse(isTextFile("image/png"))
    }

    @Test
    fun `leases and ssh targets read well`() {
        assertEquals("for 1 minute", leaseLabel(60u))
        assertEquals("for 30 minutes", leaseLabel(1_800u))
        assertEquals("for 1 hour", leaseLabel(3_600u))
        assertEquals("for 1 h 30 min", leaseLabel(5_400u))
        assertEquals("for 24 hours", leaseLabel(86_400u))
        assertEquals("build.example.com", sshTarget("build.example.com", "SHA256:x"))
        assertEquals("SHA256:x", sshTarget(null, "SHA256:x"))
        assertEquals("SHA256:x", sshTarget("  ", "SHA256:x"))
        assertEquals("an unknown server", sshTarget(null, null))
    }

    // ---- token pages ---------------------------------------------------------------------------------------------

    @Test
    fun `gitlab's page is filled in with a fresh name and the scopes git needs`() {
        assertEquals(
            "https://gitlab.com/-/user_settings/personal_access_tokens?name=Rewarden-123456&scopes=read_api,read_repository,write_repository",
            GitHosts.of("gitlab")!!.tokenUrl(123_456),
        )
        val names = (1..20).map { GitHosts.of("gitlab")!!.tokenUrl().substringAfter("name=").substringBefore("&") }.toSet()
        assertTrue(names.size > 1)
    }

    @Test
    fun `codeberg and bitbucket open their token pages`() {
        assertEquals("https://codeberg.org/user/settings/applications", GitHosts.of("codeberg")!!.tokenUrl())
        assertEquals("https://id.atlassian.com/manage-profile/security/api-tokens", GitHosts.of("bitbucket")!!.tokenUrl())
        assertNull(GitHosts.of("github"))
    }

    @Test
    fun `only real looking tokens are picked up from the clipboard`() {
        val gitlab = GitHosts.of("gitlab")!!
        assertTrue(gitlab.looksLikeToken("glpat-abcdefghijklmnopqrst"))
        assertFalse(gitlab.looksLikeToken("hello world"))
        assertFalse(gitlab.looksLikeToken(null))
        val codeberg = GitHosts.of("codeberg")!!
        assertTrue(codeberg.looksLikeToken("0123456789abcdef0123456789abcdef01234567"))
        assertFalse(codeberg.looksLikeToken("0123456789abcdef"))
        // Bitbucket needs the email too, so nothing is ever taken from the clipboard by itself.
        assertFalse(GitHosts.of("bitbucket")!!.looksLikeToken("ATATT3xFfGF0abcdefghijklmnop"))
    }

    // ---- push and links -------------------------------------------------------------------------------------------

    @Test
    fun `an upload push is accepted`() {
        assertEquals(PushPayload("blob", "blob_0123456789abcdef"), PushPayload.parse(mapOf("t" to "blob", "id" to "blob_0123456789abcdef")))
    }

    @Test
    fun `a notification link opens a waiting upload`() {
        val link = DeepLink.parse("dev.rewarden.android.OPEN_ITEM", "blob", "blob_0123456789abcdef", "dev.rewarden.android.OPEN_ITEM")
        assertEquals(DeepLink(PendingKind.BLOB, "blob_0123456789abcdef"), link)
        assertEquals(SheetTarget.Upload("blob_0123456789abcdef"), link!!.resolve(listOf(TestData.blobItem())))
        assertNull(link.resolve(emptyList()))
    }
}
