package dev.reins.android

import android.content.Context
import android.content.Intent
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.swipeDown
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
        // Robolectric reports window blurs off; the screenshots show the frosted panes unless a test asks otherwise.
        dev.reins.android.design.Transparency.forceSolid = false
        Foreground.focused = false
        Foreground.autoPopup = false
        core.resetPrimarySetup()
        core.permissionViews.clear(); core.savedPermissionDrafts.clear(); core.delegationRecords.clear(); core.permissionFailure = null; core.permissionSaveGate = null; core.permissionSaveCalls.set(0); core.permissionScopeGates.clear(); core.permissionScopeOverride = null
        core.session = SessionInfo("https://app.reins2fa.com", "me@example.com")
        core.pending = emptyList()
        core.startingPolicy = null
        core.approval = null
        core.pairing = null
        core.grants = emptyList()
        core.connections = listOf(TestData.connection("c1", "Claude"), TestData.connection("c2", "My ChatGPT"), TestData.connection("c3", "Hermes agent"), TestData.connection("c4", "notes-bot"))
        core.gmail = GmailStatus.Ready
        core.activity = emptyList()
        core.entryApprovals.clear()
        core.entryPushes.clear()
        core.commitChanges.clear()
        core.commitChangeGate = null
        core.catalogue = FakeCore.defaultServices()
        core.mcp = emptyList()
        core.blobs.clear()
        core.resetAutopilot()
        core.resetSso()
        core.resetOnboarding()
        core.payments = null
        core.spending = dev.reins.core.SpendingView(0, emptyList(), emptyList(), emptyList())
        UpdateProvider.fetcher = updates
        UpdateProvider.installer = FakeInstaller()
        androidx.work.testing.WorkManagerTestInitHelper.initializeTestWorkManager(context)
        (context.applicationContext as ReinsApp).container.state.setSession(dev.reins.android.state.SessionState.Loading)
    }

    @After
    fun tearDown() {
        scenario?.close()
        dev.reins.android.ui.settings.appPushOverride = null
        Timers.live = true
        Timers.frozenNowMillis = null
        dev.reins.android.design.Transparency.forceSolid = null
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
            // A tag inside a merged row (a row's detail line) is found in the unmerged tree.
            rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() ||
                rule.onAllNodes(hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
    }

    /** Opens every closed day of the river (only Today opens by itself), scrolling down to reach them all. */
    private fun openAllDays() {
        val closedDay = androidx.compose.ui.test.SemanticsMatcher("closed day") { n ->
            n.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.TestTag)?.startsWith("day:") == true &&
                n.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.StateDescription) == "Collapsed"
        }
        repeat(12) {
            settle()
            if (rule.onAllNodes(closedDay).fetchSemanticsNodes().isNotEmpty()) {
                rule.onAllNodes(closedDay)[0].performScrollTo().performClick()
            } else if (rule.onAllNodes(hasTestTag("activityList")).fetchSemanticsNodes().isNotEmpty()) {
                rule.onNodeWithTag("activityList").performTouchInput { swipeUp() }
            }
        }
        if (rule.onAllNodes(hasTestTag("activityList")).fetchSemanticsNodes().isNotEmpty()) {
            rule.onNodeWithTag("activityList").performScrollToIndex(0)
        }
        settle()
    }

    /** A service's row on Connections: connected ones in the Services group, the rest behind its +. */
    private fun openService(id: String) {
        await("addService")
        if (rule.onAllNodes(hasTestTag("service:$id")).fetchSemanticsNodes().isEmpty()) tap("addService")
        tap("service:$id")
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
    private fun shoot(name: String, dialogs: Boolean = false, idle: Boolean = true) {
        val dir = System.getProperty("reins.screenshots")
        // A frame in the middle of an animation is taken as it is (the clock stopped), without waiting for idle.
        if (idle) rule.waitForIdle()
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
                    val lp = root.layoutParams as? android.view.WindowManager.LayoutParams
                    val dims = lp != null && (lp.flags and android.view.WindowManager.LayoutParams.FLAG_DIM_BEHIND) != 0
                    canvas.save()
                    if (dims || lp == null) {
                        canvas.drawColor(android.graphics.Color.argb(((lp?.dimAmount ?: 0f) * 255).toInt(), 0, 0, 0))
                        // Dialogs are centred on screen (Robolectric does not lay their windows out).
                        canvas.translate((view.width - root.width) / 2f, (view.height - root.height) / 2f)
                    } else {
                        // A popup (a menu that grows from its trigger) sits where its position provider put it.
                        canvas.translate(lp.x.toFloat(), lp.y.toFloat())
                    }
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
    fun integrationConnectionCompletion() {
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        launch()
        tap("tabConnections")
        openService("gmail")
        val app = (context.applicationContext as ReinsApp).container
        kotlinx.coroutines.runBlocking { dev.reins.android.ui.gmail.GmailViewModel(app).connect("ada.long.address@gmail.com") }
        await("permissionPreset:REVIEW_EVERY_REQUEST")
        shoot("integration-account-connected")
    }

    @Suppress("DEPRECATION")
    @Test
    fun integrationConnectionCompletionLargeText() {
        val resources = context.resources
        val previous = android.content.res.Configuration(resources.configuration)
        val enlarged = android.content.res.Configuration(previous).apply { fontScale = 1.6f }
        resources.updateConfiguration(enlarged, resources.displayMetrics)
        try {
            core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
            launch()
            tap("tabConnections")
            openService("gmail")
            val app = (context.applicationContext as ReinsApp).container
            kotlinx.coroutines.runBlocking { dev.reins.android.ui.gmail.GmailViewModel(app).connect("ada.long.address@gmail.com") }
            await("permissionPreset:REVIEW_EVERY_REQUEST")
            shoot("integration-account-connected-large-text")
            tap("connectionDone")
            await("service:gmail")
        } finally {
            resources.updateConfiguration(previous, resources.displayMetrics)
        }
    }


    @Test
    fun permissionChangesReview() {
        Timers.frozenNowMillis = 1_800_014_400_000L // 2027-01-15; active permissions expire in the future.
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        launch(); tap("tabConnections"); openService("gmail"); tap("accountPermissions:me@gmail.com")
        tap("permissionPreset:READ_ASSIST"); tap("permissionAgent:c2"); tap("permissionsPreview")
        await("permissionPreviewSummary")
        shoot("permissions-review")
        rule.onNodeWithTag("permissionsSave").performScrollTo()
        shoot("permissions-review-limits")
    }

    @Test
    fun temporaryPermissionControls() {
        Timers.frozenNowMillis = 1_800_014_400_000L // 2027-01-15; active permissions expire in the future.
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        kotlinx.coroutines.runBlocking {
            val draft = core.delegationDraft("gmail", "me@gmail.com")
            core.issueDelegation(draft, object : dev.reins.core.OwnerCheck { override suspend fun confirm(reason: String) = true })
        }
        launch(); tap("tabTrust"); tap("trustPermissions")
        await("delegation:temporary-0")
        rule.onNodeWithTag("delegation:temporary-0").performScrollTo()
        shoot("permissions-temporary")
    }

    private fun openPrimaryPhoneSetup() {
        Timers.frozenNowMillis = 1_800_014_400_000L
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        core.permissionViews["gmail/me@gmail.com"] = PermissionFixtures.view("gmail", "me@gmail.com").let { view ->
            view.copy(availablePresets = listOf(dev.reins.core.AccountPermissionPreset.REVIEW_EVERY_REQUEST, dev.reins.core.AccountPermissionPreset.CUSTOM),
                capabilities = view.capabilities.map { it.copy(supportedModes = listOf(dev.reins.core.PermissionMode.ASK, dev.reins.core.PermissionMode.BLOCK)) })
        }
        launch(); tap("tabConnections"); openService("gmail"); tap("accountPermissions:me@gmail.com")
        await("primarySetupOpen")
    }

    @Test
    fun primaryPhoneSetup() {
        openPrimaryPhoneSetup(); shoot("permissions-primary-setup-entry")
        tap("primarySetupOpen"); await("primarySetupDialog"); shoot("permissions-primary-setup", dialogs = true)
    }

    @Suppress("DEPRECATION")
    @Test
    fun primaryPhoneSetupLargeText() {
        val resources = context.resources
        val previous = android.content.res.Configuration(resources.configuration)
        resources.updateConfiguration(android.content.res.Configuration(previous).apply { fontScale = 1.6f }, resources.displayMetrics)
        try {
            openPrimaryPhoneSetup(); tap("primarySetupOpen"); await("primarySetupDialog")
            rule.onNodeWithTag("primarySetupConfirm").performScrollTo()
            shoot("permissions-primary-setup-large-text", dialogs = true)
        } finally { resources.updateConfiguration(previous, resources.displayMetrics) }
    }

    @Test
    fun unsupportedAccountPermissions() {
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        launch(); tap("tabConnections"); openService("github")
        val app = (context.applicationContext as ReinsApp).container
        kotlinx.coroutines.runBlocking { app.state.accountConnected(dev.reins.core.AccountView("github", "octo-cat", now), app.state.accountEpoch.value) }
        await("permissionsReadOnly")
        shoot("permissions-unsupported-account")
    }

    @Test
    fun permissionAuthorizationActors() {
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        core.activity = listOf("account_permission", "temporary_permission").mapIndexed { index, actor ->
            TestData.entry((index + 1).toLong(), "send", "sent", 1u, now - (index + 1) * 60,
                decidedBy = actor, label = TestData.label("gmail", "send", "Send an email to", "friend@example.com"))
        }
        launch(); await("entry:1"); shoot("activity-permission-actors")
        tap("entry:1"); await("entryOutcome"); shoot("activity-account-permission-detail")
        pressBack(); await("entry:2"); tap("entry:2"); await("entryOutcome"); shoot("activity-temporary-permission-detail")
    }

    @Test
    fun uncertainExecutionOutcomes() {
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "me@gmail.com", now))
        core.activity = listOf("unknown", "succeeded_withheld", "started").mapIndexed { index, status ->
            TestData.entry((index + 1).toLong(), "send", status, 1u, now - (index + 1) * 60,
                decidedBy = "autopilot", grantId = "g1", label = TestData.label("gmail", "send", "Send an email to", "friend@example.com"))
        }
        launch(); await("entry:1")
        shoot("activity-uncertain-outcomes")
        tap("entry:1"); await("entryOutcome")
        shoot("activity-unknown-detail")
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

    /** The frosted panes over Halo in both themes: a day's pane runs past the art's foot, then under the title. */
    @Test
    fun activityGlassHalo() {
        history()
        core.activity = (20 downTo 7).map { id ->
            TestData.entry(id.toLong(), if (id % 2 == 0) "read" else "search", "released", 1u, at = now - 60L * (21 - id))
        } + core.activity
        val halo = dev.reins.android.design.WallpaperChoice.of(dev.reins.android.design.Wallpaper.Halo)
        dev.reins.android.design.Appearance.set(context, dev.reins.android.design.AppearanceChoice(wallpaperDark = halo, wallpaperLight = halo))
        try {
            launch()
            // The river opens at the new entries, so a day's pane runs across the art's foot and under the title.
            repeat(20) { settle(); Thread.sleep(100) }
            shoot("glass-halo-scrolled")
            rule.onNodeWithText("Latest").performClick()
            repeat(20) { settle(); Thread.sleep(100) }
            shoot("glass-halo")
        } finally {
            dev.reins.android.design.Appearance.set(context, dev.reins.android.design.AppearanceChoice())
        }
    }

    // ---- the approval screen, as designed (reins-designs/approval.html) ------------------------------------------

    private fun designApproval(
        view: dev.reins.core.ApprovalView,
        suggestion: dev.reins.core.SuggestionView? = TestData.suggestion(view.requestId),
        installed: Boolean = true,
    ) {
        core.connections = listOf(dev.reins.core.ConnectionView("c1", "Claude Code", "laptop", 1_700_000_000, 1_700_000_100, "claude", null))
        core.model = TestData.modelStatus(if (installed) dev.reins.core.ModelState.INSTALLED else dev.reins.core.ModelState.NOT_INSTALLED)
        suggestion?.let { core.suggestions[view.requestId] = it }
        core.pending = listOf(TestData.pending(view.requestId, view.action, 1u, waitUntil = view.waitUntil, label = view.connectionLabel, service = view.service, op = view.op, opTitle = view.opTitle))
        core.approval = view
        launch(link("request", view.requestId))
        await("approve")
        repeat(3) { settle() }
    }

    private fun pushView() = TestData.designPushView().copy(createdAt = now - 20, waitUntil = now + 40)

    @Test
    fun aprGit() {
        designApproval(pushView(), TestData.suggestion("req20", reason = "Like 4 pushes to this repo you approved"))
        shoot("android-approval-git")
    }

    /** A force push with hidden characters to the default branch, judged risky and held by Overnight: every line. */
    @Test
    fun aprWarnings() {
        val ref = TestData.gitRef("main", force = true, commitCount = 2u, commits = listOf(TestData.designCommit().copy(hiddenChars = true)), filesChanged = 1u)
        val view = pushView().copy(git = pushView().git!!.copy(refs = listOf(ref), defaultBranch = "main"), noStanding = true)
        designApproval(
            view,
            TestData.suggestion("req20", mode = dev.reins.core.AutopilotMode.OVERNIGHT, risk = TestData.risk(dev.reins.core.RiskLevel.CATASTROPHIC, "rewrites main", held = true)),
        )
        shoot("sg-apr-warnings")
    }

    /** The Reins desktop app pairing: Reins' own ask, the name quoted on its row, the key's digits apart. */
    @Test
    fun pairingComputer() {
        history()
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", "M2A", "laptop", byteArrayOf(7, 42, 88), 1_700_000_200, "4821 9930")
        launch(link("pairing", "p1"))
        await("keyFingerprint")
        shoot("sg-pair-computer")
    }

    @Test
    fun aprGmail() {
        designApproval(TestData.designMailView(now + 12).copy(createdAt = now - 48), TestData.suggestion("req1", verdict = dev.reins.core.Verdict.ASK, pApprove = 0.62f, pDeny = 0.2f))
        shoot("android-approval-gmail")
    }

    @Test
    fun aprCustom() {
        designApproval(pushView(), TestData.suggestion("req20"))
        tap("lifetimeCustom")
        tap("lifetime:UNTIL_REVOKED")
        await("foreverWarning")
        shoot("android-approval-custom")
    }

    @Test
    fun aprAutopilot() {
        designApproval(pushView(), TestData.suggestion("req20", reason = "Like 4 pushes to this repo you approved"))
        tap("suggestion")
        await("suggestionDetail")
        shoot("android-approval-autopilot")
    }

    @Test
    fun aprNoModel() {
        designApproval(pushView(), null, installed = false)
        await("getAutopilot")
        shoot("android-approval-no-model")
    }

    @Test
    fun aprCommits() {
        designApproval(pushView())
        tap("gitCommits:0")
        await("gitCommit:0:0")
        shoot("android-approval-commits")
    }

    @Test
    fun aprCommit() {
        designApproval(pushView())
        tap("gitCommits:0")
        tap("gitCommit:0:0")
        tap("commitFile:0")
        await("commitFile:0:diff")
        shoot("android-approval-commit")
    }

    @Test
    fun aprDanger() {
        designApproval(TestData.designDeleteView().copy(createdAt = now - 5, waitUntil = now + 55), TestData.suggestion("req11", floor = true, judged = false))
        shoot("android-approval-danger")
        tap("help:onceWarning")
        await("helpBubble")
        shoot("android-approval-danger-help")
    }

    @Test
    fun approvalSheet() {
        history()
        core.pending = listOf(TestData.pending("req1", "search", 3u, waitUntil = now + 34))
        core.approval = TestData.searchView(waitUntil = now + 34)
        launch(link("request", "req1"))
        await("approve")
        shoot("2-approval")
        tap("lifetimeCustom")
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

    /** A second Claude.ai account: "Claude 2" in the name field, the cursor in it, and why. */
    @Test
    fun pairingSecondAccount() {
        history()
        core.connections = listOf(TestData.connection("c1", "Claude").copy(ownLabel = "Claude", clientName = "Claude"))
        core.pending = listOf(TestData.pairingItem("p2"))
        core.pairing = PairingView("p2", "Claude", "claude.ai", byteArrayOf(7, 42, 88), 1_700_000_200, null, "Claude 2", 1u)
        launch(link("pairing", "p2"))
        await("pairSameClient")
        rule.onNodeWithTag("label").performScrollTo()
        shoot("7b-pairing-second-account")
    }

    /** A connection's Name: the field with the user's name, its own as the placeholder, what it calls itself. */
    @Test
    fun renameConnection() {
        history()
        core.connections = listOf(
            TestData.connection("c1", "Claude (work)").copy(name = "Claude (work)", ownLabel = "Claude", clientName = "Claude", lastUsedAt = now - 300),
            TestData.connection("c2", "Claude").copy(ownLabel = "Claude", clientName = "Claude"),
        )
        launch()
        tap("tabConnections")
        tap("connection:c1")
        await("connectionCalledItself")
        shoot("13c-connection-renamed")
        tap("connectionRename")
        await("renameField")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("13d-connection-rename-field")
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
        tap("tabTrust")
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
        tap("tabTrust")
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
        tap("tabConnections")
        await("service:gmail")
        shoot("15-integrations")
        openService("gmail")
        await("account:me@gmail.com")
        shoot("16-gmail-accounts")
    }

    @Test
    fun newGrant() {
        history()
        launch()
        tap("tabTrust")
        tap("newGrant")
        await("createGrant")
        shoot("11-new-grant")
    }

    // ---- Reins Next: the boards' own cases ---------------------------------------------------------------------------

    @Test
    fun nextWallpaperPicker() {
        launch()
        tap("tabSettings")
        tap("openWallpaper")
        await("wallpaperPreview")
        // The Effect row's pictures are made off the main thread, which the test clock does not wait for.
        settleWallpaper()
        shoot("next-wallpaper-picker")
        tap("wallpaperFor:light")
        settleWallpaper()
        shoot("next-wallpaper-picker-light")
        // An image that ships as it was takes any effect: Eyes (picked with its scanlines) in halftone.
        tap("wallpaper:Eyes")
        tap("effect:Halftone")
        // The picture is made off the main thread, which the test clock does not wait for.
        repeat(10) {
            Thread.sleep(500)
            frames()
        }
        shoot("next-wallpaper-effect")
        dev.reins.android.design.Appearance.set(context, dev.reins.android.design.AppearanceChoice())
    }

    private fun settleWallpaper() = repeat(4) {
        Thread.sleep(500)
        frames()
    }

    /** Activity with only history: the art stays open; then the same with transparency reduced (solid panes). */
    @Test
    fun nextActivityWallpaper() {
        history()
        launch()
        await("activityList")
        frames()
        shoot("next-activity-history")
        scenario?.close()
        dev.reins.android.design.Transparency.forceSolid = true
        launch()
        await("activityList")
        frames()
        shoot("next-activity-reduce-transparency")
    }

    /** Another wallpaper (Ruins) under waiting cards. */
    @Test
    fun nextActivityRuins() {
        history()
        core.pending = listOf(TestData.pending("req1", "write", 1u, waitUntil = now + 40, label = "M2A", conn = "c1", service = "github", account = "octo-cat", op = "push", opTitle = "Push to a branch"))
        dev.reins.android.design.Appearance.set(context, dev.reins.android.design.AppearanceChoice(wallpaperDark = dev.reins.android.design.WallpaperChoice.of(dev.reins.android.design.Wallpaper.Ruins), wallpaperLight = dev.reins.android.design.WallpaperChoice(dev.reins.android.design.Wallpaper.Torii)))
        launch()
        await("pending:req1")
        frames()
        shoot("next-activity-ruins")
        dev.reins.android.design.Appearance.set(context, dev.reins.android.design.AppearanceChoice())
    }

    @Test
    fun settings() {
        history()
        core.connections = listOf(
            TestData.connection("c1", "Claude Code").copy(lastUsedAt = now - 300),
            TestData.connection("c2", "ChatGPT").copy(clientHost = "chatgpt.com", lastUsedAt = null),
            TestData.computer("d1", "MacBook Pro").copy(lastUsedAt = now - 10),
        )
        launch()
        tap("tabSettings")
        await("accountCard")
        await("devicesRow")
        val state = (context.applicationContext as ReinsApp).container.state
        rule.waitUntil(5_000) { settle(); !state.approvalDevice.value }
        await("registerPhone")
        shoot("12e-settings-other-phone")
        dev.reins.android.ui.settings.appPushOverride = true
        state.setApprovalDevice(true)
        await("notificationsChip")
        shoot("12-settings")
        rule.onNodeWithTag("pageEnd").performScrollTo()
        shoot("12b-settings-scrolled")
        // Connecting and a connection's page live on Connections now.
        tap("tabConnections")
        tap("connect")
        await("sheetScanQr")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("12c-settings-connect")
        pressBack()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        tap("connection:c1")
        await("disconnect")
        await("connMode:follow")
        shoot("13-connection")
        // Holding Disconnect: the fill, the ring, "Keep holding".
        rule.onNodeWithTag("disconnect").performScrollTo()
        rule.mainClock.autoAdvance = false
        rule.onNodeWithTag("disconnect").performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(1_100)
        shoot("13b-connection-holding")
        rule.onNodeWithTag("disconnect").performTouchInput { up() }
        rule.mainClock.autoAdvance = true
    }

    /** The account card when notifications are off, and when another phone approves. */
    @Test
    fun settingsAttention() {
        shadowOf(context as android.app.Application).denyPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        launch()
        tap("tabSettings")
        (context.applicationContext as ReinsApp).container.state.setApprovalDevice(true)
        await("turnOnNotifications")
        shoot("12d-settings-notifications-off")
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
        tap("tabConnections")
        await("service:vault")
        shoot("15-integrations")
        openService("telegram")
        await("account:+15550100")
        shoot("16-telegram")
    }

    @Test
    fun telegramSignIn() {
        launch()
        await("tabConnections")
        tap("tabConnections")
        openService("telegram")
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
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
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
        tap("lifetime:DAY")
        tap("lifetimeCustom")
        tap("moreToggle")
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
        tap("help:onceWarning")
        await("helpBubble")
        shoot("25-repo-delete-help")
    }

    @Test
    fun desktopPairing() {
        history()
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = TestData.pairingView("p1", "M2A", "laptop", keyFingerprint = "4821 9930")
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
        tap("gitCommits:0")
        await("gitCommit:0:0")
        shoot("29-git-push-commits")
    }

    @Test
    fun gitForcePush() {
        val ref = TestData.gitRef("main", force = true, commitCount = 2u, commits = realisticCommits().take(2), filesChanged = 3u, files = realisticFiles().take(3), additions = 142u, deletions = 10u)
        openGit(TestData.gitPushView(TestData.gitPush(ref, packBytes = 3_145_728u), noStanding = true))
        shoot("30-git-force-push")
    }

    @Test
    fun gitTagsAndUnknownHistory() {
        val tag = TestData.gitRef("v1.2.0", kind = "tag", change = "create", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        val branch = TestData.gitRef("release", forceUnknown = true, force = true, commitCount = 1u, commits = realisticCommits().take(1), filesChanged = 2u, files = realisticFiles().subList(3, 5), additions = null, deletions = null)
        val gone = TestData.gitRef("old-experiment", change = "delete", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        openGit(TestData.gitPushView(TestData.gitPush(tag, branch, gone), noStanding = true, tags = true))
        shoot("32-git-tags-unknown")
    }

    /** A push of 8 commits where 2 deserve a closer look: one changes CI (Close), one deletes a file (Look). */
    @Test
    fun attentionPush() {
        openGit(TestData.attentionPushView())
        shoot("attention-1-push")
        tap("gitCommits:0")
        await("gitCommit:0:5")
        shoot("attention-2-commits")
        tap("gitCommit:0:5")
        await("commitSubject")
        shoot("attention-3-commit")
    }

    /** A search that finds 12 emails, 2 worth a closer look (a password reset, an invoice); both start ticked. */
    @Test
    fun attentionEmails() {
        history()
        val view = TestData.attentionSearchView(waitUntil = now + 40)
        core.pending = listOf(TestData.pending(view.requestId, "search", 12u, waitUntil = now + 40, label = view.connectionLabel, account = view.account))
        core.approval = view
        launch(link("request", view.requestId))
        await("check:n11")
        shoot("attention-4-emails")
        rule.onNodeWithTag("messagesHeader").performScrollTo()
        shoot("attention-5-emails-list")
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
        await("tabConnections")
        tap("tabConnections")
        openService("github")
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
        tap("tabConnections")
        await("mcp:linear")
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
        tap("tabConnections")
        tap("connect")
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
    fun commandPresentation() = renderCommandPresentation("default")

    /** Identical synthetic release invocation for the reported command-clarity before/after comparison. */
    @Test
    fun commandReleaseBuild() = renderReleaseCommand("default")

    @Test
    fun commandReleaseBuildLargeText() {
        val configuration = android.content.res.Configuration(context.resources.configuration).apply { fontScale = 1.6f }
        context.resources.updateConfiguration(configuration, context.resources.displayMetrics)
        renderReleaseCommand("large")
    }

    private fun renderReleaseCommand(size: String) {
        core.activity = listOf(CommandClarityFixtures.entry())
        core.entryApprovals[7L] = CommandClarityFixtures.view()
        launch()
        tap("entry:7")
        await("entryHero")
        frames()
        shoot(if (size == "default") "command-release-build" else "command-release-build-$size")
        tap("commandTechnicalDetails")
        await("commandSetup")
        rule.onNodeWithTag("commandSetup").performScrollTo()
        frames()
        shoot("command-release-build-$size-technical")
        tap("commandTechnicalDetails")
        tap("revealFullInvocation")
        await("fullRequestText")
        rule.onNodeWithTag("fullRequest").performScrollTo()
        frames()
        shoot("command-release-build-$size-original")
    }

    @Test
    @Suppress("DEPRECATION")
    fun commandPresentationLargeText() {
        val configuration = android.content.res.Configuration(context.resources.configuration).apply { fontScale = 1.6f }
        context.resources.updateConfiguration(configuration, context.resources.displayMetrics)
        renderCommandPresentation("large")
    }

    private fun renderCommandPresentation(size: String) {
        val raw = "JAVA_HOME=/opt/jdk ANDROID_HOME=/opt/sdk TOKEN=fixture-secret ./gradlew :app:assembleDebug --no-daemon"
        val command = dev.reins.core.CommandPresentation(
            "Run Gradle", "./gradlew :app:assembleDebug --no-daemon",
            listOf("JAVA_HOME=•••• (value hidden)", "ANDROID_HOME=•••• (value hidden)", "TOKEN=•••• (value hidden)", "Requested directory: /home/dana/dev/reins"),
            listOf("Gradle can execute project build scripts and plugins with this process’s permissions and credentials."),
        )
        val view = TestData.askView(detail = "Command:\n$raw\n\nIn: /home/dana/dev/reins", topic = "command:./gradlew").let {
            it.copy(waitUntil = null, ask = it.ask!!.copy(question = "Run: $raw"))
        }
        core.pending = listOf(TestData.pending(view.requestId, "write", 1u, service = "desktop").copy(
            label = TestData.label("terminal", "run", "Run", raw, code = true).copy(command = command),
        ))
        core.approval = view
        launch(link("request", view.requestId))
        await("commandPresentation")
        shoot("48b-command-$size-action")
        tap("commandTechnicalDetails")
        rule.onNodeWithTag("commandSetup").performScrollTo()
        shoot("48c-command-$size-setup")
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

    // ---- Reins Secrets ------------------------------------------------------------------------------------------------

    @Test
    fun secretsUiEnvRelease() {
        openView(TestData.envReleaseView(TestData.envRelease(purpose = "Run the dev server")))
        await("envRelease")
        shoot("android-secrets-env-release")
        rule.onNodeWithTag("envAlwaysAsk").performScrollTo()
        frames()
        shoot("android-secrets-env-release-scrolled")
    }

    @Test
    fun secretsUiEnvReleaseAgent() {
        val release = TestData.envRelease(
            runner = "agent:claude-code", runnerLabel = "Claude Code", programWarning = "In a temporary folder",
            program = "/tmp/build-8841/node_modules/.bin/npm",
            command = "/tmp/build-8841/node_modules/.bin/npm run dev -- --port 3000 --host 0.0.0.0 && node scripts/seed.js --reset --users 25",
            variables = listOf("DATABASE_URL", "STRIPE_KEY", "SESSION_SECRET", "OPENAI_API_KEY", "SENTRY_DSN", "REDIS_URL", "S3_BUCKET", "S3_SECRET", "MAILGUN_KEY", "JWT_SECRET"),
        )
        openView(TestData.envReleaseView(release))
        await("envWarning")
        shoot("android-secrets-env-release-agent")
        rule.onNodeWithTag("envRoot").performScrollTo()
        frames()
        shoot("android-secrets-env-release-agent-scrolled")
    }

    @Test
    fun secretsUiEnvReleaseOneShot() {
        val release = TestData.envRelease(
            env = "production", variables = listOf("STRIPE_KEY"), print = true, alwaysAsk = true, command = "reins secrets get STRIPE_KEY", lease = 60u,
        )
        openView(TestData.envReleaseView(release))
        await("noStanding")
        shoot("android-secrets-env-release-print")
    }

    @Test
    fun secretsUiRemember() {
        openView(TestData.envReleaseView())
        click("lifetime:HOUR")
        click("lifetimeCustom")
        await("rememberSheet")
        click("moreToggle")
        await("envCommands")
        click("envCommand:prefix:2")
        frames()
        shoot("android-secrets-remember")
        click("rememberConfirm")
        frames()
        shoot("android-secrets-remember-dock")
    }

    @Test
    fun secretsUiStore() {
        openView(TestData.secretsStoreView())
        await("storeCheck")
        shoot("android-secrets-store")
        click("storeEnv:0")
        frames()
        rule.onNodeWithTag("storeTokens").performScrollTo()
        frames()
        shoot("android-secrets-store-open")
    }

    @Test
    fun secretsUiToken() {
        openView(TestData.tokenStoreView())
        await("tokenCheck")
        shoot("android-secrets-token")
    }

    @Test
    fun secretsUiGrantDetail() {
        core.grants = listOf(TestData.secretsGrant(), TestData.grant("g1"))
        core.secretsRevokeAnswer = dev.reins.core.SecretsRevoked(1u, 1u, listOf("work-laptop"))
        launch()
        tap("tabTrust")
        tap("grant:s1")
        await("revokeEverywhere")
        rule.onNodeWithTag("revokeEverywhere").performScrollTo()
        frames()
        shoot("android-secrets-grant")
        click("revokeEverywhere")
        frames()
        shoot("android-secrets-grant-confirm", dialogs = true)
        rule.onAllNodes(androidx.compose.ui.test.hasText("Revoke")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        await("notDelivered:0")
        rule.onNodeWithTag("revokeResult").performScrollTo()
        frames()
        shoot("android-secrets-grant-revoked")
    }

    @Test
    fun gitlabSignIn() {
        launch()
        await("tabConnections")
        tap("tabConnections")
        openService("gitlab")
        await("openTokenPage")
        shoot("51-gitlab-token")
    }

    @Test
    fun bitbucketSignIn() {
        launch()
        await("tabConnections")
        tap("tabConnections")
        openService("bitbucket")
        await("secret")
        shoot("52-bitbucket-token")
    }

    @Test
    fun soundsAndHaptics() {
        launch()
        tap("tabSettings")
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
        tap("tabSettings")
        await("installUpdate")
        rule.onNodeWithTag("signOut").performScrollTo()
        shoot("36-settings-updates")
    }

    @Test
    fun updatesUpToDate() {
        assumeTrue(BuildConfig.SELF_UPDATE)
        launch()
        tap("tabSettings")
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
        tap("tabTrust")
        await("mode:MANUAL")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
    }

    /** The AIs and profiles of the design: Default for Claude Code and ChatGPT, Work for Cursor. */
    private fun autopilotPeople() {
        core.connections = listOf(TestData.connection("c1", "Claude Code"), TestData.connection("c2", "ChatGPT"), TestData.connection("c3", "Cursor"))
        core.profiles = listOf(
            TestData.profile("personal", "Default", memory = 140u),
            TestData.profile("work", "Work", "💼", dev.reins.core.Preset.CAUTIOUS, isDefault = false, memory = 38u, connections = listOf("c3"), classes = listOf(TestData.classView(decisions = 9u, approved = 9u, denied = 0u)), trainedAt = null),
        )
    }

    private fun frames(n: Int = 20) = repeat(n) { rule.mainClock.advanceTimeBy(50) }

    @Test
    fun autopilotModes() {
        core.model = installed
        autopilotPeople()
        openAutopilot()
        frames()
        shoot("60-autopilot-manual")
        tap("mode:AUTO")
        frames()
        shoot("61-autopilot-auto")
        tap("mode:MANUAL")
        frames()
        tap("forAWhile"); tap("mode:BYPASS")
        await("bypassSheet")
        frames()
        shoot("63-bypass-sheet")
        tap("bypassLength:custom")
        await("bypassForSheet")
        frames()
        shoot("63b-bypass-for")
        rule.onNodeWithTag("bypassFor:untilOff").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.OnClick)
        await("untilOffWarning")
        frames()
        shoot("63c-bypass-for-until-off")
        tap("closeSheet")
        frames()
        shoot("63d-bypass-sheet-until-off")
        tap("cancelBypass")
        frames()
        tap("forAWhile"); tap("mode:LOCKDOWN")
        await("endLockdown")
        frames()
        shoot("65-autopilot-lockdown")
    }

    @Test
    fun autopilotBypass() {
        core.model = installed
        autopilotPeople()
        // A 15-minute bypass with 8:12 to go.
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 8 * 60 + 12, bypassSince = now - 6 * 60 - 48)
        launch()
        tap("tabTrust")
        await("bypassClock")
        frames()
        shoot("64-autopilot-bypass")
    }

    @Test
    fun autopilotBypassUntilOff() {
        core.model = installed
        autopilotPeople()
        core.apGlobal = FakeCore.ApRow(bypassUntil = Long.MAX_VALUE, bypassSince = now - 2 * 86_400 - 3_600)
        launch()
        top()
        await("modePill")
        shoot("64c-home-bypass-until-off-pill")
        tap("tabTrust")
        await("bypassUntilOff")
        frames()
        shoot("64b-autopilot-bypass-until-off")
    }

    /**
     * Drags Trust up by [dp] and holds still before letting go, so the page stops where the finger did (no fling): the
     * running field is then exactly as far folded as the page has scrolled.
     */
    private fun dragTrust(dp: Float) {
        rule.onNodeWithTag("trustScroll").performTouchInput {
            val px = dp * density
            down(androidx.compose.ui.geometry.Offset(centerX, height * 0.75f))
            repeat(24) { moveBy(androidx.compose.ui.geometry.Offset(0f, -px / 24f), delayMillis = 16) }
            advanceEventTime(400)
            up()
        }
        frames()
    }

    /** A running mode's field stays pinned on top of Trust and folds as the page scrolls: open, half way, compact. */
    private fun foldTrust(name: String, row: FakeCore.ApRow, clockTag: String?) {
        core.model = installed
        autopilotPeople()
        automaticHistory()
        core.apGlobal = row
        launch()
        tap("tabTrust")
        await("trustField:open")
        clockTag?.let(::await)
        frames()
        shoot("$name-open")
        dragTrust(110f)
        await("trustField:folding")
        shoot("$name-folding")
        dragTrust(400f)
        await("trustField:compact")
        clockTag?.let(::await)
        shoot("$name-compact")
        // Back at the top it opens again.
        rule.onNodeWithTag("trustScroll").performTouchInput { swipeDown(top + 10f, bottom - 10f, durationMillis = 300) }
        frames(40)
        await("trustField:open")
    }

    @Test
    fun trustBypassFolds() = foldTrust("64d-trust-bypass", FakeCore.ApRow(bypassUntil = now + 47 * 60 + 12, bypassSince = now - 12 * 60), "bypassClock")

    @Test
    fun trustOvernightFolds() = foldTrust(
        "64e-trust-overnight",
        FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.OVERNIGHT, overnightUntil = now + 6 * 3_600, overnightSince = now - 12 * 60, overnightReturn = dev.reins.core.AutopilotMode.MANUAL),
        "overnightClock",
    )

    @Test
    fun trustLockdownFolds() = foldTrust("64f-trust-lockdown", FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.LOCKDOWN), null)

    /** A bypass that approved 100 requests: the latest three and "Show all 97"; opened, the rest in place. */
    @Test
    fun trustBypassListFolds() {
        core.model = installed
        autopilotPeople()
        val subjects = listOf("an email from @bank.com", "the calendar for today", "mail about the invoice", "the thread with Anna")
        core.activity = (0 until 100).map { k ->
            TestData.entry(
                1_000L + k, action = "read", outcome = "released", count = 1u, at = now - 700 + k * 6, decidedBy = "bypass",
                label = TestData.label("gmail", "read", "Read", subjects[k % subjects.size]),
            )
        }.reversed()
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 47 * 60 + 12, bypassSince = now - 12 * 60)
        launch()
        tap("tabTrust")
        await("runningShowAll")
        frames()
        shoot("64g-trust-bypass-list-folded")
        tap("runningShowAll")
        await("runningShowLess")
        frames(20)
        shoot("64h-trust-bypass-list-open")
    }

    /** Scrolled to the end, the last row of Trust and of Connections sits above the docked tab bar. */
    @Test
    fun pageBottomsClearTheTabBar() {
        core.model = installed
        autopilotPeople()
        mcpServers()
        launch()
        tap("tabTrust")
        await("pageEnd")
        rule.onNodeWithTag("pageEnd").performScrollTo()
        frames()
        shoot("69-trust-bottom")
        tap("tabConnections")
        await("pageEnd")
        rule.onNodeWithTag("pageEnd").performScrollTo()
        frames()
        shoot("69b-connections-bottom")
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
        // Leaving Lockdown asks the owner (no prompt here): the core is simply in Auto now.
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        tap("tabTrust")
        await("mode:AUTO")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        pressBack()
        top()
        await("modePill")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("66b-home-auto-pill")
    }

    /** The mode chip's menu, grown from the chip (it replaces the old floating switcher), and Bypass from it. */
    @Test
    fun autopilotQuick() {
        automaticHistory()
        core.model = installed
        launch()
        top()
        tap("modePill")
        await("autopilotQuick")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("67-quick-menu", dialogs = true)
        tap("quick:BYPASS")
        await("bypassSheet")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("67b-quick-menu-bypass")
    }

    // ---- polish round 1 ----------------------------------------------------------------------------------------------

    private fun polishHistory() {
        val l = TestData::label
        core.activity = listOf(
            TestData.entry(7, "write", "denied", 1u, at = now - 120, service = "desktop", op = "ask", label = l("terminal", "run", "Run", "git push --force origin main", true, ""),
                request = TestData.asked("git push --force origin main \\\n  --no-verify", "Force-push the rebased branch so the PR updates", "Claude Code", "~/dev/acme-web", "Rewriting git history", now - 129)),
            TestData.entry(6, "read", "released", 1u, at = now - 300, service = "desktop", op = "ask", label = l("file", "read", "Read", "~/dev/api/.env", true, "")),
            TestData.entry(5, "write", "sent", 1u, at = now - 720, service = "vault", op = "secret_release", label = l("secrets", "secrets", "Use 3 secrets for", "npm run dev", true, ""))
                .copy(connectionLabel = "My ChatGPT", connectionId = "c2"),
            TestData.entry(4, "write", "sent", 1u, at = now - 1_200, service = "github", op = "git_push", decidedBy = "autopilot", autopilot = TestData.note(),
                label = l("github", "push", "Push 3 commits to", "acme/web", false, "")),
            TestData.entry(3, "send", "sent", 1u, at = now - 3_600, label = l("gmail", "send", "Send an email to", "ada@acme.com", false, ""))
                .copy(connectionLabel = "My ChatGPT", connectionId = "c2"),
            TestData.entry(2, "write", "sent", 1u, at = now - 7_200, service = "desktop", op = "ask", label = l("terminal", "delete", "Run", "rm -rf build/", true, "")),
        )
    }

    /** Activity rows: what was touched with what it did on its corner, and the request itself. */
    @Test
    fun polishActivity() {
        polishHistory()
        core.pending = listOf(
            TestData.pending("req9", action = "write", service = "desktop", op = "ask", opTitle = "Ask you on your phone", waitUntil = now + 40)
                .copy(label = TestData.label("terminal", "run", "Run", "npm publish --tag next", true, "")),
        )
        launch()
        top()
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-activity")
    }

    /** An entry: the approval as it was shown, read-only, with what was asked in full and a quieter "This was wrong". */
    @Test
    fun polishEntry() {
        polishHistory()
        val push = core.activity.first()
        core.activity = listOf(push.copy(decidedBy = "autopilot", autopilot = TestData.note(suggested = dev.reins.core.Verdict.DENY, pApprove = 0.09f))) + core.activity.drop(1)
        core.entryApprovals[7L] = TestData.designPushView().copy(waitUntil = null, quick = null)
        launch()
        top()
        tap("entry:7")
        await("entryApproval")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-entry")
        val node = rule.onNodeWithTag("fullRequest")
        node.performScrollTo()
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-entry-full-request")
        rule.onNodeWithTag("gitCommits:0").performScrollTo()
        tap("gitCommits:0")
        await("gitCommit:0:0")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-entry-commits")
        tap("gitCommit:0:0")
        await("commitSubject")
        tap("commitFile:0")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-entry-commit-diff")
    }

    /** The paper history header: a recorded long command, a denial, and an older automatic decision. */
    @Test
    fun historyCommand() {
        val command = "JAVA_HOME=/opt/jdk ANDROID_HOME=/opt/android ./gradlew :app:assembleDebug --no-daemon --console=plain " +
            (1..8).joinToString(" ") { "-Pbuild.option$it=development-value-$it" }
        core.activity = listOf(TestData.entry(7, "write", service = "desktop", op = "ask", at = now - 120,
            label = TestData.label("terminal", "run", "Run", command, code = true),
            request = TestData.asked(command, "Build the Android app", "Claude Code", "/home/dana/dev/reins", askedAt = now - 129)).copy(connectionLabel = "M2A", account = null))
        core.entryApprovals[7L] = TestData.askView(detail = "Command:\n$command\n\nIn: /home/dana/dev/reins", topic = null)
            .copy(waitUntil = null)
        launch()
        tap("entry:7")
        await("entryHero")
        frames()
        shoot("history-command")
        rule.onNodeWithTag("fullRequest").performScrollTo()
        frames()
        shoot("history-command-full")
    }

    @Test
    fun historyLegacyAuto() {
        core.activity = listOf(TestData.entry(7, "write", at = now - 120, service = "github", op = "git_push",
            label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins"),
            decidedBy = "autopilot", autopilot = TestData.note(risk = TestData.risk(dev.reins.core.RiskLevel.BENIGN, why = "A new branch in your repository"))).copy(account = "katulevskiy"))
        launch()
        tap("entry:7")
        await("entryHero")
        frames()
        shoot("history-legacy-auto")
    }

    @Test
    fun historyDenied() {
        core.activity = listOf(reportedPush().copy(outcome = "denied", decidedBy = "autopilot",
            autopilot = TestData.note(suggested = dev.reins.core.Verdict.DENY, pApprove = 0.09f)))
        core.entryApprovals[7L] = reportedPushView()
        launch()
        tap("entry:7")
        await("entryHero")
        frames()
        shoot("history-denied")
    }

    /** The push the user reported: 3 commits to a new branch, allowed during a bypass, as Activity shows it. */
    private fun reportedPush(): dev.reins.core.ActivityEntry {
        val full = """
            git push katulevskiy/reins
            create branch signal/desktop-release: 3 commits
              9f3c2a1 Log in with a passkey when the browser offers one
              4be81d0 Remove the old login form
              c07a9e3 Keep the session for 30 days

            github_git_push
            repo: katulevskiy/reins
        """.trimIndent()
        core.connections = listOf(dev.reins.core.ConnectionView("c1", "Reins desktop app", "laptop", 1_700_000_000, 1_700_000_100, "claude", null))
        return TestData.entry(
            7, action = "write", outcome = "released", at = now - 120, service = "github", op = "git_push", opTitle = "Push with git",
            label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins"),
            request = TestData.asked(full, agent = "git", askedAt = now - 123), decidedBy = "bypass",
        ).copy(account = "katulevskiy")
    }

    private fun reportedPushView(): dev.reins.core.ApprovalView {
        val design = TestData.designPushView()
        val commits = listOf(TestData.designCommit()) + design.git!!.refs[0].commits.subList(1, 3)
        val ref = TestData.gitRef(
            "signal/desktop-release", change = "create", commitCount = 3u, commits = commits, filesChanged = 5u,
            files = TestData.designCommit().files, additions = 199u, deletions = 87u,
        )
        return design.copy(waitUntil = null, quick = null, account = "katulevskiy", connectionLabel = "Reins desktop app", git = design.git!!.copy(repo = "katulevskiy/reins", refs = listOf(ref)))
    }

    @Test
    fun pushInspect() {
        core.activity = listOf(reportedPush())
        core.entryApprovals[7L] = reportedPushView()
        launch()
        tap("entry:7")
        await("entryApproval")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-entry")
        tap("gitCommits:0")
        await("gitCommit:0:0")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-entry-commits")
        tap("gitCommit:0:0")
        await("commitSubject")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-entry-commit")
        tap("commitFile:0")
        await("commitFile:0:diff")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-entry-diff")
    }

    /** The same push decided on another device: its commits synced with the entry, a diff loaded from GitHub. */
    @Test
    fun pushOtherDevice() {
        val view = reportedPushView()
        val full = view.git!!.refs[0].commits[0]
        val synced = view.git!!.copy(refs = view.git!!.refs.map { r -> r.copy(commits = r.commits.map { c -> c.copy(files = c.files.map { it.copy(diff = emptyList()) }) }, files = r.files.map { it.copy(diff = emptyList()) }) })
        val entry = reportedPush()
        core.activity = listOf(entry.copy(request = entry.request!!.copy(git = synced)))
        core.entryApprovals[7L] = view.copy(git = synced)
        core.entryPushes[7L] = dev.reins.core.ActivityPush(synced, false, true, false)
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        core.commitChangeGate = gate
        launch()
        await("entry:7")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-other-row")
        tap("entry:7")
        await("entryApproval")
        tap("gitCommits:0")
        tap("gitCommit:0:0")
        await("loadChangesAction")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-other-not-kept")
        tap("loadChangesAction")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-other-loading", idle = false)
        gate.complete(Unit)
        await("retryChanges")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-other-failed")
        core.commitChangeGate = null
        core.commitChanges[7L to full.shortSha] = full
        tap("retryChanges")
        await("commitFile:0")
        tap("commitFile:0")
        await("commitFile:0:diff")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("push-other-loaded")
    }

    /** A Gmail entry: the emails as the approval listed them, what was shared ticked. */
    @Test
    fun polishGmailEntry() {
        core.activity = listOf(
            TestData.entry(
                5, "search", "released", 2u, at = now - 600,
                info = TestData.info("from:stripe.com invoice", listOf(
                    dev.reins.core.ActivityMessage("m1", "", "Stripe", "Your invoice from Acme Inc.", 1_697_000_000),
                    dev.reins.core.ActivityMessage("m2", "", "Stripe", "Payment received", 1_696_800_000),
                ), null, null, null),
                label = TestData.label("gmail", "search", "Search Gmail for", "from:stripe.com invoice", false, ""),
                request = TestData.asked("Search Gmail\nfrom:stripe.com invoice OR receipt newer_than:30d", agent = "", askedAt = now - 612),
            ).copy(connectionLabel = "Claude Code"),
        )
        core.entryApprovals[5L] = TestData.designMailView(null).copy(quick = null)
        launch()
        top()
        tap("entry:5")
        await("entryApproval")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-entry-gmail")
    }

    /** The quick mode menu under the header's pill: as it is, then while a bypass runs. */
    @Test
    fun polishQuickMenu() {
        polishHistory()
        core.model = installed
        launch()
        top()
        tap("modePill")
        await("autopilotQuick")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-quick-menu")
        tap("quick:BYPASS")
        await("bypassSheet")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-quick-bypass-sheet")
        tap("bypassLength:custom")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-quick-bypass-custom")
    }

    @Test
    fun polishQuickMenuDuringBypass() {
        polishHistory()
        core.model = installed
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 8 * 60 + 12, bypassSince = now - 60 * 7)
        launch()
        top()
        tap("modePill")
        await("autopilotQuick")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-quick-menu-bypass")
    }

    /** New grant: the chosen chip tinted with a check, the others outlined. */
    @Test
    fun polishNewGrant() {
        core.connections = listOf(TestData.connection("c1", "Claude Code"), TestData.connection("c2", "ChatGPT"))
        launch()
        tap("tabTrust")
        tap("newGrant")
        await("conn:c1")
        tap("conn:c1")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-new-grant")
    }

    /** Trust during a bypass: the red field with the time left and Stop now; the mode tiles step aside while it runs. */
    @Test
    fun polishAutopilotBypass() {
        core.model = installed
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 8 * 60 + 12, bypassSince = now - 60 * 7)
        launch()
        tap("tabTrust")
        await("bypassClock")
        await("stopBypass")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-autopilot-bypass")
    }

    /** The count on the Activity tab with many new entries, seen from another tab. */
    @Test
    fun polishBadge() {
        core.activity = (1L..120L).map { TestData.entry(it, at = now - 120 + it) }
        launch()
        tap("tabTrust")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        // Opening the app on Activity counted the newest as seen; from Grants, 120 are new.
        (context.applicationContext as ReinsApp).container.state.setSeenActivityId(0)
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("polish-badge")
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
        autopilotPeople()
        openAutopilot()
        await("getScore")
        frames()
        shoot("67-autopilot-no-model")
        core.model = TestData.modelStatus(dev.reins.core.ModelState.DOWNLOADING, downloaded = 156_000_000u, size = 412_000_000u)
        core.downloadFailure = null
        tap("getModel")
        await("modelProgress")
        frames()
        shoot("67b-autopilot-downloading")
    }

    @Test
    fun autopilotModelDetails() {
        core.model = installed
        openAutopilot()
        tap("openModel")
        await("modelSheet")
        frames()
        shoot("67c-autopilot-model-details")
    }

    @Test
    fun autopilotModelFailed() {
        core.model = TestData.modelStatus(dev.reins.core.ModelState.FAILED, error = "sha-256 of model.onnx does not match the pinned hash")
        openAutopilot()
        await("getModelError")
        frames()
        shoot("68-autopilot-model-failed")
    }

    @Test
    fun autopilotProfile() {
        core.model = installed
        autopilotPeople()
        openAutopilot()
        tap("profile:personal")
        await("class:github/write/push")
        frames(30)
        shoot("70-profile")
        core.replay = dev.reins.core.ThresholdReplay(30u, 41u, 12u, 0u)
        core.profiles = core.profiles.map { if (it.id == "personal") it.copy(preset = dev.reins.core.Preset.CUSTOM, approveAt = 0.93f, denyAt = 0.88f) else it }
        tap("preset:CUSTOM")
        await("replayLine")
        frames(30)
        shoot("70b-profile-custom")
        tap("class:gmail/read")
        await("lock:off:gmail/read")
        rule.onNodeWithTag("class:telegram/send").performScrollTo()
        frames()
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
        tap("suggestion")
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
        await("noStanding")
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
        tap("filters")
        tap("filter:who")
        tap("who:autopilot")
        openAllDays()
        // Entry 7 was decided by a bypass: the Autopilot filter leaves it out.
        rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:8"))
        await("entry:8")
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
        tap("tabConnections")
        tap("connection:c1")
        await("connectionMode")
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
            tap("tabSettings")
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
        tap("typeCode")
        await("pairCode")
        shoot("14d-setup-type-code")
        tap("setupNext")
        await("setupRules")
        shoot("14b25-setup-rules")
        tap("setupNext")
        await("setupFinished")
        shoot("14f-setup-done")
    }

    @Test
    fun connectComputer() {
        launch()
        tap("tabConnections")
        tap("connect")
        tap("sheetTypeCode")
        await("scanQr")
        shoot("14f-connect-computer")
    }

    @Test
    fun approvalOneTap() {
        openView(TestData.quick(TestData.searchView()))
        tap("lifetime:HOUR")
        shoot("90-approval-one-tap")
    }

    @Test
    fun approvalRepeated() {
        openView(TestData.quick(TestData.searchView(), repeats = 3u))
        await("lifetime:HOUR")
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
        tap("tabSettings")
        tap("devicesRow")
        await("device:d-old")
        shoot("99-devices")
        tap("signOutDevice:d-old")
        await("signOutDialog")
        shoot("99-devices-sign-out", dialogs = true)
    }

    private fun openVault() {
        core.resetVault()
        core.accounts = listOf(dev.reins.core.AccountView("vault", "me@example.com", now))
        launch()
        tap("tabConnections")
        openService("vault")
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
        // Making a key asks for the screen lock first: here it passes.
        val biometrics = dev.reins.android.platform.AuthenticatorProvider.factory
        dev.reins.android.platform.AuthenticatorProvider.factory = {
            dev.reins.android.platform.Authenticator { _, _ -> dev.reins.android.platform.AuthResult.Success }
        }
        try {
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
        } finally {
            dev.reins.android.platform.AuthenticatorProvider.factory = biometrics
        }
    }

    @Test
    fun payments() {
        history()
        core.payments = TestData.paymentsOverview(enabled = false)
        launch()
        tap("tabConnections")
        openService("payments")
        await("paymentsEnable")
        shoot("100-payments-off")
        core.payments = TestData.paymentsOverview()
        pressBackAndReopen()
        await("method:virtual_card")
        shoot("101-payments")
        rule.onNodeWithTag("addLimit").performScrollTo()
        shoot("102-payments-limits")
        core.spending = TestData.spending()
        tap("openSpending")
        await("purchase:p1")
        shoot("103-spending")
    }

    private fun pressBackAndReopen() {
        scenario!!.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        settle()
        openService("payments")
    }

    @Test
    fun purchaseApproval() {
        history()
        val view = TestData.purchaseView().copy(waitUntil = now + 40)
        core.pending = listOf(TestData.pending("req20", "write", 1u, waitUntil = now + 40, service = "payments", account = "this phone", op = "purchase_request", opTitle = view.opTitle))
        core.approval = view
        launch(link("request", "req20"))
        await("receipt")
        shoot("104-purchase")
        rule.onNodeWithTag("payWith:pay_on_phone").performScrollTo()
        shoot("105-purchase-pay-with")
        tap("makeLimit")
        await("limitEach")
        rule.onNodeWithTag("limitEach").performScrollTo()
        shoot("106-purchase-limit")
    }

    // ---- Activity and Grants redesign (rendered as android-activity-* / android-grants-*) ----------------------

    /** A tap without a touch: the control's pressed look does not linger in the frame. */
    private fun click(tag: String) {
        await(tag)
        try {
            rule.onNodeWithTag(tag).performScrollTo()
        } catch (_: AssertionError) {
        }
        rule.onNodeWithTag(tag).performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.OnClick)
        rule.waitForIdle()
    }

    private fun agConnections() {
        core.connections = listOf(
            TestData.connection("c1", "Claude Code"),
            TestData.connection("c2", "ChatGPT"),
            TestData.connection("c3", "Cursor"),
        )
        core.accounts = listOf(dev.reins.core.AccountView("gmail", "dan@katul.dev", now - 86_400 * 30))
    }

    private fun stripeView(id: String, vararg subjects: String) = TestData.searchView().copy(
        requestId = id, kind = dev.reins.core.ApprovalKind.READ, action = "read", query = null,
        messages = subjects.mapIndexed { i, sub -> dev.reins.core.MessageView("$id-m$i", "Stripe <receipts@stripe.com>", sub, now - 3_600, "", false, false) },
    )

    private fun agHistory() {
        val stripe = listOf(
            dev.reins.core.ActivityMessage("s1", "", "Stripe <receipts@stripe.com>", "Your invoice from Acme Inc.", now - 400),
            dev.reins.core.ActivityMessage("s2", "", "Stripe <receipts@stripe.com>", "Payment received", now - 500),
        )
        fun e(id: Long, action: String, outcome: String, count: UInt, at: Long, conn: String = "c1", label: String = "Claude Code") =
            TestData.entry(id, action, outcome, count, at = at).copy(connectionId = conn, connectionLabel = label, account = "dan@katul.dev")
        core.activity = listOf(
            e(9, "read", "released", 2u, now - 300).copy(info = TestData.info(null, stripe, null, null, null), decidedBy = "autopilot", autopilot = TestData.note(pApprove = 0.96f)),
            e(8, "write", "sent", 3u, now - 1_200).copy(service = "github", account = "octo-cat", op = "git_push", opTitle = "Push to a branch", decidedBy = "autopilot", autopilot = TestData.note()),
            e(7, "read", "released", 1u, now - 3_000, "c2", "ChatGPT").copy(grantId = "g1", info = TestData.info(null, stripe.take(1), null, null, null)),
            e(6, "search", "released", 4u, now - 5_400).copy(info = TestData.info("from:bank newer_than:7d", emptyList(), null, null, null)),
            e(5, "send", "denied", 1u, now - 86_400 - 2_000, "c2", "ChatGPT").copy(info = TestData.info(null, emptyList(), dev.reins.core.EmailView(listOf("billing@acme-pay.co"), emptyList(), "Invoice", "Hi"), null, null)),
            e(4, "write", "sent", 1u, now - 86_400 - 9_000).copy(service = "github", account = "octo-cat", op = "issue_comment", opTitle = "Comment on PR #212", decidedBy = "autopilot", autopilot = TestData.note()),
        )
    }

    private fun agSuggestion(id: String, verdict: dev.reins.core.Verdict, p: Float) = TestData.suggestion(
        id, verdict = verdict, pApprove = if (verdict == dev.reins.core.Verdict.DENY) 0.04f else p, pDeny = if (verdict == dev.reins.core.Verdict.DENY) p else 0.02f,
    )

    @Test
    fun agActivityManual() {
        agConnections()
        agHistory()
        core.model = installed
        val push = TestData.pending("req1", "write", 8u, waitUntil = now + 40, label = "Claude Code", conn = "c1", service = "github", account = "octo-cat", op = "git_push", opTitle = "Push to a branch", quick = true)
            .copy(subtitle = "octo/app", createdAt = now - 80)
        val send = TestData.pending("req2", "send", 1u, waitUntil = now + 80, label = "ChatGPT", conn = "c2", quick = true)
            .copy(title = "ChatGPT wants to send an email to billing@acme-pay.co", subtitle = "Invoice", createdAt = now - 10)
        val read = TestData.pending("req3", "read", 2u, waitUntil = now + 12, label = "Cursor", conn = "c3", quick = true).copy(createdAt = now - 48)
        core.pending = listOf(push, send, read)
        core.views["req3"] = stripeView("req3", "Your invoice from Acme Inc.", "Payment received")
        core.suggestions["req1"] = agSuggestion("req1", dev.reins.core.Verdict.APPROVE, 0.97f)
        core.suggestions["req2"] = agSuggestion("req2", dev.reins.core.Verdict.DENY, 0.91f)
        core.suggestions["req3"] = agSuggestion("req3", dev.reins.core.Verdict.ASK, 0.62f)
        launch()
        top()
        // The stack shows its top card's buttons only; the others wait behind it.
        await("allow:req1")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-manual")
    }

    @Test
    fun agActivityGroup() {
        agConnections()
        agHistory()
        core.model = installed
        core.pending = listOf(
            TestData.pending("req4", "read", 1u, waitUntil = now + 44, label = "Claude Code", conn = "c1", quick = true).copy(createdAt = now - 16),
            TestData.pending("req5", "read", 1u, waitUntil = now + 50, label = "Claude Code", conn = "c1", quick = true).copy(createdAt = now - 10),
            TestData.pending("req6", "search", 3u, waitUntil = now + 30, label = "ChatGPT", conn = "c2", quick = true).copy(subtitle = "from:stripe invoice", createdAt = now - 30),
        )
        core.views["req4"] = stripeView("req4", "Your invoice from Acme Inc.")
        core.views["req5"] = stripeView("req5", "Payment received")
        core.suggestions["req4"] = agSuggestion("req4", dev.reins.core.Verdict.APPROVE, 0.97f)
        core.suggestions["req5"] = agSuggestion("req5", dev.reins.core.Verdict.APPROVE, 0.96f)
        launch()
        top()
        await("approveAll")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-group")
    }

    private fun agAuto() {
        agConnections()
        agHistory()
        core.model = installed
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        val delete = TestData.pending("req7", "write", 1u, waitUntil = now + 55, label = "Claude Code", conn = "c1", service = "github", account = "octo-cat", op = "repo_delete", opTitle = "Delete the repo")
            .copy(subtitle = "octo/old-api", createdAt = now - 5)
        val read = TestData.pending("req8", "read", 2u, waitUntil = now + 44, label = "Claude Code", conn = "c1", quick = true).copy(createdAt = now - 16)
        core.pending = listOf(delete, read)
        core.views["req8"] = stripeView("req8", "Your invoice from Acme Inc.", "Payment received")
        core.suggestions["req7"] = TestData.suggestion("req7", floor = true, judged = false)
        core.suggestions["req8"] = agSuggestion("req8", dev.reins.core.Verdict.ASK, 0.62f)
    }

    @Test
    fun agActivityAuto() {
        agAuto()
        launch()
        top()
        await("alwaysAsks:req7")
        rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:9"))
        await("entry:9")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-auto")
        openAllDays()
        rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:5"))
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-auto-history")
    }

    @Test
    fun agActivityFilters() {
        agAuto()
        core.pending = emptyList()
        launch()
        top()
        await("entry:9")
        click("filters")
        click("filter:who")
        tap("who:autopilot")
        rule.waitForIdle()
        await("entry:4")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-filters")
    }

    @Test
    fun agActivityEntry() {
        agAuto()
        core.pending = emptyList()
        launch()
        top()
        tap("entry:9")
        await("thisWasWrong")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-entry")
        click("whyAutopilot")
        click("emailsShared")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-activity-entry-open")
    }

    private fun agGrants() {
        agConnections()
        agHistory()
        core.startingPolicy = dev.reins.core.StartingPolicy.ASK_EVERY_TIME
        fun g(id: String, conn: String, label: String) = TestData.grant(id).copy(connectionId = conn, connectionLabel = label, account = "dan@katul.dev")
        core.grants = listOf(
            g("g1", "c1", "Claude Code").copy(summary = "Read mail from @stripe.com", expiresAt = now + 240, createdAt = now - 3_360, uses = 2u, lines = listOf("From @stripe.com")),
            g("g2", "c1", "Claude Code").copy(
                summary = "Push to feature/* in octo/app", action = "write", service = "github", account = "octo-cat",
                expiresAt = now + 5 * 3_600 + 600, createdAt = now - 3 * 3_600, maxUses = 10u, uses = 3u,
                lines = listOf("Push to branches matching feature/*"), editableScope = null,
            ),
            g("g3", "c2", "ChatGPT").copy(summary = "See your calendar", service = "gcalendar", expiresAt = now + 6 * 86_400 + 600, createdAt = now - 86_400, uses = 12u, lines = listOf("Events in all calendars")),
            TestData.grant("x1", active = false),
            TestData.grant("x2", active = false, state = "used_up", maxUses = 3u, uses = 3u),
            TestData.grant("x3", active = false, state = "revoked"),
            TestData.grant("x4", active = false).copy(summary = "Send to @corp.example", action = "send"),
        )
        core.activity = core.activity.map { if (it.id == 8L) it.copy(grantId = "g2") else it }
    }

    @Test
    fun agGrantsList() {
        agGrants()
        launch()
        tap("tabTrust")
        await("grant:g3")
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-grants")
        click("help:New AI connections")
        rule.onNodeWithTag("startingRule").performScrollTo()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-grants-scrolled")
        click("expiredHeader")
        await("ended:x1")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-grants-expired")
    }

    @Test
    fun agGrantDetail() {
        agGrants()
        launch()
        tap("tabTrust")
        tap("grant:g2")
        await("extend")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-grants-detail")
        click("extend")
        await("confirmExtend")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-grants-extend")
    }

    @Test
    fun agNewGrant() {
        agGrants()
        launch()
        tap("tabTrust")
        tap("newGrant")
        await("createGrant")
        click("conn:c1")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        rule.onNodeWithTag("parties").performTextReplacement("@stripe.com, receipts@apple.com")
        rule.waitForIdle()
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("android-grants-new")
    }

    // ---- Reins Signal, phase 2: the shell, Activity, Trust, Connections, Settings (sg-*) ----------------------------

    /** The design's push, with two earlier pushes to feature branches of octo/app approved by you. */
    private fun sgPushes() {
        val push = TestData.designPushView().copy(requestId = "req1")
        core.views["req1"] = push
        val earlier = (20L..21L).map { id ->
            TestData.entry(id, "write", "sent", 2u, at = now - 86_400 * 2 - id * 60).copy(connectionId = "c1", connectionLabel = "Claude Code", service = "github", account = "octo-cat", op = "git_push", opTitle = "Push to a branch")
        }
        core.activity = core.activity + earlier
        earlier.forEach { core.entryApprovals[it.id] = push }
        core.profiles = listOf(TestData.profile("personal", "Default", memory = 140u, classes = listOf(TestData.classView(decisions = 12u, approved = 11u, denied = 1u))))
    }

    private fun sgWaiting() {
        agConnections()
        agHistory()
        core.model = installed
        val push = TestData.pending("req1", "write", 8u, waitUntil = now + 72, label = "Claude Code", conn = "c1", service = "github", account = "octo-cat", op = "git_push", opTitle = "Push to a branch", quick = true)
            .copy(subtitle = "octo/app", createdAt = now - 48)
        val send = TestData.pending("req2", "send", 1u, waitUntil = now + 80, label = "ChatGPT", conn = "c2", quick = true)
            .copy(title = "ChatGPT wants to send an email to billing@acme-pay.co", subtitle = "Invoice", createdAt = now - 10)
        val read = TestData.pending("req3", "read", 2u, waitUntil = now + 12, label = "Cursor", conn = "c3", quick = true).copy(createdAt = now - 48)
        core.pending = listOf(push, send, read)
        core.views["req3"] = stripeView("req3", "Your invoice from Acme Inc.", "Payment received")
        core.suggestions["req1"] = agSuggestion("req1", dev.reins.core.Verdict.APPROVE, 0.97f).copy(profileId = "personal", classKey = "github/write/push")
        core.suggestions["req2"] = agSuggestion("req2", dev.reins.core.Verdict.DENY, 0.91f)
        core.suggestions["req3"] = agSuggestion("req3", dev.reins.core.Verdict.ASK, 0.62f)
        sgPushes()
    }

    @Test
    fun sgActivity() {
        sgWaiting()
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        launch()
        top()
        await("allow:req1")
        frames()
        shoot("sg-01-activity-stack")
        // Mid-flick: the top card follows the thumb and tilts.
        rule.onNodeWithTag("pending:req1").performTouchInput {
            down(center)
            moveBy(androidx.compose.ui.geometry.Offset(width * 0.28f, 6f))
        }
        frames(4)
        shoot("sg-02-activity-flick")
        rule.onNodeWithTag("pending:req1").performTouchInput { up() }
        frames(20)
    }

    @Test
    fun sgActivityQuiet() {
        agConnections()
        agHistory()
        core.model = installed
        shadowOf(context as android.app.Application).denyPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        launch()
        top()
        await("notificationsOff")
        frames()
        shoot("sg-03-activity-quiet-warn")
    }

    /** #101's sync notice on Activity, the Signal way: a soft row with the sync glyph and OK. */
    @Test
    fun sgActivitySyncNotice() {
        agConnections()
        agHistory()
        core.model = installed
        core.syncNotice = "Synced with your other phone. A backup of this phone's changes is kept."
        launch()
        top()
        await("syncNotice")
        frames()
        shoot("sg-08-activity-sync-notice")
    }

    // ---- How a request is written and marked (design/signal/ASK-TEXT.md) ----------------------------------------------

    private val askCommand = "rm -rf jarx && npm run build && npm test"

    private fun askTextView() = TestData.askView(
        detail = "Command:\n$askCommand\n\nIn: /home/dana/dev/reins\nRule: rm -rf*",
        topic = "command:rm -rf",
        rule = dev.reins.core.AskRuleView("Deletes files", "rm -rf*", "rm -rf jarx", null, true, false),
    ).let { v -> v.copy(requestId = "req1", connectionId = "c1", connectionLabel = "Claude Code", ask = v.ask!!.copy(question = "Claude Code wants to run: $askCommand", destructive = true)) }

    /** Waiting and decided requests of every kind, each with the core's label. */
    private fun askTextData() {
        agConnections()
        core.model = installed
        val run = TestData.pending("req1", "write", 1u, waitUntil = now + 60, label = "Claude Code", conn = "c1", service = "desktop", account = null, op = "ask")
            .copy(createdAt = now - 20, label = TestData.label("terminal", "delete", "Run", askCommand, code = true))
        val push = TestData.pending("req2", "write", 3u, waitUntil = now + 72, label = "Claude Code", conn = "c1", service = "github", account = "octo-cat", op = "git_push", quick = true)
            .copy(createdAt = now - 40, label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins"))
        val read = TestData.pending("req3", "read", 2u, waitUntil = now + 80, label = "Cursor", conn = "c3", quick = true)
            .copy(createdAt = now - 48, label = TestData.label("gmail", "read", "Read 2 emails from", "Stripe"))
        core.pending = listOf(run, push, read)
        core.views["req1"] = askTextView()
        core.views["req2"] = TestData.designPushView().copy(requestId = "req2")
        core.views["req3"] = stripeView("req3", "Your invoice from Acme Inc.", "Payment received")
        core.suggestions["req2"] = agSuggestion("req2", dev.reins.core.Verdict.APPROVE, 0.97f)
        core.suggestions["req3"] = agSuggestion("req3", dev.reins.core.Verdict.ASK, 0.62f)
        fun e(id: Long, label: dev.reins.core.RequestLabel, service: String, minutes: Long, outcome: String = "released", conn: String = "c1", who: String = "Claude Code") =
            TestData.entry(id, "write", outcome, 1u, at = now - minutes * 60, label = label, service = service).copy(connectionId = conn, connectionLabel = who)
        val ran = "cargo test -p reins-core && git push origin signal/desktop-release"
        core.activity = listOf(
            e(40, TestData.label("terminal", "run", "Run", ran, code = true), "desktop", 2).copy(op = "ask", request = TestData.asked(ran, agent = "Claude Code", cwd = "/home/dana/dev/reins")),
            e(41, TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins"), "github", 9).copy(op = "git_push", decidedBy = "autopilot", autopilot = TestData.note()),
            e(42, TestData.label("file", "read", "Read", "~/dev/api/.env", code = true, tail = " and 1 more"), "desktop", 14, conn = "c3", who = "Cursor"),
            e(43, TestData.label("gmail", "send", "Send an email to", "Dana Katul <dana@katul.dev>", tail = " and 1 more"), "gmail", 30, outcome = "denied", conn = "c2", who = "ChatGPT"),
            e(44, TestData.label("ssh", "secrets", "Sign in to", "build.example.com", code = true), "vault", 50),
            e(45, TestData.label("secrets", "secrets", "Use 2 secrets for", "npm run deploy", code = true), "vault", 70),
            e(46, TestData.label("mcp:linear", "write", "Use", "Create issue", tail = " on Linear"), "mcp:linear", 95, conn = "c2", who = "ChatGPT"),
            e(47, TestData.label("github", "delete", "Delete branch old/experiment from", "octo/app"), "github", 130).copy(op = "git_push"),
            e(48, TestData.label("gmail", "search", "Search Gmail for", "invoice"), "gmail", 180, conn = "c3", who = "Cursor"),
        )
        core.entryApprovals[40] = askTextView().copy(requestId = "e40", ask = askTextView().ask!!.copy(question = "Claude Code wants to run: $ran", detail = "Command:\n$ran\n\nIn: /home/dana/dev/reins\nRule: git push*", rule = null, destructive = false))
    }

    @Test
    fun sgAskText() {
        askTextData()
        launch()
        top()
        await("pending:req1")
        frames()
        shoot("sg-20-asktext-stack")
        openAllDays()
        rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:44"))
        frames()
        shoot("sg-21-asktext-river")
    }

    /** Crafted requests: look-alike names, a climbing path, a store that is not ASCII. None reaches the serif. */
    @Test
    fun sgAskTextSpoofs() {
        agConnections()
        core.model = installed
        core.pending = emptyList()
        fun e(id: Long, label: dev.reins.core.RequestLabel, service: String, minutes: Long) =
            TestData.entry(id, "write", "released", 1u, at = now - minutes * 60, label = label, service = service).copy(connectionId = "c1", connectionLabel = "Claude Code")
        core.activity = listOf(
            // People's names in one script are shown as they are; a name that mixes scripts is not.
            e(56, TestData.label("gmail", "read", "Read 2 emails from", "Дмитрий"), "gmail", 1),
            e(57, TestData.label("telegram", "read", "Read messages in", "Αλέξανδρος"), "telegram", 1),
            e(58, TestData.label("gmail", "read", "Read 1 email from", "Pаvel"), "gmail", 1),
            e(50, TestData.label("github", "push", "Push 2 commits to", "katulevskiy/reаins"), "github", 2).copy(op = "git_push"),
            e(51, TestData.label("file", "read", "Read", "/tmp/../home/dana/.ssh/id_ed25519", code = true), "desktop", 5),
            e(52, TestData.label("file", "read", "Read", "/home/dana/pаypal-invoice.pdf", code = true), "desktop", 9),
            e(53, TestData.label("ssh", "secrets", "Sign in to", "gоogle.com", code = true), "vault", 14),
            e(54, TestData.label("gmail", "send", "Send an email to", "Your bank <help@bаnk.com>"), "gmail", 20),
            e(55, TestData.label("desktop", "ask", "Approve the pаypal payout?"), "desktop", 30).copy(op = "ask"),
        )
        launch()
        top()
        await("entry:56")
        frames()
        shoot("sg-25-asktext-spoofs")
    }

    @Test
    fun sgAskTextField() {
        askTextData()
        core.approval = askTextView().copy(waitUntil = now + 60)
        launch(link("request", "req1"))
        await("approve")
        frames()
        shoot("sg-22-asktext-field")
    }

    @Test
    fun sgAskTextEntry() {
        askTextData()
        launch()
        top()
        await("activityList")
        openAllDays()
        rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:40"))
        rule.onNodeWithTag("entry:40").performTouchInput { down(center); up() }
        rule.waitUntil(10_000) { settle(); listOf("entryBar", "detailTitle").any { rule.onAllNodes(hasTestTag(it), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() } }
        frames()
        shoot("sg-23-asktext-entry")
        pressBack()
        await("activityList")
        rule.onNodeWithTag("activityList").performScrollToNode(hasTestTag("entry:41"))
        rule.onNodeWithTag("entry:41").performTouchInput { down(center); up() }
        await("detailTitle")
        frames()
        shoot("sg-24-asktext-entry-kept")
    }

    @Test
    fun sgActivityMenuAndFilters() {
        sgWaiting()
        core.pending = emptyList()
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        launch()
        top()
        tap("modePill")
        await("autopilotQuick")
        frames()
        shoot("sg-04-activity-mode-menu", dialogs = true)
        pressBack()
        frames()
        click("filters")
        click("filter:who")
        tap("who:autopilot")
        rule.waitForIdle()
        await("entry:4")
        frames()
        shoot("sg-05-activity-filtered")
    }

    @Test
    fun sgActivityFold() {
        sgWaiting()
        core.pending = core.pending.take(1)
        launch()
        top()
        await("allow:req1")
        frames()
        // Answered elsewhere (its sheet): the card folds away and lands at the top of the river.
        core.pending = emptyList()
        core.activity = listOf(TestData.entry(30, "write", "sent", 8u, at = now - 5).copy(connectionId = "c1", connectionLabel = "Claude Code", service = "github", account = "octo-cat", op = "git_push", opTitle = "Push to a branch")) + core.activity
        rule.mainClock.autoAdvance = false
        val container = (context.applicationContext as ReinsApp).container
        // Off the main thread: a refresh may need it, and the clock is stopped here.
        Thread { kotlinx.coroutines.runBlocking { container.refreshPending() } }.start()
        rule.waitUntil(10_000) {
            settle()
            container.state.pending.value.isEmpty() && container.state.activity.value.firstOrNull()?.id == 30L
        }
        frames(3)
        shoot("sg-06-activity-folding", idle = false)
        rule.mainClock.autoAdvance = true
        frames(10)
        shoot("sg-07-activity-landed")
    }

    @Test
    fun sgTrust() {
        agGrants()
        autopilotPeople()
        core.model = installed
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        launch()
        tap("tabTrust")
        await("grant:g3")
        frames()
        shoot("sg-10-trust")
        rule.onNodeWithTag("startingRule").performScrollTo()
        frames()
        shoot("sg-11-trust-scrolled")
        click("expiredHeader")
        await("ended:x1")
        frames()
        shoot("sg-12-trust-ended")
    }

    @Test
    fun sgTrustRunning() {
        agGrants()
        core.model = installed
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 8 * 60 + 12, bypassSince = now - 6 * 60 - 48)
        launch()
        tap("tabTrust")
        await("bypassClock")
        frames()
        shoot("sg-13-trust-bypass")
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.OVERNIGHT, overnightUntil = now + 7 * 3_600, overnightSince = now - 3_600, overnightReturn = dev.reins.core.AutopilotMode.AUTO)
        kotlinx.coroutines.runBlocking { (context.applicationContext as ReinsApp).container.refreshAutopilot() }
        await("overnightClock")
        frames()
        shoot("sg-14-trust-overnight")
        core.apGlobal = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.LOCKDOWN)
        kotlinx.coroutines.runBlocking { (context.applicationContext as ReinsApp).container.refreshAutopilot() }
        await("endLockdown")
        frames()
        shoot("sg-15-trust-lockdown")
    }

    @Test
    fun sgConnections() {
        agConnections()
        core.connections = core.connections + TestData.connection("c9", "ThinkPad", keyFingerprint = "SHA256:abc")
        core.accounts = listOf(
            dev.reins.core.AccountView("gmail", "dan@katul.dev", now - 86_400 * 30),
            dev.reins.core.AccountView("gmail", "dana.kim@gmail.com", now - 86_400 * 3),
            dev.reins.core.AccountView("github", "octo-dana", now - 86_400 * 9),
            dev.reins.core.AccountView("sms", "this phone", now - 86_400 * 9),
        )
        mcpServers()
        core.apConnections["c2"] = FakeCore.ApRow(mode = dev.reins.core.AutopilotMode.AUTO)
        launch()
        tap("tabConnections")
        await("service:gmail")
        frames()
        shoot("sg-20-connections")
        tap("connect")
        await("connectSheet")
        frames(20)
        shoot("sg-21-connections-connect")
    }

    @Test
    fun sgSettings() {
        dev.reins.android.ui.settings.appPushOverride = true
        launch()
        tap("tabSettings")
        await("accountCard")
        frames()
        shoot("sg-30-settings")
        rule.onNodeWithTag("pageEnd").performScrollTo()
        frames()
        shoot("sg-31-settings-below")
        rule.onNodeWithTag("openAppearance").performScrollTo()
        tap("openAppearance")
        await("appearance")
        frames()
        shoot("sg-32-settings-appearance")
    }

    /** A new account: the passkey offer, the recovery code, then Settings › Passkeys with the one it has. */
    @Test
    fun nextPasskeyOfferAndPasskeys() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.recoveryCode = FakeCore.RECOVERY_CODE
        core.vaultPasskeys = emptyList()
        launch()
        await("passkeyOffer")
        frames()
        shoot("next-passkey-offer")
        tap("skipPasskey")
        await("recoveryRecorded")
        frames(30)
        shoot("next-recovery-code", dialogs = true)
        tap("recoveryRecorded")
        tap("recoveryCodeDone")
        frames()
        core.vaultPasskeys = listOf(dev.reins.core.VaultPasskeyView(FakeCore.EXISTING_PASSKEY_ID, "Pixel 9", now - 86_400 * 30))
        tap("tabSettings")
        rule.onNodeWithTag("vaultPasskeysRow").performScrollTo()
        tap("vaultPasskeysRow")
        frames(30)
        shoot("next-settings-passkeys")
    }

    /** Settings 04-06: Appearance, Passkeys and Delete account, on the kit. */
    @Test
    fun nextSettingsPages() {
        dev.reins.android.ui.settings.appPushOverride = true
        launch()
        tap("tabSettings")
        await("accountCard")
        rule.onNodeWithTag("openAppearance").performScrollTo()
        tap("openAppearance")
        await("appearance")
        frames()
        shoot("next-settings-appearance")
        pressBack()
        frames()
        if (rule.onAllNodes(hasTestTag("vaultPasskeysRow")).fetchSemanticsNodes().isNotEmpty()) {
            rule.onNodeWithTag("vaultPasskeysRow").performScrollTo()
            tap("vaultPasskeysRow")
            frames(30)
            shoot("next-settings-passkeys")
            pressBack()
            frames()
        }
        rule.onNodeWithTag("deleteAccount").performScrollTo()
        tap("deleteAccount")
        await("deleteAccountSheet")
        rule.onNodeWithTag("deleteAccountEmail").performTextReplacement("me@example.com")
        frames(30)
        shoot("next-settings-delete-account", dialogs = true)
    }

    @Test
    fun sgSettingsUpdate() {
        dev.reins.android.ui.settings.appPushOverride = true
        assumeTrue(BuildConfig.SELF_UPDATE)
        updates.publish(29_834_567, versionName = "1.5")
        launch()
        tap("tabSettings")
        await("updatePrompt")
        frames()
        shoot("sg-33-settings-update")
    }

    // ---- Activity by day, and its search (rendered as sg-40.. / sg-42..) ------------------------------------------

    /** Runs [block] with the phone in UTC, so the frozen clock (22:15) puts the days where the shots expect them. */
    private fun inUtc(block: () -> Unit) {
        val zone = java.util.TimeZone.getDefault()
        java.util.TimeZone.setDefault(java.util.TimeZone.getTimeZone("UTC"))
        try {
            block()
        } finally {
            java.util.TimeZone.setDefault(zone)
        }
    }

    private val today get() = dev.reins.android.ui.activity.localDay(now, java.time.ZoneOffset.UTC)

    /** Five days of Activity: today's, yesterday's with a denial, and older days with a failure. */
    private fun dayHistory() {
        agConnections()
        core.model = installed
        core.pending = emptyList()
        fun e(id: Long, label: dev.reins.core.RequestLabel, service: String, at: Long, outcome: String = "released", conn: String = "c1", who: String = "Claude Code") =
            TestData.entry(id, label.action, outcome, 1u, at = at, label = label, service = service).copy(connectionId = conn, connectionLabel = who)
        fun mail(subject: String, body: String) = TestData.info(null, emptyList(), dev.reins.core.EmailView(listOf("Dana Katul <dana@katul.dev>"), emptyList(), subject, body), null, null)
        val day = 86_400L
        core.activity = listOf(
            e(70, TestData.label("github", "push", "Push 3 commits to", "acme/web"), "github", now - 4 * 60).copy(op = "git_push", decidedBy = "autopilot", autopilot = TestData.note()),
            e(69, TestData.label("gmail", "read", "Read 2 emails from", "Stripe"), "gmail", now - 18 * 60, conn = "c2", who = "ChatGPT"),
            e(68, TestData.label("gmail", "search", "Search Gmail for", "invoice"), "gmail", now - 32 * 60).copy(grantId = "g1"),
            e(67, TestData.label("gmail", "send", "Send an email to", "Dana Katul <dana@katul.dev>"), "gmail", now - 2 * 3_600, conn = "c2", who = "ChatGPT")
                .copy(info = mail("Q3 numbers", "Hi Dana, the quarterly invoice for Acme is attached. Let me know if the totals look right.")),
            e(66, TestData.label("github", "delete", "Delete branch old/experiment from", "octo/app"), "github", now - 5 * 3_600, outcome = "denied", conn = "c3", who = "Cursor").copy(op = "git_push"),
            e(60, TestData.label("gmail", "read", "Read 4 emails from", "GitHub"), "gmail", now - day - 3_600, conn = "c2", who = "ChatGPT"),
            e(59, TestData.label("gmail", "send", "Send an email to", "billing@acme-pay.co"), "gmail", now - day - 2 * 3_600, outcome = "denied", conn = "c2", who = "ChatGPT")
                .copy(info = mail("Invoice 2207", "Please pay invoice 2207 today to avoid a late fee.")),
            e(58, TestData.label("file", "read", "Read", "~/dev/api/.env", code = true), "desktop", now - day - 4 * 3_600, conn = "c3", who = "Cursor"),
            e(57, TestData.label("github", "push", "Push 12 commits to", "acme/web"), "github", now - day - 6 * 3_600).copy(op = "git_push", decidedBy = "autopilot", autopilot = TestData.note()),
            e(50, TestData.label("gmail", "search", "Search Gmail for", "invoice march"), "gmail", now - 2 * day - 3_600),
            e(49, TestData.label("github", "push", "Push 2 commits to", "octo/app"), "github", now - 2 * day - 5 * 3_600, outcome = "error").copy(op = "git_push"),
            e(48, TestData.label("gmail", "read", "Read 1 email from", "Linear"), "gmail", now - 2 * day - 8 * 3_600, conn = "c2", who = "ChatGPT"),
            e(40, TestData.label("gmail", "send", "Send an email to", "Jonas"), "gmail", now - 5 * day, conn = "c2", who = "ChatGPT")
                .copy(info = mail("Re: invoice", "Thanks Jonas, I forwarded the invoice to accounting.")),
            e(39, TestData.label("file", "read", "Read", "~/dev/invoices/2026-q3.pdf", code = true), "desktop", now - 5 * day - 3_600, conn = "c3", who = "Cursor"),
        )
    }

    @Test
    fun sgActivityDays() = inUtc {
        dayHistory()
        launch()
        top()
        await("day:$today")
        await("entry:70")
        frames()
        shoot("sg-40-activity-days")
        // Yesterday opened beside the closed days.
        click("day:$today")
        click("day:${today - 1}")
        await("entry:59")
        frames()
        shoot("sg-41-activity-day-open")
    }

    @Test
    fun sgActivitySearch() = inUtc {
        dayHistory()
        launch()
        top()
        await("openSearch")
        // The list at rest first: what it shows counts as seen.
        frames()
        tap("openSearch")
        await("activitySearch")
        rule.onNodeWithTag("activitySearch").performTextInput("invoice")
        await("searchResults")
        tap("filters")
        tap("filter:kind")
        tap("service:gmail")
        await("filtersClear")
        tap("filter:when")
        tap("when:week")
        await("entry:40")
        rule.onNodeWithTag("activityList").performScrollToIndex(0)
        rule.onNodeWithTag("filterRow").performTouchInput { swipeRight() }
        frames()
        shoot("sg-42-activity-search")
    }

    /** Computers with their platform's logo, AIs with their provider's, a bot with its blobatar; then the Icon sheet. */
    @Test
    fun connectionAvatars() {
        history()
        core.connections = TestData.showcaseConnections()
        launch()
        tap("tabConnections")
        await("connection:c3")
        shoot("av-1-connections")
        tap("connection:d2")
        await("disconnect")
        shoot("av-2-computer")
        tap("connectionIcon")
        await("blobatarEditor")
        repeat(20) { rule.mainClock.advanceTimeBy(50) }
        shoot("av-3-icon-sheet")
        tap("shape:5")
        rule.waitUntil(5_000) { settle(); core.connections.single { it.id == "d2" }.icon == "blob" }
        repeat(10) { rule.mainClock.advanceTimeBy(50) }
        shoot("av-5-icon-sheet-picked")
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

/**
 * The Signal gallery (Settings, Developer, debug builds), one PNG per page: `signal-<page>-<theme>.png` in the
 * `reins.screenshots` directory. Drawn on a tall window so each page fits whole.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
abstract class SignalGalleryShots(private val suffix: String) {
    @get:Rule val rule = androidx.compose.ui.test.junit4.createAndroidComposeRule<androidx.activity.ComponentActivity>()

    @Before
    fun setUp() {
        assumeTrue(System.getProperty("reins.screenshots") != null)
        Timers.live = false
    }

    @After
    fun tearDown() {
        Timers.live = true
    }

    @Test
    fun gallery() {
        val dir = File(System.getProperty("reins.screenshots")!!).apply { mkdirs() }
        val page = androidx.compose.runtime.mutableStateOf(dev.reins.android.ui.signal.GalleryPage.Type)
        rule.setContent {
            dev.reins.android.design.ReinsTheme {
                androidx.compose.runtime.key(page.value) { dev.reins.android.ui.signal.SignalGallery(onBack = {}, initialPage = page.value) }
            }
        }
        for (p in dev.reins.android.ui.signal.GalleryPage.entries) {
            rule.runOnIdle { page.value = p }
            rule.waitForIdle()
            rule.mainClock.advanceTimeBy(1_000)
            rule.waitForIdle()
            val view = rule.activity.window.decorView
            val bitmap = android.graphics.Bitmap.createBitmap(view.width, view.height, android.graphics.Bitmap.Config.ARGB_8888)
            view.draw(android.graphics.Canvas(bitmap))
            File(dir, "signal-${p.name.lowercase()}-$suffix.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
        }
    }
}

@Config(sdk = [35], qualifiers = "w411dp-h2600dp-normal-notnight-xxhdpi")
class SignalGalleryLight : SignalGalleryShots("light")

@Config(sdk = [35], qualifiers = "w411dp-h2600dp-normal-night-xxhdpi")
class SignalGalleryDark : SignalGalleryShots("dark")
