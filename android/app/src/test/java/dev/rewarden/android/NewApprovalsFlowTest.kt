package dev.rewarden.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.platform.AuthResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class NewApprovalsFlowTest : FlowHarness() {
    // ---- MCP calls --------------------------------------------------------------------------------------------------

    @Test
    fun anMcpCallShowsTheServerTheToolAndItsArguments() {
        core.mcp = listOf(TestData.mcpServer())
        openRequest(TestData.mcpCallView())
        awaitTag("mcpCall")
        rule.onNodeWithTag("mcpServer").assertTextContains("Linear")
        rule.onNodeWithTag("mcpHost").assertTextEquals("mcp.linear.app")
        rule.onNodeWithTag("mcpTool").assertTextContains("Create issue")
        awaitText("create_issue")
        awaitText("Creates an issue in a team.")
        rule.onNodeWithTag("mcpArguments").assertTextContains("\"title\": \"Login fails on Safari\"", substring = true)
        rule.onNodeWithTag("mcpEffect").assertTextEquals("Changes things")
        assertFalse(has("mcpDestructive"))
        // The tool's title is the headline.
        rule.onNodeWithTag("what").assertTextEquals("Create issue")
        assertFalse(has("writePreview"))
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("req30", core.approvals.single().first)
    }

    @Test
    fun aReadOnlyCallSaysSo() {
        openRequest(TestData.mcpCallView(readOnly = true))
        awaitTag("mcpEffect")
        rule.onNodeWithTag("mcpEffect").assertTextEquals("Read only")
    }

    @Test
    fun aDestructiveCallIsAskedEveryTime() {
        openRequest(TestData.mcpCallView(destructive = true))
        awaitTag("mcpDestructive")
        awaitText("asked for every time", substring = true)
    }

    // ---- a file a write uses -------------------------------------------------------------------------------------------

    @Test
    fun aWriteWithAnUploadedFileShowsTheFile() {
        openRequest(TestData.fileWriteView())
        awaitTag("writePreview")
        awaitTag("attachedFile")
        rule.onNodeWithTag("fileName").assertTextEquals("report.csv")
        rule.onNodeWithTag("fileSize").assertTextContains("18.0 KB", substring = true)
        rule.onNodeWithTag("fileSize").assertTextContains("text/csv", substring = true)
        rule.onNodeWithTag("fileSha").assertTextContains("9f86d081884c7d65", substring = true)
        rule.onNodeWithTag("fileText").assertTextContains("quarter,revenue,costs", substring = true)
        assertFalse(has("fileImage"))
    }

    @Test
    fun anImageIsPreviewedAsAnImage() {
        openRequest(TestData.fileWriteView(TestData.blob(name = "chart.png", contentType = "image/png", previewText = null, previewImage = TestData.tinyPng)))
        awaitTag("fileImage")
        assertFalse(has("fileText"))
    }

    @Test
    fun aBinaryFileIsDescribedNotShownAsText() {
        openRequest(TestData.fileWriteView(TestData.blob(name = "manual.pdf", contentType = "application/pdf", previewText = "PDF document")))
        awaitTag("fileKind")
        rule.onNodeWithTag("fileKind").assertTextEquals("PDF document")
        assertFalse(has("fileText"))
    }

    // ---- the desktop app -------------------------------------------------------------------------------------------

    @Test
    fun aQuestionIsAnsweredYesOrNo() {
        openRequest(TestData.askView())
        awaitTag("askQuestion")
        rule.onNodeWithTag("askQuestion").assertTextEquals("Force-push main?")
        rule.onNodeWithTag("askDetail").assertTextEquals("git push --force origin main")
        rule.onNodeWithTag("askTopic").assertTextContains("git push --force", substring = true)
        rule.onNodeWithTag("approve").assertTextEquals("Yes")
        rule.onNodeWithTag("deny").assertTextEquals("No")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("req40", core.approvals.single().first)
    }

    @Test
    fun noToAQuestionDenies() {
        openRequest(TestData.askView(detail = null, topic = null))
        awaitTag("askQuestion")
        assertFalse(has("askDetail"))
        assertFalse(has("askTopic"))
        tap("deny")
        awaitCore { core.denials.toList() == listOf("req40") }
    }

    @Test
    fun secretsListNamesNeverValues() {
        openRequest(TestData.secretsView())
        awaitTag("secrets")
        rule.onNodeWithTag("secretsCommand").assertTextEquals("npm run deploy")
        rule.onNodeWithTag("secretsPurpose").assertTextContains("Deploy the site", substring = true)
        rule.onNodeWithTag("secret:0").assertTextEquals("Netlify · password")
        rule.onNodeWithTag("secret:1").assertTextEquals("GitHub · totp")
        rule.onNodeWithTag("secretsLease").assertTextContains("for 30 minutes", substring = true)
    }

    @Test
    fun anSshSignInNamesTheServerAndTheKey() {
        openRequest(TestData.sshView())
        awaitTag("ssh")
        rule.onNodeWithTag("sshWhat").assertTextEquals("Sign in to build.example.com with Deploy key")
        rule.onNodeWithTag("sshKey").assertTextContains("SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8", substring = true)
        rule.onNodeWithTag("sshHostKey").assertTextContains("SHA256:Ql3mV1hd2bS9WgS0aVq2Jv0Rr2q8Zb0yRrj2uMm3n0E", substring = true)
    }

    @Test
    fun anSshSignInToAnUnnamedServerNamesItsHostKey() {
        openRequest(TestData.sshView(host = null))
        awaitTag("sshWhat")
        rule.onNodeWithTag("sshWhat").assertTextEquals("Sign in to SHA256:Ql3mV1hd2bS9WgS0aVq2Jv0Rr2q8Zb0yRrj2uMm3n0E with Deploy key")
    }

    @Test
    fun aPushToGitlabNamesGitlab() {
        val ref = TestData.gitRef("old", change = "delete", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        openRequest(TestData.gitPushView(TestData.gitPush(ref)).copy(service = "gitlab"))
        awaitText("The branch is removed from GitLab.")
    }

    // ---- uploads ------------------------------------------------------------------------------------------------------

    private fun upload(view: dev.rewarden.core.BlobView = TestData.blob()) {
        core.blobs[view.id] = view
        core.pending = listOf(TestData.blobItem(view))
    }

    @Test
    fun anUploadWaitsInTheListAndItsSheetShowsTheFile() {
        upload()
        launch()
        awaitTag("pending:blob_0123456789abcdef")
        awaitText("Claude: Share a file")
        awaitText("18.0 KB · The quarterly numbers for the summary")
        if (!has("uploadSheet")) tap("pending:blob_0123456789abcdef")
        awaitTag("uploadSheet")
        rule.onNodeWithTag("uploadReason").assertTextContains("The quarterly numbers for the summary", substring = true)
        rule.onNodeWithTag("fileName").assertTextEquals("report.csv")
        rule.onNodeWithTag("fileText").assertTextContains("Q1,120,80", substring = true)
        rule.onNodeWithTag("fileSha").assertTextContains("9f86d081884c7d65", substring = true)
    }

    @Test
    fun approvingAnUploadNeedsTheScreenLockAndTellsTheCore() {
        upload()
        launch(link("blob", "blob_0123456789abcdef"))
        awaitTag("uploadSheet")
        tap("approve")
        awaitCore { core.blobAnswers.isNotEmpty() }
        assertEquals("blob_0123456789abcdef" to true, core.blobAnswers.single())
        assertEquals(1, prompts.get())
        awaitGone("uploadSheet")
    }

    @Test
    fun aCancelledScreenLockApprovesNothing() {
        authResult = AuthResult.Cancelled
        upload()
        launch(link("blob", "blob_0123456789abcdef"))
        tap("approve")
        settle()
        assertTrue(core.blobAnswers.isEmpty())
        assertTrue(has("uploadSheet"))
    }

    @Test
    fun denyingAnUploadTellsTheCore() {
        upload(TestData.blob(name = "photo.png", contentType = "image/png", previewText = null, previewImage = TestData.tinyPng))
        launch(link("blob", "blob_0123456789abcdef"))
        awaitTag("fileImage")
        tap("deny")
        awaitCore { core.blobAnswers.isNotEmpty() }
        assertEquals("blob_0123456789abcdef" to false, core.blobAnswers.single())
        assertEquals(0, prompts.get())
        awaitGone("uploadSheet")
    }

    @Test
    fun anUploadThatIsGoneSaysSo() {
        core.pending = listOf(TestData.blobItem())
        launch(link("blob", "blob_0123456789abcdef"))
        awaitTag("uploadError")
    }
}
