package dev.reins.android

import android.content.Context
import android.content.Intent
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performTextReplacement
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.core.CoreFactory
import dev.reins.android.core.CoreProvider
import dev.reins.android.design.Timers
import dev.reins.android.platform.AppNotifier
import dev.reins.android.platform.Foreground
import dev.reins.android.platform.update.FakeInstaller
import dev.reins.android.platform.update.FakeUpdateServer
import dev.reins.android.platform.update.UpdateProvider
import dev.reins.core.ActivityInfo
import dev.reins.core.EmailView
import dev.reins.core.GmailStatus
import dev.reins.core.PairingView
import dev.reins.core.SessionInfo
import java.io.File
import org.junit.After
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.BeforeClass
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Renders the main screens to PNG files for design review (`-Dreins.screenshots=/some/dir`). Skipped otherwise, so
 * it costs nothing in a normal test run. Clocks are frozen so every frame is reproducible.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
abstract class ScreenshotsBase(private val suffix: String) {
    @get:Rule val rule = createEmptyComposeRule()
    private val core = FakeCore.shared
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private var scenario: ActivityScenario<MainActivity>? = null
    private val now = 1_700_000_100L
    private val updates = FakeUpdateServer()

    @Before
    fun setUp() {
        dev.reins.android.TestNativeKeys.install()
        assumeTrue(System.getProperty("reins.screenshots") != null)
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        // Robolectric has no screen lock; the tests that need none set it.
        dev.reins.android.platform.ScreenLock.check = { true }
        Timers.live = false
        Timers.frozenNowMillis = now * 1000
        Foreground.focused = false
        Foreground.autoPopup = false
        core.session = SessionInfo("https://app.reins2fa.com", "me@example.com")
        core.pending = emptyList()
        core.startingPolicy = null
        core.approval = null
        core.pairing = null
        core.grants = emptyList()
        core.connections = listOf(TestData.connection("c1", "Claude"), TestData.connection("c2", "My ChatGPT"), TestData.connection("c3", "Hermes agent"), TestData.connection("c4", "notes-bot"))
        core.gmail = GmailStatus.Ready
        core.activity = emptyList()
        core.catalogue = FakeCore.defaultServices()
        core.mcp = emptyList()
        core.blobs.clear()
        core.resetAutopilot()
        core.resetSso()
        core.resetOnboarding()
        UpdateProvider.fetcher = updates
        UpdateProvider.installer = FakeInstaller()
        androidx.work.testing.WorkManagerTestInitHelper.initializeTestWorkManager(context)
        (context.applicationContext as ReinsApp).container.state.setSession(dev.reins.android.state.SessionState.Loading)
    }

    @After
    fun tearDown() {
        scenario?.close()
        Timers.live = true
        Timers.frozenNowMillis = null
        Foreground.autoPopup = true
    }

    private fun launch(intent: Intent = Intent(context, MainActivity::class.java)) {
        scenario = ActivityScenario.launch(intent)
    }

    private fun link(kind: String, id: String) = Intent(context, MainActivity::class.java)
        .setAction(AppNotifier.ACTION_OPEN_ITEM)
        .putExtra(AppNotifier.EXTRA_KIND, kind)
        .putExtra(AppNotifier.EXTRA_ID, id)

    private fun settle() {
        androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications()
        shadowOf(android.os.Looper.getMainLooper()).idle()
    }

    private fun await(tag: String) {
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun tap(tag: String) {
        await(tag)
        val node = rule.onNodeWithTag(tag)
        try {
            node.performScrollTo()
        } catch (_: AssertionError) {
        }
        node.performClick()
    }

    /** [dialogs]: also draws dialog windows (each over its dim), which live outside the activity's window. */
    private fun shoot(name: String, dialogs: Boolean = false) {
        val dir = System.getProperty("reins.screenshots")
        rule.waitForIdle()
        repeat(3) { settle() }
        var bitmap: android.graphics.Bitmap? = null
        scenario!!.onActivity { activity ->
            val view = activity.window.decorView
            val b = android.graphics.Bitmap.createBitmap(view.width, view.height, android.graphics.Bitmap.Config.ARGB_8888)
            val canvas = android.graphics.Canvas(b)
            view.draw(canvas)
            if (dialogs) {
                val global = Class.forName("android.view.WindowManagerGlobal").getMethod("getInstance").invoke(null)!!
                @Suppress("UNCHECKED_CAST")
                val roots = global.javaClass.getDeclaredField("mViews").apply { isAccessible = true }.get(global) as List<android.view.View>
                roots.filter { it !== view && it.isShown && it.width > 0 }.forEach { root ->
                    val dim = (root.layoutParams as? android.view.WindowManager.LayoutParams)?.dimAmount ?: 0f
                    canvas.drawColor(android.graphics.Color.argb((dim * 255).toInt(), 0, 0, 0))
                    // Dialogs are centred on screen (Robolectric does not lay their windows out).
                    canvas.save()
                    canvas.translate((view.width - root.width) / 2f, (view.height - root.height) / 2f)
                    root.draw(canvas)
                    canvas.restore()
                }
            }
            bitmap = b
        }
        File(dir!!).mkdirs()
        File(dir, "$name-$suffix.png").outputStream().use { bitmap!!.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun history() {
        val messages = TestData.messages("Your March statement is ready", "Wire transfer receipt", "Security alert")
        core.activity = listOf(
            TestData.entry(6, "read", "released", 3u, at = now - 120, info = TestData.info(null, messages, null, null, null)).copy(connectionLabel = "My ChatGPT", connectionId = "c2"),
            TestData.entry(5, "send", "sent", 2u, at = now - 900, info = TestData.info(null, emptyList(), EmailView(listOf("ann@corp.example"), listOf("bob@corp.example"), "Q3 report", "Hi Ann,\n\nAttached is the Q3 report as discussed.\n\nBest"), null, null)),
            TestData.entry(4, "search", "denied", 5u, at = now - 3600).copy(connectionLabel = "notes-bot", connectionId = "c4"),
            TestData.entry(3, "grant", "granted", 1u, at = now - 7200).copy(connectionLabel = "Hermes agent", connectionId = "c3"),
            TestData.entry(2, "read", "released", 1u, at = now - 86_400 * 2, grantId = "g1"),
            TestData.entry(1, "search", "error", 0u, at = now - 86_400 * 3),
        )
        core.accounts = listOf(
            dev.reins.core.AccountView("gmail", "me@gmail.com", now - 86_400 * 30),
            dev.reins.core.AccountView("gmail", "work@corp.example", now - 86_400 * 4),
        )
        core.accountStatuses = mapOf("work@corp.example" to GmailStatus.NeedsConsent)
        core.grants = listOf(
            TestData.grant("g1", uses = 12u, leftSeconds = 3_000, ageSeconds = 600),
            TestData.grant("g2", maxUses = 1u, uses = 0u, action = "send", leftSeconds = 5 * 86_400 + 600, ageSeconds = 86_400),
            TestData.grant("g4", maxUses = 5u, uses = 3u, leftSeconds = 240, ageSeconds = 3_360),
            TestData.grant("g5", maxUses = 40u, uses = 9u, leftSeconds = 10 * 7 * 86_400, ageSeconds = 3_600),
            TestData.grant("g3", active = false),
            TestData.grant("g6", active = false, state = "used_up", maxUses = 3u, uses = 3u),
            TestData.grant("g7", active = false, state = "revoked"),
        )
    }

    @Test
    fun activity() {
        history()
        core.pending = listOf(
            TestData.pending("req1", "search", 4u, waitUntil = now + 34, label = "Claude", conn = "c1"),
            TestData.pending("req9", "send", 2u, waitUntil = now + 9, label = "My ChatGPT", conn = "c2"),
        )
        launch()
        await("entry:6")
        shoot("1-activity")
    }

    @Test
    fun approvalSheet() {
        history()
        core.pending = listOf(TestData.pending("req1", "search", 3u, waitUntil = now + 34))
        core.approval = TestData.searchView(waitUntil = now + 34)
        launch(link("request", "req1"))
        await("approve")
        shoot("2-approval")
        tap("moreToggle")
        await("allMail:HOUR")
        shoot("3-approval-more")
    }

    @Test
    fun lateApproval() {
        history()
        core.pending = listOf(TestData.pending("req1", "search", 3u, waitUntil = now - 20))
        core.approval = TestData.searchView(waitUntil = now - 20)
        launch(link("request", "req1"))
        await("lateBanner")
        shoot("4-approval-late")
    }

    @Test
    fun sendSheet() {
        history()
        core.pending = listOf(TestData.pending("req2", "send", 2u, waitUntil = now + 12, label = "My ChatGPT", conn = "c2"))
        core.approval = TestData.sendView().copy(connectionId = "c2", connectionLabel = "My ChatGPT", waitUntil = now + 12)
        launch(link("request", "req2"))
        await("emailPreview")
        shoot("5-send")
    }

    @Test
    fun permissionRequest() {
        history()
        core.pending = listOf(TestData.pending("req3", "grant", 1u, waitUntil = now + 30, label = "Hermes agent", conn = "c3"))
        core.approval = TestData.grantView().copy(connectionId = "c3", connectionLabel = "Hermes agent", waitUntil = now + 30)
        launch(link("request", "req3"))
        await("grantCard")
        shoot("6-permission")
    }

    @Test
    fun pairing() {
        history()
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", "Claude", "claude.ai", byteArrayOf(7, 42, 88), 1_700_000_200, null)
        launch(link("pairing", "p1"))
        await("code:42")
        shoot("7-pairing")
    }

    @Test
    fun activityDetails() {
        history()
        launch()
        tap("entry:5")
        await("detailEmail")
        shoot("8-detail-send")
    }

    @Test
    fun grants() {
        history()
        launch()
        tap("tabGrants")
        await("grant:g1")
        shoot("9-grants")
        tap("expiredHeader")
        await("ended:g3")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("9b-grants-expired")
    }

    @Test
    fun grantDetail() {
        history()
        launch()
        tap("tabGrants")
        tap("grant:g1")
        await("revoke")
        shoot("10-grant-detail")
    }

    @Test
    fun accountsRequest() {
        history()
        core.pending = listOf(TestData.pending("req5", "accounts", 3u, waitUntil = now + 30))
        core.approval = TestData.accountsView(listOf("alex@example.com", "alex.work@example.com", "alex.shop@example.com"))
        launch(link("request", "req5"))
        await("accountsCard")
        tap("acct:alex.work@example.com")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("17-accounts-request")
    }

    @Test
    fun integrations() {
        history()
        launch()
        tap("integrations")
        await("service:gmail")
        shoot("15-integrations")
        tap("service:gmail")
        await("account:me@gmail.com")
        shoot("16-gmail-accounts")
    }

    @Test
    fun newGrant() {
        history()
        launch()
        tap("tabGrants")
        tap("newGrant")
        await("createGrant")
        shoot("11-new-grant")
    }

    @Test
    fun settings() {
        history()
        launch()
        tap("openSettings")
        await("registerPhone")
        shoot("12-settings")
        tap("connection:c1")
        await("disconnect")
        shoot("13-connection")
    }

    @Test
    fun serviceList() {
        history()
        core.accounts = core.accounts + listOf(
            dev.reins.core.AccountView("telegram", "+15550100", now - 86_400),
            dev.reins.core.AccountView("github", "octo-cat", now - 86_400),
            dev.reins.core.AccountView("sms", "this phone", now - 3_600),
        )
        launch()
        await("entry:6")
        tap("integrations")
        await("service:vault")
        shoot("15-integrations")
        tap("service:telegram")
        await("account:+15550100")
        shoot("16-telegram")
    }

    @Test
    fun telegramSignIn() {
        launch()
        await("integrations")
        tap("integrations")
        tap("service:telegram")
        await("phone")
        shoot("17-telegram-signin")
    }

    @Test
    fun chatRead() {
        history()
        core.pending = listOf(TestData.pending("req7", "read", 3u, waitUntil = now + 40, service = "telegram", account = "+15550100", op = "read", opTitle = "Read Telegram messages"))
        core.approval = TestData.fetchView().copy(waitUntil = now + 40)
        launch(link("request", "req7"))
        await("approve")
        shoot("18-chat-read")
        tap("moreToggle")
        await("lifetime:HOUR")
        tap("lifetime:HOUR")
        await("resource:100")
        shoot("19-chat-read-more")
    }

    @Test
    fun passwordRequest() {
        history()
        core.pending = listOf(TestData.pending("req8", "read", 1u, waitUntil = now + 40, service = "vault", account = "me@example.com", op = "get", opTitle = "Get a login from the vault"))
        core.approval = TestData.vaultView().copy(waitUntil = now + 40)
        launch(link("request", "req8"))
        await("approve")
        shoot("20-password")
    }

    @Test
    fun messageToSend() {
        history()
        core.pending = listOf(TestData.pending("req9", "send", 1u, waitUntil = now + 40, service = "telegram", account = "+15550100", op = "send", opTitle = "Send a Telegram message"))
        core.approval = TestData.writeView().copy(waitUntil = now + 40)
        launch(link("request", "req9"))
        await("writePreview")
        shoot("21-telegram-send")
    }

    @Test
    fun repositoryChange() {
        history()
        val view = TestData.repoWriteView().copy(waitUntil = now + 40)
        core.pending = listOf(TestData.pending("req10", "write", 1u, waitUntil = now + 40, service = "github", account = "octo-cat", op = "file_put", opTitle = view.opTitle))
        core.approval = view
        launch(link("request", "req10"))
        await("writePreview")
        shoot("22-repo-change")
        tap("moreToggle")
        tap("lifetime:DAY")
        tap("class:issues")
        await("widerCaption")
        rule.onNodeWithTag("resource:octo").performScrollTo()
        shoot("23-repo-change-more")
    }

    @Test
    fun repositoryDeleteAskedEveryTime() {
        history()
        val view = TestData.onceOnlyWriteView().copy(waitUntil = now + 40)
        core.pending = listOf(TestData.pending("req11", "write", 1u, waitUntil = now + 40, service = "github", account = "octo-cat", op = "repo_delete", opTitle = view.opTitle))
        core.approval = view
        launch(link("request", "req11"))
        await("onceWarning")
        shoot("24-repo-delete")
        tap("moreToggle")
        await("noStanding")
        shoot("25-repo-delete-more")
    }

    @Test
    fun desktopPairing() {
        history()
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = TestData.pairingView("p1", "Reins desktop app on laptop", "laptop", keyFingerprint = "4821 9930")
        launch(link("pairing", "p1"))
        await("keyFingerprint")
        shoot("27-desktop-pairing")
    }

    private fun openGit(view: dev.reins.core.ApprovalView) {
        history()
        core.pending = listOf(
            TestData.pending(view.requestId, "write", 1u, waitUntil = now + 40, label = view.connectionLabel, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle),
        )
        core.approval = view.copy(waitUntil = now + 40)
        launch(link("request", view.requestId))
        await("gitPush")
    }

    private fun realisticFiles() = listOf(
        TestData.gitFile("src/auth/session.rs", "modified", 42u, 7u),
        TestData.gitFile("src/auth/login.rs", "added", 88u, 0u),
        TestData.gitFile("crates/reins-core/src/connector/github/very/deeply/nested/module/path/git.rs", "modified", 12u, 3u),
        TestData.gitFile("assets/logo.png", "added", null, null, binary = true),
        TestData.gitFile("src/old_login.rs", "deleted", 0u, 64u),
        TestData.gitFile("scripts/run.sh", "type_changed", 0u, 0u),
        TestData.gitFile("README.md", "modified", 6u, 2u),
        TestData.gitFile("Cargo.toml", "modified", 1u, 0u),
        TestData.gitFile("Cargo.lock", "modified", 30u, 12u),
        TestData.gitFile("docs/login.md", "added", 40u, 0u),
    )

    private fun realisticCommits() = listOf(
        dev.reins.core.GitCommitView("9f3c2a1", "Log in with a passkey when the browser offers one", "Ada Lovelace <ada@example.com>"),
        dev.reins.core.GitCommitView("4be81d0", "Remove the old login form", "Ada Lovelace <ada@example.com>"),
        dev.reins.core.GitCommitView("c07a9e3", "Keep the session for 30 days", "Grace Hopper <grace@example.com>"),
        dev.reins.core.GitCommitView("17d0f5b", "Document the login flow", "Ada Lovelace <ada@example.com>"),
        dev.reins.core.GitCommitView("e2a4c66", "Bump dependencies", "dependabot[bot] <support@github.com>"),
        dev.reins.core.GitCommitView("88b13f2", "Fix a typo", "Grace Hopper <grace@example.com>"),
    )

    @Test
    fun gitPush() {
        val ref = TestData.gitRef("feature/passkeys", commitCount = 8u, commits = realisticCommits(), filesChanged = 11u, files = realisticFiles(), additions = 219u, deletions = 88u)
        openGit(TestData.gitPushView(TestData.gitPush(ref, notes = listOf("1 file was too large to count lines."), packBytes = 48_213u)))
        shoot("28-git-push")
        tap("gitMoreCommits:0")
        tap("gitMoreFiles:0")
        await("gitFile:0:9")
        rule.onNodeWithTag("gitNote:0").performScrollTo()
        shoot("29-git-push-expanded")
    }

    @Test
    fun gitForcePush() {
        val ref = TestData.gitRef("main", force = true, commitCount = 2u, commits = realisticCommits().take(2), filesChanged = 3u, files = realisticFiles().take(3), additions = 142u, deletions = 10u)
        openGit(TestData.gitPushView(TestData.gitPush(ref, packBytes = 3_145_728u), noStanding = true))
        shoot("30-git-force-push")
        tap("moreToggle")
        await("noStanding")
        shoot("31-git-force-push-more")
    }

    @Test
    fun gitTagsAndUnknownHistory() {
        val tag = TestData.gitRef("v1.2.0", kind = "tag", change = "create", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        val branch = TestData.gitRef("release", forceUnknown = true, force = true, commitCount = 1u, commits = realisticCommits().take(1), filesChanged = 2u, files = realisticFiles().subList(3, 5), additions = null, deletions = null)
        val gone = TestData.gitRef("old-experiment", change = "delete", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        openGit(TestData.gitPushView(TestData.gitPush(tag, branch, gone), noStanding = true, tags = true))
        shoot("32-git-tags-unknown")
    }

    @Test
    fun gitFetch() {
        history()
        val view = TestData.gitFetchView().copy(waitUntil = now + 40)
        core.pending = listOf(TestData.pending(view.requestId, "read", 1u, waitUntil = now + 40, label = view.connectionLabel, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle))
        core.approval = view
        launch(link("request", view.requestId))
        await("message:octo/app")
        shoot("33-git-fetch")
    }

    @Test
    fun githubTokenPage() {
        launch()
        await("integrations")
        tap("integrations")
        tap("service:github")
        await("openGithubClassic")
        shoot("26-github-token")
    }

    // ---- MCP servers, uploads, the desktop app, more git hosts -----------------------------------------------------

    private fun mcpServers() {
        core.mcp = listOf(
            TestData.mcpServer(),
            TestData.mcpServer("notion", "Notion", "https://mcp.notion.com/mcp", status = "needs_sign_in", tools = emptyList()),
            TestData.mcpServer(
                "sentry", "Sentry", "https://mcp.sentry.dev/mcp", status = "error",
                error = "The server did not answer in time. Try again later.", tools = TestData.mcpTools().take(2),
            ),
        )
    }

    @Test
    fun mcpList() {
        history()
        mcpServers()
        launch()
        tap("integrations")
        await("mcp:linear")
        rule.onNodeWithTag("addMcp").performScrollTo()
        shoot("40-mcp-list")
        tap("mcp:linear")
        await("tool:export_project")
        shoot("41-mcp-server")
        tap("mcpRemove")
        rule.waitForIdle()
        shoot("41b-mcp-remove", dialogs = true)
    }

    @Test
    fun mcpAdd() {
        launch()
        tap("integrations")
        tap("addMcp")
        await("mcpUrl")
        shoot("42-mcp-add")
    }

    private fun openView(view: dev.reins.core.ApprovalView) {
        history()
        core.pending = listOf(
            TestData.pending(
                view.requestId, view.action, 1u, waitUntil = now + 40, label = view.connectionLabel, service = view.service,
                account = view.account, op = view.op, opTitle = view.opTitle,
            ),
        )
        core.approval = view.copy(waitUntil = now + 40)
        launch(link("request", view.requestId))
        await("approve")
    }

    @Test
    fun mcpCall() {
        mcpServers()
        openView(TestData.mcpCallView())
        await("mcpCall")
        shoot("43-mcp-call")
    }

    @Test
    fun mcpDestructiveCall() {
        mcpServers()
        openView(TestData.mcpCallView(destructive = true))
        await("mcpDestructive")
        rule.onNodeWithTag("mcpDestructive").performScrollTo()
        shoot("44-mcp-call-destructive")
    }

    @Test
    fun uploadSheet() {
        history()
        val blob = TestData.blob(
            name = "q3-numbers.csv",
            previewText = "quarter,region,revenue,costs\nQ3,EMEA,1204000,880000\nQ3,Americas,2310500,1502000\nQ3,APAC,980250,701000",
        )
        core.blobs[blob.id] = blob
        core.pending = listOf(TestData.blobItem(blob))
        launch(link("blob", blob.id))
        await("uploadSheet")
        shoot("45-upload")
    }

    @Test
    fun uploadPending() {
        history()
        val blob = TestData.blob()
        core.blobs[blob.id] = blob
        core.pending = listOf(TestData.blobItem(blob))
        launch()
        await("pending:${blob.id}")
        shoot("46-upload-pending")
    }

    @Test
    fun attachedFile() {
        openView(TestData.fileWriteView())
        await("attachedFile")
        rule.onNodeWithTag("fileText").performScrollTo()
        shoot("47-attached-file")
    }

    @Test
    fun ask() {
        openView(TestData.askView())
        await("askQuestion")
        shoot("48-ask")
    }

    @Test
    fun secrets() {
        openView(TestData.secretsView())
        await("secrets")
        shoot("49-secrets")
    }

    @Test
    fun ssh() {
        openView(TestData.sshView())
        await("ssh")
        shoot("50-ssh")
    }

    @Test
    fun gitlabSignIn() {
        launch()
        await("integrations")
        tap("integrations")
        tap("service:gitlab")
        await("openTokenPage")
        shoot("51-gitlab-token")
    }

    @Test
    fun bitbucketSignIn() {
        launch()
        await("integrations")
        tap("integrations")
        tap("service:bitbucket")
        await("secret")
        shoot("52-bitbucket-token")
    }

    @Test
    fun soundsAndHaptics() {
        launch()
        tap("openSettings")
        tap("openSounds")
        await("soundsMaster")
        shoot("53-sounds")
        rule.onNodeWithTag("try:Alert").performScrollTo()
        shoot("54-sounds-try")
    }

    @Test
    fun updates() {
        assumeTrue(BuildConfig.SELF_UPDATE)
        history()
        updates.publish(29_834_567, versionName = "0.2.0")
        launch()
        await("updatePrompt")
        shoot("35-update-prompt")
        tap("openSettings")
        await("installUpdate")
        rule.onNodeWithTag("signOut").performScrollTo()
        shoot("36-settings-updates")
    }

    @Test
    fun updatesUpToDate() {
        assumeTrue(BuildConfig.SELF_UPDATE)
        launch()
        tap("openSettings")
        tap("checkUpdates")
        await("updateStatus")
        rule.onNodeWithTag("signOut").performScrollTo()
        shoot("37-settings-up-to-date")
    }

    @Test
    fun installPermission() {
        assumeTrue(BuildConfig.SELF_UPDATE)
        val installer = FakeInstaller().apply { allowed = false }
        UpdateProvider.installer = installer
        updates.publish(29_834_567, versionName = "0.2.0")
        launch()
        tap("updatePromptInstall")
        await("allowInstalls")
        shoot("38-install-permission", dialogs = true)
    }

    // ---- Autopilot ------------------------------------------------------------------------------------------------

    private val installed = TestData.modelStatus(dev.reins.core.ModelState.INSTALLED, size = 412_000_000u)

    /** History with what Autopilot, a bypass and Lockdown decided. */
    private fun automaticHistory() {
        history()
        val auto = listOf(
            TestData.entry(9, "write", "released", 1u, at = now - 60, opTitle = "Push to a branch", decidedBy = "autopilot", autopilot = TestData.note()).copy(service = "github", account = "dkat", detail = "feature/laya → dkat/reins · 3 commits"),
            TestData.entry(8, "send", "denied", 1u, at = now - 300, decidedBy = "autopilot", autopilot = TestData.note(suggested = dev.reins.core.Verdict.DENY, pApprove = 0.04f)).copy(connectionLabel = "notes-bot", connectionId = "c4", detail = "To backup-svc@protonmail.example"),
            TestData.entry(7, "read", "released", 2u, at = now - 600, decidedBy = "bypass").copy(connectionLabel = "My ChatGPT", connectionId = "c2"),
        )
        core.activity = auto + core.activity
    }

    private fun openAutopilot() {
        launch()
        tap("openSettings")
        tap("openAutopilot")
        await("mode:MANUAL")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
    }

    @Test
    fun autopilotModes() {
        core.model = installed
        openAutopilot()
        shoot("60-autopilot-assisted")
        rule.onNodeWithTag("newProfile").performScrollTo()
        shoot("60b-autopilot-model-profiles")
        tap("mode:AUTO")
        await("heroMode")
        rule.onNodeWithTag("modeHero").performScrollTo()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("61-autopilot-auto")
        tap("mode:MANUAL")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("62-autopilot-manual")
        tap("mode:BYPASS")
        await("bypassDialog")
        tap("bypass:30")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("63-bypass-dialog", dialogs = true)
        tap("confirmBypass")
        await("bypassClock")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("64-autopilot-bypass")
        tap("mode:LOCKDOWN")
        rule.waitForIdle()
        rule.onNodeWithText("Lock down").performClick()
        await("endLockdown")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("65-autopilot-lockdown")
    }

    @Test
    fun autopilotHeaderPill() {
        automaticHistory()
        core.model = installed
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 14 * 60 + 32)
        launch()
        top()
        await("modePill")
        shoot("66-home-bypass-pill")
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.LOCKDOWN)
        tap("openSettings")
        tap("openAutopilot")
        await("endLockdown")
        rule.onNodeWithTag("endLockdown").performClick()
        await("mode:MANUAL")
        rule.onNodeWithTag("mode:AUTO").performClick()
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        pressBack()
        pressBack()
        top()
        await("modePill")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("66b-home-auto-pill")
    }

    /** The activity list opens at the oldest unseen entry; these shots want its top. */
    private fun top() {
        await("activityList")
        repeat(3) { settle() }
        rule.onNodeWithTag("activityList").performScrollToIndex(0)
        rule.waitForIdle()
    }

    private fun pressBack() {
        scenario!!.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    @Test
    fun autopilotModel() {
        openAutopilot()
        rule.onNodeWithTag("modelCard").performScrollTo()
        shoot("67-autopilot-no-model")
        core.model = TestData.modelStatus(dev.reins.core.ModelState.DOWNLOADING, downloaded = 151_000_000u, size = 412_000_000u)
        core.downloadFailure = null
        tap("wifiOnly")
        await("modelProgress")
        rule.onNodeWithTag("modelCard").performScrollTo()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("67b-autopilot-downloading")
    }

    @Test
    fun autopilotModelFailed() {
        core.model = TestData.modelStatus(dev.reins.core.ModelState.FAILED, error = "sha-256 of model.onnx does not match the pinned hash")
        openAutopilot()
        await("modelError")
        rule.onNodeWithTag("modelCard").performScrollTo()
        shoot("68-autopilot-model-failed")
    }

    @Test
    fun autopilotProfile() {
        core.model = installed
        openAutopilot()
        tap("profile:personal")
        await("class:github/write/push")
        repeat(30) { rule.mainClock.advanceTimeBy(50) }
        shoot("70-profile")
        tap("class:gmail/read")
        await("lock:off:gmail/read")
        rule.onNodeWithTag("class:telegram/send").performScrollTo()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("71-profile-classes")
        tap("lock:off:gmail/read")
        rule.waitForIdle()
        shoot("71b-profile-unlock", dialogs = true)
    }

    @Test
    fun autopilotTryIt() {
        core.model = installed
        openAutopilot()
        tap("openTryIt")
        await("situation")
        shoot("72-try-it")
        tap("evaluate")
        await("verdict")
        rule.onNodeWithTag("verdict").performScrollTo()
        repeat(30) { rule.mainClock.advanceTimeBy(50) }
        shoot("73-try-it-verdict")
        rule.onNodeWithTag("situation").performScrollTo()
        rule.onNodeWithTag("situation").performTextReplacement(dev.reins.android.autopilot.AutopilotText.examples.last().situation)
        tap("evaluate")
        await("verdictReason")
        rule.onNodeWithTag("verdict").performScrollTo()
        repeat(30) { rule.mainClock.advanceTimeBy(50) }
        shoot("73b-try-it-deny")
    }

    @Test
    fun suggestionStrip() {
        history()
        core.model = installed
        val view = TestData.gitPushView(TestData.gitPush(TestData.gitRef("feature/laya", commitCount = 3u, filesChanged = 7u, additions = 120u, deletions = 14u))).copy(waitUntil = now + 40)
        core.pending = listOf(TestData.pending(view.requestId, "write", 1u, waitUntil = now + 40, label = view.connectionLabel, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle, suggestion = "Autopilot would approve · 97%"))
        core.approval = view
        core.suggestions[view.requestId] = TestData.suggestion(view.requestId)
        launch(link("request", view.requestId))
        await("suggestion")
        repeat(30) { rule.mainClock.advanceTimeBy(50) }
        shoot("74-suggestion")
        tap("suggestionToggle")
        await("suggestionDetail")
        repeat(30) { rule.mainClock.advanceTimeBy(50) }
        shoot("75-suggestion-open")
    }

    @Test
    fun suggestionFloor() {
        history()
        core.model = installed
        val view = TestData.vaultView().copy(waitUntil = now + 40)
        core.pending = listOf(TestData.pending(view.requestId, "read", 1u, waitUntil = now + 40, service = "vault", account = "me@example.com", op = "get", opTitle = view.opTitle))
        core.approval = view
        core.suggestions[view.requestId] = TestData.suggestion(view.requestId, floor = true, judged = false, reason = "", neighbours = emptyList())
        launch(link("request", view.requestId))
        await("suggestion")
        tap("suggestionToggle")
        await("suggestionDetail")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("75b-suggestion-floor")
    }

    @Test
    fun pendingWithSuggestion() {
        automaticHistory()
        core.model = installed
        core.pending = listOf(
            TestData.pending("req1", "write", 1u, waitUntil = now + 34, label = "Claude", conn = "c1", service = "github", account = "dkat", op = "push", opTitle = "Push to a branch", suggestion = "Autopilot would approve · 97%"),
        )
        launch()
        await("pending:req1")
        shoot("76a-activity-pending-suggestion")
    }

    @Test
    fun activityAutomatic() {
        automaticHistory()
        core.model = installed
        launch()
        top()
        await("filter:automatic")
        tap("filter:automatic")
        await("entry:7")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("76-activity-automatic")
        tap("entry:9")
        await("entryAutopilot")
        rule.onNodeWithTag("entryAutopilot").performScrollTo()
        repeat(30) { rule.mainClock.advanceTimeBy(50) }
        shoot("77-entry-autopilot")
    }

    @Test
    fun connectionAutopilot() {
        history()
        core.model = installed
        core.apConnections["c1"] = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO, profileId = "work")
        launch()
        tap("openSettings")
        tap("connection:c1")
        await("connectionMode")
        rule.onNodeWithTag("connProfile:work").performScrollTo()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("78-connection-autopilot")
    }

    /** "Use another server" on the welcome page, with a self-hosted server typed in (the password forms are for one). */
    private fun useOwnServer() {
        tap("useAnotherServer")
        await("server")
        rule.onNodeWithTag("server").performTextReplacement("https://reins.example.com")
    }

    @Test
    fun signIn() {
        core.session = null
        launch()
        await("welcome")
        shoot("14-welcome")
        useOwnServer()
        await("startSignIn")
        shoot("14j-welcome-own-server")
        tap("startSignIn")
        await("signIn")
        shoot("14a-signin")
    }

    @Test
    fun unlock() {
        core.session = null
        core.ssoKeys = dev.reins.core.AccountKeys.LOCKED
        launch()
        tap("continue")
        rule.waitUntil(10_000) { container().ssoSignIn.pending() != null }
        scenario?.close()
        launch(
            Intent(context, MainActivity::class.java)
                .setAction(dev.reins.android.platform.SsoRedirectActivity.ACTION_SIGNED_IN)
                .setData(android.net.Uri.parse("com.reins2fa.app://sso-callback?code=c0de&state=${FakeCore.SSO_STATE}")),
        )
        await("unlock")
        shoot("14g-unlock")
        core.joinWaits = Int.MAX_VALUE
        tap("askOtherPhone")
        await("joinCode")
        shoot("14h-unlock-asking")
        tap("cancelJoin")
        tap("enterRecoveryCode")
        await("recoveryCode")
        shoot("14i-unlock-recovery-code")
    }

    @Test
    fun joinSheet() {
        core.pending = listOf(TestData.joinItem())
        core.joins["join1"] = TestData.joinView()
        launch(link("join", "join1"))
        await("joinCode")
        shoot("7b-join")
    }

    @Test
    fun recoverySetup() {
        context.getSharedPreferences("recovery-record", Context.MODE_PRIVATE).edit().clear().commit()
        core.recoveryCode = FakeCore.RECOVERY_CODE
        launch()
        await("recoveryRecorded")
        shoot("14j-required-recovery", dialogs = true)
    }

    @Test
    fun recoveryCode() {
        dev.reins.android.state.RecoveryRecord(context).confirm(core.session!!.serverUrl, FakeCore.RECOVERY_CODE)
        core.recoveryCode = FakeCore.RECOVERY_CODE
        val biometrics = dev.reins.android.platform.AuthenticatorProvider.factory
        dev.reins.android.platform.AuthenticatorProvider.factory = {
            dev.reins.android.platform.Authenticator { _, _ -> dev.reins.android.platform.AuthResult.Success }
        }
        try {
            launch()
            tap("openSettings")
            tap("recoveryCodeRow")
            await("recoveryCodeSheet")
            shoot("50b-recovery-code", dialogs = true)
        } finally {
            dev.reins.android.platform.AuthenticatorProvider.factory = biometrics
        }
    }

    private fun container() = (context.applicationContext as ReinsApp).container

    @Test
    fun createAccount() {
        core.session = null
        launch()
        useOwnServer()
        tap("createAccount")
        await("create")
        rule.onNodeWithTag("email").performTextReplacement("me@example.com")
        rule.onNodeWithTag("password").performTextReplacement("correct horse battery")
        rule.onNodeWithTag("confirmPassword").performTextReplacement("correct horse battery")
        shoot("14b-create-account")
    }

    @Test
    fun setupAfterSigningIn() {
        core.session = null
        launch()
        useOwnServer()
        tap("startSignIn")
        await("signIn")
        rule.onNodeWithTag("email").performTextReplacement("me@example.com")
        rule.onNodeWithTag("password").performTextReplacement("correct horse battery")
        tap("signIn")
        await("setupWelcome")
        shoot("14a-setup-welcome")
        tap("setupNext")
        await("setupNotifications")
        shoot("14b-setup-notifications")
        if (rule.onAllNodes(hasTestTag("notificationsLater")).fetchSemanticsNodes().isNotEmpty()) tap("notificationsLater") else tap("setupNext")
        await("setupIntegrations")
        shoot("14b2-setup-integrations")
        tap("setupNext")
        await("setupRules")
        shoot("14b25-setup-rules")
        tap("setupNext")
        await("setupAutopilot")
        shoot("14b3-setup-autopilot")
        tap("setupNext")
        await("setupComputer")
        shoot("14c-setup-computer")
        tap("typeCode")
        await("pairCode")
        shoot("14d-setup-type-code")
        tap("setupNext")
        await("setupAi")
        shoot("14e-setup-ai")
        tap("setupNext")
        await("setupFinished")
        shoot("14f-setup-done")
    }

    @Test
    fun connectComputer() {
        launch()
        tap("openSettings")
        tap("connectComputer")
        await("scanQr")
        shoot("14f-connect-computer")
    }

    @Test
    fun approvalOneTap() {
        openView(TestData.quick(TestData.searchView()))
        await("approveAllow")
        shoot("90-approval-one-tap")
    }

    @Test
    fun approvalRepeated() {
        openView(TestData.quick(TestData.searchView(), repeats = 3u))
        await("repeatHint")
        shoot("91-approval-repeated")
    }

    @Test
    fun activityBurst() {
        history()
        core.pending = listOf(
            TestData.pending("req1", "search", 4u, waitUntil = now + 34, quick = true),
            TestData.pending("req2", "read", 2u, waitUntil = now + 36, quick = true),
            TestData.pending("req3", "send", 1u, waitUntil = now + 38, quick = true),
            TestData.pending("req4", "grant", 1u, waitUntil = now + 40),
        )
        launch()
        await("burst")
        shoot("92-activity-burst")
    }

    @Test
    fun devices() {
        core.resetDevices()
        core.connections = listOf(TestData.connection("c1", "Claude"), TestData.computer("d1", "MacBook Pro"))
        launch()
        tap("openSettings")
        tap("devicesRow")
        await("device:d-old")
        shoot("99-devices")
    }

    private fun openVault() {
        core.resetVault()
        core.accounts = listOf(dev.reins.core.AccountView("vault", "me@example.com", now))
        launch()
        tap("integrations")
        tap("service:vault")
        await("openVault")
    }

    @Test
    fun vault() {
        openVault()
        shoot("93-vault-service")
        tap("openVault")
        await("vaultItem:openai")
        shoot("94-vault-list")
        tap("vaultItem:openai")
        await("vaultUse:vault:OpenAI/password")
        shoot("95-vault-item")
        tap("vaultEdit")
        await("vaultInput:password")
        shoot("96-vault-edit")
    }

    @Test
    fun vaultAdd() {
        openVault()
        tap("openVault")
        tap("vaultAdd")
        await("newItem:ApiKey")
        shoot("97-vault-add")
        tap("newItem:SshKey")
        await("vaultName")
        rule.onNodeWithTag("vaultName").performTextInput("Laptop")
        tap("vaultGenerate")
        await("sshMade")
        shoot("98-vault-ssh-made")
    }

    companion object {
        @JvmStatic
        @BeforeClass
        fun installFakeCore() {
            CoreProvider.factory = CoreFactory { FakeCore.shared }
        }
    }
}

@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-night-xxhdpi")
class ScreenshotsDark : ScreenshotsBase("dark")

@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-notnight-xxhdpi")
class ScreenshotsLight : ScreenshotsBase("light")
