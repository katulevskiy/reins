package dev.reins.android

import android.content.Context
import android.content.Intent
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import dev.reins.android.core.CoreFactory
import dev.reins.android.core.CoreProvider
import dev.reins.android.design.Timers
import dev.reins.android.platform.AppNotifier
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.AuthenticatorProvider
import dev.reins.android.platform.Foreground
import dev.reins.android.platform.update.FakeInstaller
import dev.reins.android.platform.update.FakeUpdateServer
import dev.reins.android.platform.update.UpdateProvider
import dev.reins.android.state.SessionState
import dev.reins.core.AccountView
import dev.reins.core.GmailStatus
import dev.reins.core.SessionInfo
import org.junit.After
import org.junit.Before
import org.junit.BeforeClass
import org.junit.Rule
import org.robolectric.Shadows.shadowOf

/** What the flow tests of the newer screens share: a signed-in app over [FakeCore], frozen clocks, and helpers. */
abstract class FlowHarness {
    @get:Rule val rule = createEmptyComposeRule()

    protected val core = FakeCore.shared
    protected val context: Context get() = ApplicationProvider.getApplicationContext()
    protected val container: AppContainer get() = (context.applicationContext as ReinsApp).container
    protected var scenario: ActivityScenario<MainActivity>? = null

    @Volatile protected var authResult: AuthResult = AuthResult.Success
    protected val prompts = java.util.concurrent.atomic.AtomicInteger()

    @Before
    fun resetCore() {
        dev.reins.android.TestNativeKeys.install()
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        Timers.live = false
        Timers.frozenNowMillis = 1_700_000_100_000
        Foreground.focused = false
        Foreground.autoPopup = true
        core.session = SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.loginError = null
        core.pending = emptyList()
        core.approval = null
        core.pairing = null
        core.grants = emptyList()
        core.connections = listOf(TestData.connection())
        core.activity = emptyList()
        core.gmail = GmailStatus.Ready
        core.accounts = listOf(AccountView("gmail", "me@gmail.com", 1_700_000_000))
        core.catalogue = FakeCore.defaultServices()
        core.accountStatuses = emptyMap()
        core.serviceStatuses = emptyMap()
        core.serviceFailure = null
        core.tokensAdded.clear()
        core.approvals.clear()
        core.denials.clear()
        core.blobs.clear()
        core.blobAnswers.clear()
        core.mcp = emptyList()
        core.mcpNextAdd = null
        core.mcpNextRefresh = null
        core.mcpFailure = null
        core.mcpAdds.clear()
        core.mcpTokenAdds.clear()
        core.mcpSignIns.clear()
        core.mcpRefreshes.clear()
        core.mcpRemoved.clear()
        core.mcpHeavy.clear()
        core.resetAutopilot()
        core.resetSso()
        container.ssoSignIn.clear()
        authResult = AuthResult.Success
        prompts.set(0)
        UpdateProvider.fetcher = FakeUpdateServer()
        UpdateProvider.installer = FakeInstaller()
        androidx.work.testing.WorkManagerTestInitHelper.initializeTestWorkManager(context)
        AuthenticatorProvider.factory = {
            Authenticator { _, _ ->
                prompts.incrementAndGet()
                authResult
            }
        }
        container.state.setSession(SessionState.Loading)
        container.state.setRegistrationError(null)
    }

    @After
    fun closeApp() {
        scenario?.close()
        Foreground.focused = false
        Timers.live = true
        Timers.frozenNowMillis = null
    }

    protected fun launch(intent: Intent? = null) {
        scenario = ActivityScenario.launch(intent ?: Intent(context, MainActivity::class.java))
    }

    protected fun relaunch(intent: Intent) {
        scenario?.close()
        launch(intent)
    }

    protected fun settle() {
        androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications()
        shadowOf(android.os.Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    protected fun has(tag: String): Boolean {
        settle()
        return rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
    }

    protected fun awaitTag(tag: String) {
        rule.waitUntil(10_000) { has(tag) }
    }

    protected fun awaitGone(tag: String) {
        rule.waitUntil(10_000) { !has(tag) }
    }

    protected fun showsText(text: String, substring: Boolean = false): Boolean {
        settle()
        return rule.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()
    }

    protected fun awaitText(text: String, substring: Boolean = false) {
        rule.waitUntil(10_000) { showsText(text, substring) }
    }

    protected fun tap(tag: String) {
        awaitTag(tag)
        val node = rule.onNodeWithTag(tag)
        try {
            node.performScrollTo()
        } catch (_: AssertionError) {
        }
        node.performClick()
    }

    protected fun awaitCore(condition: () -> Boolean) {
        rule.waitUntil(10_000) {
            settle()
            condition()
        }
    }

    /** The activity the app last asked Android to start (a browser page, a Custom Tab), if any. */
    protected fun nextStarted(): Intent? = shadowOf(context as android.app.Application).nextStartedActivity

    protected fun link(kind: String, id: String) = Intent(context, MainActivity::class.java)
        .setAction(AppNotifier.ACTION_OPEN_ITEM)
        .putExtra(AppNotifier.EXTRA_KIND, kind)
        .putExtra(AppNotifier.EXTRA_ID, id)

    /** Opens the approval sheet of a waiting request (it may already have popped up by itself). */
    protected fun openRequest(view: dev.reins.core.ApprovalView) {
        core.pending = listOf(
            TestData.pending(
                view.requestId, view.action, 1u, label = view.connectionLabel, service = view.service, account = view.account,
                op = view.op, opTitle = view.opTitle,
            ),
        )
        core.approval = view
        launch()
        awaitTag("pending:${view.requestId}")
        if (!has("sheet")) tap("pending:${view.requestId}")
        awaitTag("approve")
    }

    companion object {
        /** Read when the Application (and its container) is created, so it must be set before any test starts. */
        @JvmStatic
        @BeforeClass
        fun installFakeCore() {
            CoreProvider.factory = CoreFactory { FakeCore.shared }
        }
    }
}
