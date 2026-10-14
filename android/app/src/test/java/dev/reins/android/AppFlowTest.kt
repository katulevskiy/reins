package dev.reins.android

import android.content.Context
import android.content.Intent
import android.view.WindowManager
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextReplacement
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
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
import dev.reins.core.ActivityInfo
import dev.reins.core.ApprovalKind
import dev.reins.core.CoreException
import dev.reins.core.EmailView
import dev.reins.core.GmailStatus
import dev.reins.core.PairingView
import dev.reins.core.SessionInfo
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.BeforeClass
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class AppFlowTest {
    @get:Rule val rule = createEmptyComposeRule()

    private val core = FakeCore.shared
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private var scenario: ActivityScenario<MainActivity>? = null

    @Volatile private var authResult: AuthResult = AuthResult.Success
    private val prompts = java.util.concurrent.atomic.AtomicInteger()

    private val signedIn = SessionInfo("http://127.0.0.1:8000", "me@example.com")

    @Before
    fun setUp() {
        dev.reins.android.TestNativeKeys.install()
        shadowOf(context as android.app.Application).grantPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        // Robolectric has no screen lock; the tests that need none set it.
        dev.reins.android.platform.ScreenLock.check = { true }
        Timers.live = false
        Timers.frozenNowMillis = 1_700_000_100_000
        Foreground.focused = false
        Foreground.autoPopup = true
        core.session = signedIn
        core.loginError = null
        core.pending = emptyList()
        core.approval = null
        core.pairing = null
        core.grants = emptyList()
        core.connections = listOf(TestData.connection())
        core.activity = emptyList()
        core.gmail = GmailStatus.Ready
        core.resetAutopilot()
        core.resetSso()
        core.accounts = listOf(AccountView("gmail", "me@gmail.com", 1_700_000_000))
        core.accountStatuses = emptyMap()
        core.serviceAdded.clear()
        core.tokensAdded.clear()
        core.serviceRemoved.clear()
        core.loginCalls.clear()
        core.serviceFailure = null
        core.serviceStatuses = emptyMap()
        core.createdGrantAccounts.clear()
        core.resumed.clear()
        core.addedAccounts.clear()
        core.removedAccounts.clear()
        core.deletedGrants.clear()
        core.resumedEdited.clear()
        core.emails.clear()
        core.openedEmails.clear()
        core.approvals.clear()
        core.denials.clear()
        core.pairingAnswers.clear()
        core.revokedGrants.clear()
        core.revokedConnections.clear()
        core.createdGrants.clear()
        core.icons.clear()
        core.logins.clear()
        core.registrations.clear()
        core.resetOnboarding()
        authResult = AuthResult.Success
        prompts.set(0)
        // The updater (in the `full` build) never reaches a real server or installer; its tests are UpdatesFlowTest.
        UpdateProvider.fetcher = FakeUpdateServer()
        UpdateProvider.installer = FakeInstaller()
        androidx.work.testing.WorkManagerTestInitHelper.initializeTestWorkManager(context)
        AuthenticatorProvider.factory = {
            Authenticator { _, _ ->
                prompts.incrementAndGet()
                authResult
            }
        }
        val container = (context.applicationContext as ReinsApp).container
        container.state.setSession(SessionState.Loading)
        container.state.setRegistrationError(null)
    }

    @After
    fun tearDown() {
        scenario?.close()
        Foreground.focused = false
        Timers.live = true
        Timers.frozenNowMillis = null
    }

    // ---- helpers -----------------------------------------------------------------------------------------------

    private fun launch(intent: Intent? = null) {
        scenario = ActivityScenario.launch(intent ?: Intent(context, MainActivity::class.java))
    }

    /** Holds the dangerous Approve until it fills. */
    private fun holdApprove() {
        awaitTag("approve")
        rule.onNodeWithTag("approve").performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(2_300)
        // Once it fills the screen closes, so the finger may have nothing left to lift from.
        runCatching { rule.onNodeWithTag("approve").performTouchInput { up() } }
    }

    private fun pressBack() {
        scenario!!.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        settle()
    }

    private fun settle() {
        androidx.compose.runtime.snapshots.Snapshot.sendApplyNotifications()
        shadowOf(android.os.Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    private fun has(tag: String): Boolean {
        settle()
        return rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
    }

    private fun awaitTag(tag: String) {
        rule.waitUntil(10_000) { has(tag) }
    }

    private fun awaitGone(tag: String) {
        rule.waitUntil(10_000) { !has(tag) }
    }

    private fun awaitText(text: String) {
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun awaitTextContaining(text: String) {
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
    }

    /** Holds the hold-to-confirm button tagged [tag] down until it confirms, then lets go. */
    private fun hold(tag: String) {
        awaitTag(tag)
        val node = rule.onNodeWithTag(tag)
        try {
            node.performScrollTo()
        } catch (_: AssertionError) {
        }
        node.performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(dev.reins.android.design.HOLD_MILLIS + 300L)
        // What it confirmed may have closed it before the finger lifts.
        runCatching { rule.onNodeWithTag(tag).performTouchInput { up() } }
        rule.waitForIdle()
    }

    private fun tap(tag: String) {
        awaitTag(tag)
        val node = rule.onNodeWithTag(tag)
        // Floating controls (nav bar, sheet buttons) are not inside a scroll container.
        try {
            node.performScrollTo()
        } catch (_: AssertionError) {
        }
        node.performClick()
    }

    private fun awaitCore(condition: () -> Boolean) {
        rule.waitUntil(10_000) {
            settle()
            condition()
        }
    }

    private fun secure(): Boolean {
        var flag = false
        scenario!!.onActivity { flag = it.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0 }
        return flag
    }

    /** Opens the approval sheet of a waiting item (it may already have popped up by itself). */
    private fun openItem(id: String) {
        awaitTag("pending:$id")
        if (!has("sheet")) tap("pending:$id")
        awaitTag("approve")
    }

    private fun link(kind: String, id: String) = Intent(context, MainActivity::class.java)
        .setAction(AppNotifier.ACTION_OPEN_ITEM)
        .putExtra(AppNotifier.EXTRA_KIND, kind)
        .putExtra(AppNotifier.EXTRA_ID, id)

    private fun searchRequest() {
        core.pending = listOf(TestData.pending())
        core.approval = TestData.searchView()
    }

    // ---- sign-in ------------------------------------------------------------------------------------------------

    /** Signed out: the welcome screen, "Use another server" (the password forms are only for one), then "Sign in". */
    private fun openSignIn() {
        launch()
        awaitTag("welcome")
        tap("useAnotherServer")
        awaitTag("server")
        rule.onNodeWithTag("server").performTextReplacement("https://s.example.com")
        tap("startSignIn")
        awaitTag("signIn")
    }

    /** The setup after a fresh sign-in, skipped. */
    private fun skipSetup() {
        awaitTag("setup")
        tap("setupSkip")
    }

    @Test
    fun signInRegistersThePhoneAndShowsActivity() {
        core.session = null
        openSignIn()
        rule.onNodeWithTag("signIn").assertIsNotEnabled()
        rule.onNodeWithTag("email").performTextReplacement("me@example.com")
        rule.onNodeWithTag("password").performTextReplacement("hunter2")
        rule.onNodeWithTag("signIn").performScrollTo().assertIsEnabled().performClick()
        skipSetup()
        awaitTag("noActivity")
        assertEquals(listOf("https://s.example.com", "me@example.com", "hunter2", null), core.logins.single())
        val state = (context.applicationContext as ReinsApp).container.state
        awaitCore { state.approvalDevice.value }
        assertEquals(1, core.registrations.size)
    }

    @Test
    fun theSignInFormSaysWhichServerItUses() {
        core.session = null
        openSignIn()
        assertFalse(has("server"))
        rule.onNodeWithTag("serverName").assertTextContains("s.example.com", substring = true)
    }

    @Test
    fun aTwoFactorPromptAppearsWhenTheServerAsksForOne() {
        core.session = null
        core.loginError = CoreException.TwoFactorRequired()
        openSignIn()
        rule.onNodeWithTag("email").performTextReplacement("me@example.com")
        rule.onNodeWithTag("password").performTextReplacement("hunter2")
        rule.onNodeWithTag("signIn").performScrollTo().performClick()
        awaitTag("totp")
        core.loginError = null
        rule.onNodeWithTag("totp").performTextReplacement("123456")
        rule.onNodeWithTag("signIn").performScrollTo().assertIsEnabled().performClick()
        skipSetup()
        awaitTag("noActivity")
        assertEquals("123456", core.logins.last()[3])
    }

    @Test
    fun wrongCredentialsShowAnActionableError() {
        core.session = null
        core.loginError = CoreException.InvalidCredentials()
        openSignIn()
        rule.onNodeWithTag("email").performTextReplacement("me@example.com")
        rule.onNodeWithTag("password").performTextReplacement("nope")
        rule.onNodeWithTag("signIn").performScrollTo().performClick()
        awaitText("Wrong email, password or two-factor code.")
    }

    // ---- the main screen -----------------------------------------------------------------------------------------

    @Test
    fun activityIsTheFirstTabAndShowsWaitingRequestsAboveTheHistory() {
        core.pending = listOf(TestData.pending())
        core.activity = listOf(TestData.entry(2, "send", "sent", 1u), TestData.entry(1))
        launch()
        awaitTag("pending:req1")
        awaitTag("entry:2")
        awaitTag("entry:1")
        // The waiting card says what it wants; the history says what was done.
        rule.onNodeWithTag("sentence:req1", useUnmergedTree = true).assertTextContains("Wants to search Gmail", substring = true)
        rule.onNodeWithTag("entryTitle:1", useUnmergedTree = true).assertTextEquals("Searched Gmail")
        rule.onNodeWithTag("entryTitle:2", useUnmergedTree = true).assertTextEquals("Sent an email")
    }

    /** The number on a nav item, read from its (merged) text; null when there is none. */
    private fun navCount(tab: String): Int? {
        settle()
        val nodes = rule.onAllNodes(hasTestTag(tab)).fetchSemanticsNodes()
        val config = nodes.firstOrNull()?.config ?: return null
        val texts = if (config.contains(androidx.compose.ui.semantics.SemanticsProperties.Text)) config[androidx.compose.ui.semantics.SemanticsProperties.Text] else return null
        return texts.map { it.text }.firstNotNullOfOrNull { it.toIntOrNull() ?: it.removeSuffix("+").toIntOrNull() }
    }

    @Test
    fun theNavBarCountsUnseenActivityAndActiveGrants() {
        core.grants = listOf(TestData.grant("g1"), TestData.grant("g2"), TestData.grant("g3", active = false))
        core.activity = (60L downTo 1L).map { TestData.entry(it) }
        (context.applicationContext as ReinsApp).container.state.setSeenActivityId(0)
        launch()
        awaitTag("tabGrants")
        rule.waitUntil(10_000) { navCount("tabGrants") == 2 }
        assertEquals(2, navCount("tabGrants"))
        // Most of the 60 entries are far off screen, so most are still unseen.
        rule.waitUntil(10_000) { (navCount("tabActivity") ?: 0) > 20 }
        // Jumping to the latest shows the newest, which clears what is above.
        tap("scrollToLatest")
        rule.waitUntil(10_000) { navCount("tabActivity") == null }
    }

    @Test
    fun allActivityIsMarkedSeenOnceTheNewestEntriesAreOnScreen() {
        core.activity = listOf(TestData.entry(3), TestData.entry(2), TestData.entry(1))
        launch()
        awaitTag("entry:3")
        // Everything fits on screen, so nothing stays unseen.
        rule.waitUntil(10_000) { navCount("tabActivity") == null }
    }

    @Test
    fun anAccountsRequestShowsExactlyWhatWouldBeSharedAndDefaultsToAMonth() {
        core.pending = listOf(TestData.pending("req5", "accounts", 2u, waitUntil = 1_700_000_245))
        core.approval = TestData.accountsView()
        launch()
        openItem("req5")
        awaitTag("accountsCard")
        awaitTag("shared:0")
        awaitTag("shared:1")
        rule.onNodeWithText("work@corp.example").assertExists()
        rule.onNodeWithTag("what").assertTextContains("Gmail addresses", substring = true)
        rule.onNodeWithTag("lifetimeCustom").assertTextContains("30 days", substring = true)
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val choice = core.approvals.single().second
        assertEquals("a month unless changed", 2_592_000uL, choice.standing!!.durationSecs)
        assertEquals("everything starts ticked", listOf("me@gmail.com", "work@corp.example"), choice.selectedMessageIds)
        assertEquals(1, prompts.get())
    }

    @Test
    fun anAccountThatIsUntickedIsNotSharedAndCannotBeSharedByAccident() {
        core.pending = listOf(TestData.pending("req5", "accounts", 3u, waitUntil = 1_700_000_245))
        core.approval = TestData.accountsView(listOf("a@gmail.com", "b@gmail.com", "c@gmail.com"))
        launch()
        openItem("req5")
        awaitTag("acct:b@gmail.com")
        tap("acct:b@gmail.com")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("a@gmail.com", "c@gmail.com"), core.approvals.single().second.selectedMessageIds)
    }

    @Test
    fun withNothingTickedTheAccountsCannotBeApproved() {
        core.pending = listOf(TestData.pending("req5", "accounts", 2u, waitUntil = 1_700_000_245))
        core.approval = TestData.accountsView()
        launch()
        openItem("req5")
        tap("acct:me@gmail.com")
        tap("acct:work@corp.example")
        tap("approve")
        awaitTextContaining("Tick at least one account")
        assertTrue(core.approvals.isEmpty())
        assertEquals("no prompt for something that cannot go through", 0, prompts.get())
    }

    @Test
    fun accountsTheAiCanAlreadySeeAreTickedAndLockedWhenItAsksForMore() {
        core.pending = listOf(TestData.pending("req5", "accounts", 1u, waitUntil = 1_700_000_245))
        core.approval = TestData.accountsView(listOf("a@gmail.com", "b@gmail.com"), shared = listOf("a@gmail.com"))
        launch()
        openItem("req5")
        awaitTag("acct:a@gmail.com")
        rule.onNodeWithTag("acct:a@gmail.com").assertIsNotEnabled()
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("only the new one is a pick", listOf("b@gmail.com"), core.approvals.single().second.selectedMessageIds)
    }

    @Test
    fun anAccountsRequestCanBeAllowedJustOnceFromTheDock() {
        core.pending = listOf(TestData.pending("req5", "accounts", 2u, waitUntil = 1_700_000_245))
        core.approval = TestData.accountsView()
        launch()
        openItem("req5")
        rule.onNodeWithTag("lifetimeCustom").assertTextContains("30 days", substring = true)
        tap("lifetime:ONCE")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun anAccountsEntryListsExactlyTheAccountsThatWereShown() {
        core.activity = listOf(
            TestData.entry(8, "accounts", "released", 2u, info = TestData.info(null, emptyList(), null, null, null, listOf("me@gmail.com", "work@corp.example"))),
        )
        launch()
        tap("entry:8")
        tap("accountsShared")
        awaitTag("sharedAccount:1")
        rule.onNodeWithText("work@corp.example").assertExists()
        rule.onNodeWithTag("detailTitle").assertTextContains("Saw Gmail accounts")
    }

    @Test
    fun anEmailInAnEntryOpensInFullFromGmail() {
        core.emails["m0"] = dev.reins.core.EmailContent(
            "m0", "Bank <alerts@bank.com>", listOf("me@gmail.com"), emptyList(), "Your March statement is ready", 1_700_000_000,
            "Dear customer,\nyour statement is attached.\nRegards",
        )
        core.activity = listOf(
            TestData.entry(5, "read", "released", 1u, info = TestData.info(null, TestData.messages("Your March statement is ready"), null, null, null)),
        )
        launch()
        tap("entry:5")
        tap("emailsShared")
        tap("detailMessage:0")
        awaitTag("emailBody")
        rule.onNodeWithText("Dear customer,\nyour statement is attached.\nRegards").assertExists()
        rule.onNodeWithTag("emailSubject").assertExists()
        assertEquals(listOf("me@gmail.com" to "m0"), core.openedEmails.toList())
    }

    @Test
    fun anEmailThatIsGoneFromGmailSaysSoAndCanBeRetried() {
        core.activity = listOf(
            TestData.entry(5, "search", "released", 1u, info = TestData.info(null, TestData.messages("Old news"), null, null, null)),
        )
        launch()
        tap("entry:5")
        tap("emailsShared")
        tap("detailMessage:0")
        awaitTag("emailError")
        rule.onNodeWithText("Old news").assertExists()
        core.emails["m0"] = dev.reins.core.EmailContent("m0", "a@b.com", emptyList(), emptyList(), "Old news", 1_700_000_000, "Back again")
        tap("emailRetry")
        awaitTag("emailBody")
        rule.onNodeWithText("Back again").assertExists()
    }

    @Test
    fun anEntryFromBeforeIdsWereKeptListsItsEmailsButCannotOpenThem() {
        val old = TestData.messages("Ancient").map { it.copy(id = "") }
        core.activity = listOf(TestData.entry(5, "read", "released", 1u, info = TestData.info(null, old, null, null, null)))
        launch()
        tap("entry:5")
        tap("emailsShared")
        awaitTag("detailMessage:0")
        rule.onNodeWithTag("detailMessage:0").performClick()
        settle()
        assertFalse(has("emailBody") || has("emailLoading") || has("emailError"))
        assertTrue(core.openedEmails.isEmpty())
    }

    @Test
    fun aRequestOpensAnEightyPercentSheetWithOneTapApprove() {
        searchRequest()
        launch()
        openItem("req1")
        awaitTag("check:m1")
        rule.onNodeWithTag("what").assertIsDisplayed()
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val (id, choice) = core.approvals.single()
        assertEquals("req1", id)
        assertEquals("everything found starts ticked", listOf("m1", "m2", "m3"), choice.selectedMessageIds)
        assertNull(choice.standing)
        assertEquals(1, prompts.get())
        awaitGone("sheet")
    }

    @Test
    fun theDockOffersHowLongAndCustomHoldsTheRest() {
        searchRequest()
        launch()
        openItem("req1")
        awaitTag("lifetime:HOUR")
        assertFalse(has("allMail:HOUR"))
        assertFalse(has("lifetime:UNTIL_REVOKED"))
        tap("lifetimeCustom")
        awaitTag("lifetime:UNTIL_REVOKED")
        assertFalse("what it covers stays folded", has("allMail:HOUR"))
        tap("moreToggle")
        awaitTag("allMail:HOUR")
    }

    @Test
    fun unticksAndClearWorkAndNothingTickedIsRefused() {
        searchRequest()
        launch()
        openItem("req1")
        tap("check:m2")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("m1", "m3"), core.approvals.single().second.selectedMessageIds)
    }

    @Test
    fun clearThenApproveAsksForAtLeastOneEmail() {
        core.pending = listOf(TestData.pending())
        core.approval = TestData.searchView().copy(messages = listOf(TestData.message("m1", "Bank <a@bank.com>"), TestData.message("m2", "b@x.com")))
        launch()
        openItem("req1")
        tap("clearAll")
        awaitTag("selectAll")
        tap("approve")
        awaitText("Select at least one message.")
        assertTrue(core.approvals.isEmpty())
        assertEquals(0, prompts.get())
        tap("selectAll")
        awaitTag("clearAll")
    }

    @Test
    fun aStandingHourlyGrantIsOneSegment() {
        searchRequest()
        launch()
        openItem("req1")
        tap("lifetime:HOUR")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(3_600UL, standing.durationSecs)
        assertTrue(standing.scope.selectedMessagesOnly)
    }

    @Test
    fun allowingAllMailForADayIsOneChipInCustom() {
        searchRequest()
        launch()
        openItem("req1")
        tap("lifetimeCustom")
        tap("moreToggle")
        tap("allMail:DAY")
        tap("rememberConfirm")
        rule.onNodeWithTag("lifetimeCustom").assertTextContains("All mail", substring = true)
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertTrue(standing.scope.allMail)
        assertEquals(86_400UL, standing.durationSecs)
    }

    @Test
    fun widerGrantsToAPublicMailDomainAreWarnedAbout() {
        searchRequest()
        launch()
        openItem("req1")
        tap("check:m1")
        tap("lifetime:DAY")
        tap("lifetimeCustom")
        tap("moreToggle")
        tap("similar")
        tap("senderDomain:gmail.com")
        tap("rememberConfirm")
        awaitTag("publicDomainWarning")
        tap("help:publicDomain")
        awaitTextContaining("millions")
    }

    @Test
    fun denyIsOneTapWithNoAuthentication() {
        searchRequest()
        launch()
        openItem("req1")
        tap("deny")
        awaitCore { core.denials.isNotEmpty() }
        assertEquals(listOf("req1"), core.denials.toList())
        assertEquals(0, prompts.get())
    }

    @Test
    fun aCancelledBiometricPromptApprovesNothing() {
        searchRequest()
        authResult = AuthResult.Cancelled
        launch()
        openItem("req1")
        tap("approve")
        awaitCore { prompts.get() == 1 }
        assertTrue(core.approvals.isEmpty())
        rule.onNodeWithTag("approve").assertIsEnabled()
    }

    @Test
    fun withoutAScreenLockApprovalFailsClosedWithAnExplanation() {
        searchRequest()
        authResult = AuthResult.Unavailable
        launch()
        openItem("req1")
        tap("approve")
        awaitText("Set a screen lock or fingerprint on this phone to approve.")
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun aSendRequestShowsTheWholeEmailAndCanBeApproved() {
        core.pending = listOf(TestData.pending("req2", "send", 2u))
        core.approval = TestData.sendView()
        launch()
        openItem("req2")
        awaitTag("emailPreview")
        rule.onNodeWithText("Body text").assertIsDisplayed()
        rule.onNodeWithText("Hi there").assertIsDisplayed()
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertTrue(core.approvals.single().second.selectedMessageIds.isEmpty())
    }

    @Test
    fun aSendCanBeWidenedToADomainUnderMore() {
        core.pending = listOf(TestData.pending("req2", "send", 2u))
        core.approval = TestData.sendView()
        launch()
        openItem("req2")
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
        tap("domain:ann@corp.com")
        tap("rememberConfirm")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val scope = core.approvals.single().second.standing!!.scope
        assertEquals(listOf("bob@corp.com"), scope.recipientAddresses)
        assertEquals(listOf("corp.com"), scope.recipientDomains)
    }

    // ---- permission requests --------------------------------------------------------------------------------------

    @Test
    fun aPermissionRequestIsSetApartAndCanOnlyBeShortened() {
        core.pending = listOf(TestData.pending("req3", "grant", 1u))
        core.approval = TestData.grantView(duration = 3600u)
        launch()
        openItem("req3")
        awaitTag("grantCard")
        rule.onNodeWithTag("approve").assertTextContains("Allow", substring = true)
        tap("grantPeriod")
        awaitTag("grantSlider")
        rule.onNodeWithTag("grantSlider").performSemanticsAction(SemanticsActions.SetProgress) { it(0f) }
        settle()
        tap("grantDone")
        rule.onNodeWithTag("grantPeriod").assertTextContains("For 1 minute", substring = true)
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(60UL, standing.durationSecs)
    }

    @Test
    fun allowingAPermissionAsAskedSendsNoOverride() {
        core.pending = listOf(TestData.pending("req3", "grant", 1u))
        core.approval = TestData.grantView(duration = 3600u)
        launch()
        openItem("req3")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    // ---- freshness ------------------------------------------------------------------------------------------------

    @Test
    fun theSheetSaysHowLongTheAiKeepsWaiting() {
        core.pending = listOf(TestData.pending(waitUntil = 1_700_000_140))
        core.approval = TestData.searchView(waitUntil = 1_700_000_140)
        launch()
        openItem("req1")
        awaitTag("waitLine")
        rule.onNodeWithTag("waitLine").assertIsDisplayed()
        rule.onNodeWithTag("waitLine").assertTextContains("40s", substring = true)
    }

    @Test
    fun afterTheWaitEndsTheSheetSaysApprovingStillWorks() {
        core.pending = listOf(TestData.pending(waitUntil = 1_700_000_050))
        core.approval = TestData.searchView(waitUntil = 1_700_000_050)
        launch()
        openItem("req1")
        awaitTag("lateBanner")
        tap("lateBanner")
        awaitTextContaining("stopped waiting")
        tap("lateBanner")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
    }

    @Test
    fun aNewRequestPopsUpByItselfWhenTheAppIsInFrontAndOnlyOnce() {
        core.approval = TestData.searchView()
        launch()
        awaitTag("noActivity")
        Foreground.focused = true
        // The foreground poll (FakeCore.sync) brings it in, as on a phone.
        core.pending = listOf(TestData.pending())
        awaitTag("approve")
        tap("closeSheet")
        awaitGone("approve")
        // Dismissed, it stays in the list and does not pop up again.
        settle()
        assertFalse(has("approve"))
        awaitTag("pending:req1")
    }

    // ---- pairing --------------------------------------------------------------------------------------------------

    @Test
    fun pairingNeedsTheMatchingCodeAndSendsTheLabel() {
        core.pending = listOf(TestData.pairingItem("pair1"))
        core.pairing = PairingView("pair1", "Claude", "claude.ai", byteArrayOf(7, 42, 99), 1_700_000_200, null)
        launch()
        awaitTag("pending:pair1")
        if (!has("sheet")) tap("pending:pair1")
        awaitTag("code:42")
        rule.onNodeWithTag("approve").assertIsNotEnabled()
        tap("code:07")
        rule.onNodeWithTag("approve").assertIsEnabled()
        tap("code:42")
        rule.onNodeWithTag("label").performTextReplacement("Claude Work")
        tap("approve")
        awaitCore { core.pairingAnswers.isNotEmpty() }
        val answer = core.pairingAnswers.single()
        assertEquals("pair1", answer[0])
        assertEquals(true, answer[1])
        assertEquals(42.toUByte(), answer[2])
        assertEquals("Claude Work", answer[3])
    }

    @Test
    fun withoutAScreenLockPairingOffersAndroidsSettingsForOne() {
        authResult = AuthResult.Unavailable
        core.pending = listOf(TestData.pairingItem("pair1"))
        core.pairing = PairingView("pair1", "Claude", "claude.ai", byteArrayOf(7, 42, 99), 1_700_000_200, null)
        launch()
        awaitTag("pending:pair1")
        if (!has("sheet")) tap("pending:pair1")
        tap("code:42")
        tap("approve")
        awaitTag("screenLockOff")
        assertTrue(core.pairingAnswers.isEmpty())
        tap("setScreenLock")
        assertEquals(android.provider.Settings.ACTION_BIOMETRIC_ENROLL, shadowOf(context as android.app.Application).nextStartedActivity?.action)
    }

    @Test
    fun activitySaysWhenThePhoneHasNoScreenLock() {
        dev.reins.android.platform.ScreenLock.check = { false }
        launch()
        awaitTag("screenLockOff")
        tap("setScreenLock")
        assertEquals(android.provider.Settings.ACTION_BIOMETRIC_ENROLL, shadowOf(context as android.app.Application).nextStartedActivity?.action)
        // Set in Android's settings, back in the app: the card is gone.
        dev.reins.android.platform.ScreenLock.check = { true }
        scenario!!.moveToState(androidx.lifecycle.Lifecycle.State.STARTED)
        scenario!!.moveToState(androidx.lifecycle.Lifecycle.State.RESUMED)
        awaitGone("screenLockOff")
    }

    @Test
    fun theDesktopAppsKeyIsShownToCompareBeforeConnecting() {
        core.pending = listOf(TestData.pairingItem("pair1"))
        core.pairing = TestData.pairingView(name = "Reins desktop app on laptop", host = "laptop", keyFingerprint = "4821 9930")
        launch()
        openPairing("pair1")
        awaitTag("keyFingerprint")
        rule.onNodeWithTag("keyFingerprint").assertTextContains("Desktop app key")
        rule.onNodeWithTag("keyFingerprint").assertTextContains("4821 9930")
        rule.onNodeWithTag("keyFingerprint")
            .assertTextContains("Check that your computer shows the same numbers. If they differ, deny.")
        // It sits above the codes, so it is read before anything can be approved.
        val card = rule.onNodeWithTag("keyFingerprint").fetchSemanticsNode().boundsInRoot
        val codes = rule.onNodeWithTag("code:42").fetchSemanticsNode().boundsInRoot
        assertTrue(card.bottom <= codes.top)
        tap("deny")
        awaitCore { core.pairingAnswers.isNotEmpty() }
        assertEquals(false, core.pairingAnswers.single()[1])
    }

    @Test
    fun anAiPairingShowsNoKey() {
        core.pending = listOf(TestData.pairingItem("pair1"))
        core.pairing = TestData.pairingView()
        launch()
        openPairing("pair1")
        assertFalse(has("keyFingerprint"))
    }

    private fun openPairing(id: String) {
        awaitTag("pending:$id")
        if (!has("sheet")) tap("pending:$id")
        awaitTag("code:42")
    }

    // ---- git through the desktop app -------------------------------------------------------------------------------

    private fun openGitPush(view: dev.reins.core.ApprovalView) {
        core.pending = listOf(
            TestData.pending(view.requestId, "write", 1u, label = view.connectionLabel, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle),
        )
        core.approval = view
        launch()
        openItem(view.requestId)
        awaitTag("gitPush")
    }

    private fun count(prefix: String): Int {
        settle()
        return rule.onAllNodes(
            androidx.compose.ui.test.SemanticsMatcher("tag starts with $prefix") {
                it.config.getOrElseNullable(androidx.compose.ui.semantics.SemanticsProperties.TestTag) { null }?.startsWith(prefix) == true
            },
        ).fetchSemanticsNodes().size
    }

    @Test
    fun aPushShowsTheBranchItsCommitsAndItsFiles() {
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(), notes = listOf("Line counts are approximate"))))
        rule.onNodeWithTag("what").assertTextContains("push 9 commits", substring = true).assertTextContains("octo/app", substring = true)
        rule.onNodeWithTag("gitRef:0").assertTextContains("main")
        assertFalse("an ordinary update needs no chip", has("gitChip:0"))
        // The git section replaces the core's plain lines.
        assertFalse(has("writePreview"))
        assertFalse(has("forceWarning:0"))
        assertFalse(has("forceUnknown:0"))
        assertFalse(has("onceWarning"))
        rule.onNodeWithTag("gitNote:0").assertTextContains("Line counts are approximate")

        // The commits fold into one row with the lines added and removed; it opens them, and how many were not listed.
        rule.onNodeWithTag("gitCommits:0").assertTextContains("9 commits", substring = true)
        rule.onNodeWithTag("gitTotals:0", useUnmergedTree = true).assertExists()
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        rule.onNodeWithTag("pageTitle").assertTextContains("9 commits")
        rule.onNodeWithTag("gitCommit:0:0").assertTextContains("a1b2c01").assertTextContains("Commit number 1").assertTextContains("Ada Lovelace")
        assertEquals(7, count("gitCommit:0:"))
        rule.onNodeWithTag("gitCommitsNotListed:0").assertTextContains("2 more not listed")
        // The dock stays on every page.
        awaitTag("approve")

        // The files of the whole push, with their counts, and how many were not listed.
        tap("gitFiles:0")
        awaitTag("gitFile:0:0")
        rule.onNodeWithTag("gitFile:0:0").assertTextContains("src/module1/File1.kt", substring = true).assertTextContains("+3").assertTextContains("−1")
        assertEquals(10, count("gitFile:0:"))
        rule.onNodeWithTag("gitFilesNotListed:0").assertTextContains("2 more not listed")
        pressBack()
        pressBack()
        awaitTag("hero")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("req20", core.approvals.single().first)
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun aCommitOpensToItsFilesAndEachFileToItsLines() {
        openGitPush(TestData.designPushView())
        tap("gitCommits:0")
        tap("gitCommit:0:0")
        awaitTag("commitSubject")
        rule.onNodeWithTag("commitSubject").assertTextContains("Log in with a passkey when the browser offers one")
        rule.onNodeWithTag("commitFile:0").assertTextContains("src/auth/login.ts", substring = true).assertTextContains("+38")
        assertFalse(has("commitFile:0:diff"))
        tap("commitFile:0")
        awaitTag("commitFile:0:diff")
        awaitTextContaining("browserSupportsPasskeys")
        tap("commitFile:0")
        awaitGone("commitFile:0:diff")
    }

    @Test
    fun aCommitTheDesktopAppDidNotCompareDoesNotOpen() {
        openGitPush(TestData.gitPushView())
        tap("gitCommits:0")
        awaitTag("gitCommit:0:0")
        rule.onNodeWithTag("gitCommit:0:0").performClick()
        settle()
        assertFalse(has("commitSubject"))
    }

    @Test
    fun hiddenCharactersInACommitAreFlagged() {
        val commit = TestData.designCommit().copy(hiddenChars = true)
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(commits = listOf(commit)))))
        awaitTag("hiddenChars")
        tap("gitCommits:0")
        awaitTag("commitHidden")
    }

    @Test
    fun aForcePushWarnsThatItRewritesHistoryAndIsAskedEveryTime() {
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)), noStanding = true))
        rule.onNodeWithTag("forceWarning:0").assertTextContains("Rewrites history")
        tap("help:forceWarning:0")
        awaitTextContaining("replaces commits")
        tap("help:forceWarning:0")
        awaitGone("helpBubble")
        assertFalse(has("forceUnknown:0"))
        awaitTag("noStanding")
        rule.onAllNodes(hasTestTag("lifetime:HOUR")).assertCountEquals(0)
        rule.onNodeWithTag("approve").assertTextContains("Hold to approve")
        holdApprove()
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun aPushWhoseHistoryCouldNotBeCheckedIsTreatedAsAForcePush() {
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true, forceUnknown = true)), noStanding = true))
        rule.onNodeWithTag("forceUnknown:0").assertTextContains("History not checked")
        assertFalse(has("forceWarning:0"))
    }

    @Test
    fun aTagPushShowsATagChipAndANewBranchItsOwn() {
        val tag = TestData.gitRef("v1.2", kind = "tag", change = "create", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        val branch = TestData.gitRef("feature/login", change = "create", commitCount = 2u, commits = (1..2).map(TestData::gitCommit), filesChanged = 1u, files = listOf(TestData.gitFile("README.md", "added", 4u, 0u)), additions = 4u, deletions = 0u)
        openGitPush(TestData.gitPushView(TestData.gitPush(tag, branch), tags = true))
        rule.onNodeWithTag("what").assertTextContains("push 2 commits", substring = true)
        rule.onNodeWithTag("gitRef:0").assertTextContains("v1.2")
        rule.onNodeWithTag("gitChip:0", useUnmergedTree = true).assertTextContains("New tag")
        assertFalse(has("gitCommits:0"))
        rule.onNodeWithTag("gitChip:1", useUnmergedTree = true).assertTextContains("New branch")
        rule.onNodeWithTag("gitCommits:1").assertTextContains("2 commits", substring = true)
        tap("gitCommits:1")
        awaitTag("gitCommit:1:1")
        assertFalse(has("gitCommitsNotListed:1"))
    }

    @Test
    fun lineCountsAreLeftOutWhenTheyCouldNotBeCounted() {
        val files = listOf(
            TestData.gitFile("assets/logo.png", "added", null, null, binary = true),
            TestData.gitFile("data/huge.json", "modified", null, null),
        )
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(commitCount = 0u, commits = emptyList(), filesChanged = 2u, files = files, additions = null, deletions = null))))
        rule.onNodeWithTag("gitFiles:0").assertTextContains("2 files", substring = true)
        tap("gitFiles:0")
        rule.onNodeWithTag("gitFile:0:0").assertTextContains("binary", substring = true)
        assertFalse(has("gitFilesNotListed:0"))
    }

    @Test
    fun aGitFetchIsAnOrdinaryItemList() {
        val view = TestData.gitFetchView()
        core.pending = listOf(TestData.pending(view.requestId, "read", 1u, label = view.connectionLabel, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle))
        core.approval = view
        launch()
        openItem(view.requestId)
        rule.onNodeWithTag("what").assertTextContains("clone and fetch with git", substring = true)
        awaitTag("message:octo/app")
        assertFalse(has("gitPush"))
    }

    @Test
    fun gitOperationsAreNamedInTheListAndTheActivity() {
        core.pending = listOf(TestData.pending("req21", "read", 1u, label = "Reins desktop app on laptop", service = "github", account = "octo-cat", op = "git_fetch", opTitle = "Clone and fetch with git"))
        core.activity = listOf(
            TestData.entry(7, "write", "sent", count = 1u, opTitle = "Push with git")
                .copy(connectionLabel = "Reins desktop app on laptop", service = "github", account = "octo-cat", op = "git_push"),
        )
        Foreground.autoPopup = false
        launch()
        awaitTag("pending:req21")
        rule.onNodeWithTag("sentence:req21", useUnmergedTree = true).assertTextContains("Wants to clone and fetch with git", substring = true)
        rule.onNodeWithTag("entryTitle:7", useUnmergedTree = true).assertTextEquals("Pushed with git")
        rule.onNodeWithText("Reins desktop app on laptop", substring = true).assertExists()
        tap("entry:7")
        awaitTag("detailTitle")
        rule.onNodeWithTag("detailTitle").assertTextContains("Pushed with git")
    }

    // ---- deep links -----------------------------------------------------------------------------------------------

    @Test
    fun spoofedDeepLinksNeverOpenTheSheet() {
        for (id in listOf("nope", "../../etc", "req1;drop")) {
            launch(link("request", id))
            awaitTag("noActivity")
            settle()
            assertFalse(has("approve"))
            scenario!!.close()
        }
        // A well-formed id of something that is not waiting says so instead of opening anything.
        launch(link("request", "unknownid"))
        awaitText("That request is no longer waiting.")
        assertFalse(has("approve"))
    }

    @Test
    fun aGenuineNotificationLinkOpensTheSheet() {
        searchRequest()
        launch(link("request", "req1"))
        awaitTag("approve")
    }

    @Test
    fun aLinkToAnUnknownItemLandsOnActivityWithAnExplanation() {
        launch(link("request", "gone"))
        awaitText("That request is no longer waiting.")
    }

    // ---- activity details -----------------------------------------------------------------------------------------

    @Test
    fun anActivityEntryOpensToShowWhatWasReleased() {
        core.activity = listOf(
            TestData.entry(
                5, "read", "released", 2u,
                info = TestData.info("from:bank", TestData.messages("Your statement", "Wire receipt"), null, null, null),
            ),
        )
        launch()
        tap("entry:5")
        awaitTag("detailTitle")
        awaitTextContaining("Read 2 emails")
        tap("emailsShared")
        awaitTag("detailMessage:0")
        rule.onNodeWithText("Your statement").assertExists()
        rule.onNodeWithText("Wire receipt").assertExists()
    }

    @Test
    fun anApprovedSendShowsTheEmailThatWentOutAndToWhom() {
        core.activity = listOf(
            TestData.entry(
                9, "send", "sent", 2u,
                info = TestData.info(null, emptyList(), EmailView(listOf("ann@corp.com"), listOf("bob@corp.com"), "Quarterly report", "Attached, as discussed."), null, null),
            ),
        )
        launch()
        tap("entry:9")
        awaitTag("detailEmail")
        rule.onNodeWithText("ann@corp.com").assertExists()
        rule.onNodeWithText("bob@corp.com").assertExists()
        rule.onNodeWithText("Quarterly report").assertExists()
        rule.onNodeWithText("Attached, as discussed.").assertExists()
    }

    @Test
    fun anEntryUnderAGrantLinksToTheGrant() {
        core.grants = listOf(TestData.grant("g1"))
        core.activity = listOf(TestData.entry(4, grantId = "g1"))
        launch()
        tap("entry:4")
        tap("openGrantFromEntry")
        awaitTag("grantTitle")
    }

    // ---- grants ---------------------------------------------------------------------------------------------------

    @Test
    fun grantsListActiveOnesFirstAndOpenToTheirDetails() {
        core.grants = listOf(TestData.grant("old", active = false), TestData.grant("g1", uses = 4u))
        core.activity = listOf(TestData.entry(7, grantId = "g1"))
        launch()
        tap("tabGrants")
        awaitTag("grant:g1")
        rule.onNodeWithText("Claude · used 4 times").assertExists()
        tap("grant:g1")
        awaitTag("grantTitle")
        rule.onNodeWithText("Read emails from @bank.com").assertExists()
        rule.onNodeWithText("From @bank.com").assertExists()
        rule.onNodeWithTag("uses").assertTextContains("4", substring = true)
        awaitTag("revoke")
    }

    @Test
    fun theGrantsTabHasNoSubtitleAndKeepsEndedGrantsInAClosedList() {
        core.grants = listOf(
            TestData.grant("g1"),
            TestData.grant("old", active = false),
            TestData.grant("spent", active = false, state = "used_up", maxUses = 3u, uses = 3u),
            TestData.grant("gone", active = false, state = "revoked"),
        )
        launch()
        tap("tabGrants")
        awaitTag("grant:g1")
        assertFalse(has("ended:old"))
        assertFalse("nothing explains the list under the title", rule.onAllNodes(hasText("active. Everything else asks you first.", substring = true)).fetchSemanticsNodes().isNotEmpty())
        rule.onNodeWithTag("expiredHeader").assertTextContains("Expired", substring = true).assertTextContains("3", substring = true)
        tap("expiredHeader")
        awaitTag("ended:old")
        awaitTag("ended:spent")
        awaitTag("ended:gone")
        rule.onNodeWithText("Deleted", substring = true).assertExists()
        rule.onNodeWithText("All 3 uses spent", substring = true).assertExists()
        // The list stays open while the user looks at a grant and comes back.
        tap("ended:old")
        awaitTag("grantTitle")
        rule.onNodeWithContentDescription("Back").performClick()
        awaitTag("expiredList")
    }

    @Test
    fun aRunningGrantShowsItsTimeLeftAsAPieAndWarnsWhenItEndsSoon() {
        core.grants = listOf(
            TestData.grant("calm", leftSeconds = 3_000, ageSeconds = 600),
            TestData.grant("soon", leftSeconds = 240, ageSeconds = 3_360),
            TestData.grant("days", leftSeconds = 5 * 86_400 + 100, ageSeconds = 100),
        )
        launch()
        tap("tabGrants")
        awaitTag("grant:calm")
        rule.onNodeWithText("50m").assertExists()
        rule.onNodeWithText("4m").assertExists()
        rule.onNodeWithText("5d").assertExists()
        assertEquals("only the grant that is nearly over is marked", 1, rule.onAllNodes(hasTestTag("endsSoon"), useUnmergedTree = true).fetchSemanticsNodes().size)
        assertEquals(3, rule.onAllNodes(hasTestTag("pie"), useUnmergedTree = true).fetchSemanticsNodes().size)
    }

    @Test
    fun anEndedGrantCanBeResumedForAChosenPeriodAfterAuthentication() {
        core.grants = listOf(TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("resume:old")
        awaitTag("resumeDialog")
        tap("period:WEEK")
        tap("confirmResume")
        awaitCore { core.resumed.isNotEmpty() }
        assertEquals(listOf("old" to 604_800uL), core.resumed.toList())
        assertEquals("resuming needs the same authentication as approving", 1, prompts.get())
        awaitTag("grant:old")
    }

    @Test
    fun aCustomTimeCanBeTypedUnderMoreOptions() {
        core.grants = listOf(TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("resume:old")
        awaitTag("resumeDialog")
        assertFalse("the extras stay tucked away", has("customAmount"))
        tap("resumeMore")
        awaitTag("customAmount")
        rule.onNodeWithTag("customAmount").performTextReplacement("90")
        tap("unit:MINUTES")
        tap("confirmResume")
        awaitCore { core.resumed.isNotEmpty() }
        assertEquals(listOf("old" to 5_400uL), core.resumed.toList())
        assertTrue(core.resumedEdited.isEmpty())
    }

    @Test
    fun theSendersAndTheUsesOfAnEndedGrantCanBeTweakedWhenResuming() {
        core.grants = listOf(TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("resume:old")
        tap("resumeMore")
        awaitTag("resumeParties")
        rule.onNodeWithTag("resumeParties").performTextReplacement("@bank.com, alerts@other.com")
        tap("usesLimit")
        rule.onNodeWithTag("resumeUses").performTextReplacement("4")
        tap("period:WEEK")
        tap("confirmResume")
        awaitCore { core.resumedEdited.isNotEmpty() }
        val (id, standing) = core.resumedEdited.single()
        assertEquals("old", id)
        assertEquals(604_800uL, standing.durationSecs)
        assertEquals(4u, standing.maxUses)
        assertEquals(listOf("alerts@other.com"), standing.scope.senderAddresses)
        assertEquals(listOf("bank.com"), standing.scope.senderDomains)
        assertTrue(core.resumed.isEmpty())
        assertEquals(1, prompts.get())
    }

    @Test
    fun anInvalidResumeExplainsWhatIsWrongAndChangesNothing() {
        core.grants = listOf(TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("resume:old")
        tap("resumeMore")
        rule.onNodeWithTag("resumeParties").performTextReplacement("")
        tap("confirmResume")
        awaitTag("resumeInvalid")
        assertTrue(core.resumed.isEmpty() && core.resumedEdited.isEmpty())
        assertEquals(0, prompts.get())
    }

    @Test
    fun aGrantTiedToSpecificEmailsSaysItCanOnlyComeBackAsItWas() {
        core.grants = listOf(TestData.grant("old", active = false, editable = null))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("resume:old")
        tap("resumeMore")
        awaitTextContaining("Comes back as it was")
        assertFalse(has("resumeParties"))
    }

    @Test
    fun aGrantForAllMailCannotBeResumedForAMonth() {
        core.grants = listOf(TestData.grant("wide", active = false, allMail = true))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("resume:wide")
        awaitTag("resumeDialog")
        awaitTag("period:WEEK")
        assertFalse(has("period:MONTH"))
    }

    @Test
    fun withNothingRunningThePlaceholderStillShowsAboveTheExpiredIsland() {
        core.grants = listOf(TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        awaitTag("noGrants")
        rule.onNodeWithText("No active grants").assertExists()
        awaitTag("expiredHeader")
        // The foreground poll re-reads the lists.
        core.grants = emptyList()
        awaitGone("expiredHeader")
        rule.onNodeWithText("No grants").assertExists()
    }

    @Test
    fun anEndedGrantCanBeDeletedForGoodAfterConfirming() {
        core.grants = listOf(TestData.grant("old", active = false), TestData.grant("other", active = false))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("delete:old")
        rule.onAllNodes(hasText("Delete")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitCore { core.deletedGrants.isNotEmpty() }
        assertEquals(listOf("old"), core.deletedGrants.toList())
        awaitGone("ended:old")
        awaitTag("ended:other")
    }

    @Test
    fun anEndedGrantCanBeDeletedFromItsDetailsToo() {
        core.grants = listOf(TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("ended:old")
        tap("deleteEnded")
        rule.onAllNodes(hasText("Delete")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitCore { core.deletedGrants.isNotEmpty() }
        awaitGone("grantTitle")
    }

    @Test
    fun theExpiredIslandOpensAndClosesFromItsHeader() {
        core.grants = listOf(TestData.grant("g1"), TestData.grant("old", active = false))
        launch()
        tap("tabGrants")
        awaitTag("grant:g1")
        assertFalse(has("expiredList"))
        tap("expiredHeader")
        awaitTag("expiredList")
        tap("expiredHeader")
        awaitGone("expiredList")
        awaitTag("grant:g1")
    }

    @Test
    fun anEntryLoggedBeforeConnectionIdsWereKeptStillGetsTheChosenIcon() {
        core.activity = listOf(TestData.entry(3).copy(connectionId = ""))
        launch()
        awaitTag("entry:3")
        tap("tabSettings")
        tap("connection:c1")
        tap("connectionIcon")
        tap("icon:grok")
        awaitCore { core.icons["c1"] == "grok" }
        rule.onNodeWithContentDescription("Back").performClick()
        tap("tabActivity")
        awaitTag("entry:3")
        settle()
        assertTrue(rule.onAllNodes(androidx.compose.ui.test.hasContentDescription("Grok"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
    }

    @Test
    fun aDeletedGrantCanBeResumedFromItsDetails() {
        core.grants = listOf(TestData.grant("gone", active = false, state = "revoked"))
        launch()
        tap("tabGrants")
        tap("expiredHeader")
        tap("ended:gone")
        awaitTag("grantTitle")
        assertFalse("an ended grant is deleted for good, not revoked", has("revoke"))
        awaitTag("deleteEnded")
        tap("resume")
        tap("confirmResume")
        awaitCore { core.resumed.isNotEmpty() }
        assertEquals("gone", core.resumed.single().first)
    }

    @Test
    fun aGrantCanBeRevokedFromItsDetails() {
        core.grants = listOf(TestData.grant("g1"))
        launch()
        tap("tabGrants")
        tap("grant:g1")
        tap("revoke")
        rule.onAllNodes(hasText("End now")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitCore { core.revokedGrants.isNotEmpty() }
        assertEquals(listOf("g1"), core.revokedGrants.toList())
    }

    @Test
    fun withNothingConnectedTheEmptyActivityOffersToConnect() {
        core.connections = emptyList()
        launch()
        awaitTag("noActivity")
        tap("emptyConnectAi")
        val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
        assertEquals("http://127.0.0.1:8000/mcp", clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        awaitTextContaining("Copied http://127.0.0.1:8000/mcp")
        tap("emptyConnectComputer")
        awaitTag("scanQr")
    }

    @Test
    fun withAnAiConnectedTheEmptyActivityHasNoConnectButtons() {
        launch()
        awaitTag("noActivity")
        assertFalse(has("emptyConnectComputer"))
    }

    @Test
    fun settingsListsComputersAndAiAppsAndAComputersPageShowsItsKey() {
        core.connections = listOf(TestData.computer(), TestData.connection())
        launch()
        tap("tabSettings")
        awaitTag("connection:d1")
        rule.onNodeWithTag("connectionDetail:d1", useUnmergedTree = true).assertTextContains("computer", substring = true)
        rule.onNodeWithTag("connectionDetail:c1", useUnmergedTree = true).assertTextContains("claude.ai", substring = true)
        // An AI app is connected: the address is still there to copy, for the next one.
        tap("connect")
        awaitTag("copyMcpUrl")
        tap("closeSheet")
        // A computer's page shows the key it paired with.
        tap("connection:d1")
        awaitTag("connectionHeadline")
        rule.onNodeWithTag("connectionHeadline", useUnmergedTree = true).assertTextContains("Key 4821 9930", substring = true)
    }

    @Test
    fun theIntegrationsCountLeavesOutTheVault() {
        core.accounts = listOf(dev.reins.core.AccountView("vault", "me@example.com", 1_700_000_000))
        launch()
        tap("tabSettings")
        awaitTag("openIntegrations")
        rule.onNodeWithTag("openIntegrations").assertTextContains("None", substring = true)
    }

    @Test
    fun aNewGrantOffersOnlyMailAccountsNotTheVault() {
        // A new account has its vault and no Gmail yet: the mail grant has no account to offer.
        core.accounts = listOf(dev.reins.core.AccountView("vault", "me@example.com", 1_700_000_000))
        launch()
        tap("tabGrants")
        tap("newGrant")
        awaitTag("noAccounts")
        assertFalse(has("acct:me@example.com"))
    }

    @Test
    fun aGrantCanBeCreatedInAdvanceAndOneTimeIsAnOption() {
        launch()
        tap("tabGrants")
        tap("newGrant")
        awaitTag("createGrant")
        tap("kind:read")
        rule.onNodeWithTag("parties").performTextReplacement("alerts@bank.com, @statements.bank.com")
        tap("newLifetime:ONE_TIME")
        tap("createGrant")
        awaitCore { core.createdGrants.isNotEmpty() }
        val (conn, kind, standing) = core.createdGrants.single()
        assertEquals("c1", conn)
        assertEquals("the only account is used without asking", listOf("me@gmail.com"), core.createdGrantAccounts.toList())
        assertEquals(ApprovalKind.READ, kind)
        assertEquals(1u, standing.maxUses)
        assertNull(standing.durationSecs)
        assertEquals(listOf("alerts@bank.com"), standing.scope.senderAddresses)
        assertEquals(listOf("statements.bank.com"), standing.scope.senderDomains)
        assertEquals("creating a permission needs the same authentication", 1, prompts.get())
    }

    @Test
    fun anIncompleteNewGrantExplainsWhatIsMissing() {
        launch()
        tap("tabGrants")
        tap("newGrant")
        tap("createGrant")
        awaitTextContaining("Enter which senders")
        assertTrue(core.createdGrants.isEmpty())
    }

    @Test
    fun withSeveralAccountsANewGrantAsksWhichOneItIsFor() {
        core.accounts = listOf(
            AccountView("gmail", "me@gmail.com", 1_700_000_000),
            AccountView("gmail", "work@gmail.com", 1_700_000_050),
        )
        launch()
        tap("tabGrants")
        tap("newGrant")
        awaitTag("createGrant")
        rule.onNodeWithTag("parties").performTextReplacement("@bank.com")
        tap("createGrant")
        awaitTextContaining("Choose which Gmail account")
        assertTrue(core.createdGrants.isEmpty())
        tap("acct:work@gmail.com")
        tap("createGrant")
        awaitCore { core.createdGrants.isNotEmpty() }
        assertEquals(listOf("work@gmail.com"), core.createdGrantAccounts.toList())
    }

    // ---- integrations and accounts --------------------------------------------------------------------------------

    @Test
    fun theActivityScreenLeadsToTheIntegrationsAndTheirAccounts() {
        core.accounts = listOf(
            AccountView("gmail", "me@gmail.com", 1_700_000_000),
            AccountView("gmail", "work@gmail.com", 1_700_000_050),
        )
        launch()
        tap("integrations")
        awaitTag("service:gmail")
        rule.onNodeWithText("2 accounts").assertExists()
        tap("service:gmail")
        awaitTag("account:me@gmail.com")
        awaitTag("account:work@gmail.com")
        awaitTag("addAccount")
        awaitText("Connected")
    }

    @Test
    fun anAccountThatLostItsPermissionCanBeAllowedAgainAndAnyAccountCanBeRemoved() {
        core.accounts = listOf(
            AccountView("gmail", "me@gmail.com", 1_700_000_000),
            AccountView("gmail", "work@gmail.com", 1_700_000_050),
        )
        core.accountStatuses = mapOf("work@gmail.com" to GmailStatus.NeedsConsent)
        core.grants = listOf(TestData.grant("g1"), TestData.grant("gw").copy(account = "work@gmail.com"))
        launch()
        tap("integrations")
        tap("service:gmail")
        awaitTag("reconnect:work@gmail.com")
        assertFalse(has("reconnect:me@gmail.com"))
        tap("removeAccount:work@gmail.com")
        rule.onAllNodes(hasText("Remove")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitCore { core.removedAccounts.isNotEmpty() }
        assertEquals(listOf("work@gmail.com"), core.removedAccounts.toList())
        awaitGone("account:work@gmail.com")
        awaitTag("account:me@gmail.com")
    }

    @Test
    fun aFreshPhoneExplainsThatNoAccountIsConnectedYet() {
        core.accounts = emptyList()
        launch()
        tap("integrations")
        tap("service:gmail")
        awaitTag("noAccounts")
        awaitTag("addAccount")
    }

    @Test
    fun addingAGoogleAccountSaysWhatGoogleWillShow() {
        launch()
        tap("integrations")
        tap("service:gmail")
        awaitTag("googleSignInNote")
        rule.onNodeWithTag("googleSignInNote").assertTextContains("hasn't verified this app", substring = true)
    }

    @Test
    fun calendarSaysWhatGoogleWillShowToo() {
        launch()
        tap("integrations")
        tap("service:gcalendar")
        awaitTag("googleSignInNote")
    }

    @Test
    fun anAccountTheUserPickedIsConnectedThroughTheCore() {
        launch()
        tap("integrations")
        tap("service:gmail")
        awaitTag("addAccount")
        val container = (context.applicationContext as ReinsApp).container
        kotlinx.coroutines.runBlocking { dev.reins.android.ui.gmail.GmailViewModel(container).connect("Work@Gmail.com") }
        assertEquals(listOf("Work@Gmail.com"), core.addedAccounts.toList())
        awaitTag("account:work@gmail.com")
    }

    // ---- other integrations -----------------------------------------------------------------------------------------

    @Test
    fun everyIntegrationIsListedAndOpensItsOwnScreen() {
        launch()
        tap("integrations")
        // Text messages are listed in the `full` build only (DistributionTest).
        for (id in listOf("gmail", "gcalendar", "gcontacts", "telegram", "github", "device_calendar", "device_contacts", "vault")) {
            awaitTag("service:$id")
        }
        tap("service:telegram")
        awaitTag("noAccounts")
        awaitTag("phone")
    }

    @Test
    fun aTelegramAccountSignsInWithPhoneCodeAndPassword() {
        launch()
        tap("integrations")
        tap("service:telegram")
        awaitTag("phone")
        rule.onNodeWithTag("sendCode").assertIsNotEnabled()
        rule.onNodeWithTag("phone").performTextReplacement("+1 555 010 0100")
        tap("sendCode")
        awaitTag("code")
        rule.onNodeWithTag("code").performTextReplacement("22222")
        tap("submitCode")
        awaitTag("tgPassword")
        rule.onNodeWithTag("tgPassword").performTextReplacement("s3cret")
        tap("submitPassword")
        awaitTag("account:+1 555 010 0100")
        assertEquals(
            listOf(listOf("begin", "+1 555 010 0100"), listOf("code", "22222"), listOf("password", "s3cret")),
            core.loginCalls.toList(),
        )
    }

    @Test
    fun aWrongTelegramCodeIsExplainedAndTheNumberCanBeChanged() {
        launch()
        tap("integrations")
        tap("service:telegram")
        awaitTag("phone")
        rule.onNodeWithTag("phone").performTextReplacement("+15550100")
        tap("sendCode")
        awaitTag("code")
        rule.onNodeWithTag("code").performTextReplacement("00000")
        tap("submitCode")
        awaitText("That code is wrong or has expired. Ask for a new one.")
        tap("restartLogin")
        awaitTag("phone")
    }

    @Test
    fun aGithubTokenIsSentOnceAndTheFieldIsEmptiedAtOnce() {
        launch()
        tap("integrations")
        tap("service:github")
        awaitTag("openGithub")
        awaitTag("openGithubClassic")
        awaitText("Chosen repositories")
        awaitText("Everything")
        tap("pasteManually")
        awaitTag("secret")
        rule.onNodeWithTag("connectSecret").assertIsNotEnabled()
        rule.onNodeWithTag("secret").performTextReplacement("ghp_secret")
        tap("connectSecret")
        awaitTag("account:octo-cat")
        assertEquals(listOf("github" to "ghp_secret"), core.tokensAdded.toList())
    }

    @Test
    fun theVaultIsUnlockedWithTheMasterPasswordAndOnlyOnce() {
        launch()
        tap("integrations")
        tap("service:vault")
        awaitTag("secret")
        rule.onNodeWithTag("secret").performTextReplacement("correct horse")
        tap("connectSecret")
        awaitTag("account:me@example.com")
        assertEquals(listOf("vault" to "correct horse"), core.tokensAdded.toList())
        rule.onAllNodes(hasTestTag("secret")).assertCountEquals(0)
    }

    @Test
    fun theFailureOfAServiceIsShownNotSwallowed() {
        core.serviceFailure = CoreException.Invalid("Wrong password")
        launch()
        tap("integrations")
        tap("service:github")
        tap("pasteManually")
        awaitTag("secret")
        rule.onNodeWithTag("secret").performTextReplacement("ghp_bad")
        tap("connectSecret")
        awaitTag("accountError")
        awaitText("Wrong password")
    }

    @Test
    fun thePhonesOwnServicesConnectOnceAndroidAllowedThem() {
        launch()
        tap("integrations")
        tap("service:device_contacts")
        awaitTag("allowDevice")
        val container = (context.applicationContext as ReinsApp).container
        kotlinx.coroutines.runBlocking { dev.reins.android.ui.services.ServiceViewModel(container, "device_contacts").addDevice() }
        awaitCore { core.serviceAdded.isNotEmpty() }
        assertEquals(listOf("device_contacts" to ""), core.serviceAdded.toList())
        awaitTag("account:this phone")
        rule.onAllNodes(hasTestTag("allowDevice")).assertCountEquals(0)
    }

    @Test
    fun aServiceAccountIsRemovedAfterConfirming() {
        core.accounts = core.accounts + AccountView("telegram", "+15550100", 1_700_000_000)
        launch()
        tap("integrations")
        tap("service:telegram")
        awaitTag("account:+15550100")
        tap("removeAccount:+15550100")
        awaitText("Remove +15550100?")
        rule.onNodeWithText("Remove").performClick()
        awaitCore { core.serviceRemoved.isNotEmpty() }
        assertEquals(listOf("telegram" to "+15550100"), core.serviceRemoved.toList())
        awaitGone("account:+15550100")
    }

    @Test
    fun aChatReadListsTheMessagesWithACodeLeftUntickedAndFlagged() {
        core.pending = listOf(TestData.pending("req7", "read", 3u, service = "telegram", account = "+15550100", op = "read", opTitle = "Read Telegram messages"))
        core.approval = TestData.fetchView()
        launch()
        openItem("req7")
        rule.onNodeWithTag("what").assertTextContains("read Telegram messages", substring = true)
        awaitTag("check:100:2")
        awaitTag("sensitive:100:9")
        rule.onNodeWithTag("approve").assertTextContains("Share 2")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val (id, choice) = core.approvals.single()
        assertEquals("req7", id)
        assertEquals(listOf("100:2", "100:1"), choice.selectedMessageIds)
        assertNull(choice.standing)
    }

    @Test
    fun aCodeCanBeSharedOnlyByTickingItYourself() {
        core.pending = listOf(TestData.pending("req7", "read", 3u, service = "telegram", account = "+15550100", op = "read", opTitle = "Read Telegram messages"))
        core.approval = TestData.fetchView()
        launch()
        openItem("req7")
        tap("check:100:9")
        tap("check:100:1")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("100:2", "100:9"), core.approvals.single().second.selectedMessageIds)
    }

    @Test
    fun aChatReadCanBeRememberedForThatChatOnly() {
        core.pending = listOf(TestData.pending("req7", "read", 3u, service = "telegram", account = "+15550100", op = "read", opTitle = "Read Telegram messages"))
        core.approval = TestData.fetchView()
        launch()
        openItem("req7")
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
        tap("resource:100")
        tap("resource:100")
        tap("rememberConfirm")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(listOf("100"), standing.scope.resources)
        assertEquals(3_600uL, standing.durationSecs)
    }

    @Test
    fun aPasswordRequestSaysItIsAskedEveryTimeAndOffersNothingToRemember() {
        core.pending = listOf(TestData.pending("req8", "read", 1u, service = "vault", account = "me@example.com", op = "get", opTitle = "Get a login from the vault"))
        core.approval = TestData.vaultView()
        launch()
        openItem("req8")
        rule.onNodeWithTag("what").assertTextContains("get a login from the vault", substring = true)
        awaitText("Password for GitHub")
        awaitTag("sensitive:git:password")
        awaitTag("noStanding")
        rule.onAllNodes(hasTestTag("lifetime:HOUR")).assertCountEquals(0)
        tap("check:git:password")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val choice = core.approvals.single().second
        assertEquals(listOf("git:password"), choice.selectedMessageIds)
        assertNull(choice.standing)
    }

    @Test
    fun aMessageToSendIsShownInFullBeforeAnythingIsSent() {
        core.pending = listOf(TestData.pending("req9", "send", 1u, service = "telegram", account = "+15550100", op = "send", opTitle = "Send a Telegram message"))
        core.approval = TestData.writeView()
        launch()
        openItem("req9")
        rule.onNodeWithTag("what").assertTextContains("send a Telegram message", substring = true).assertTextContains("Family", substring = true)
        rule.onNodeWithTag("approve").assertTextContains("Approve")
        awaitTag("writePreview")
        rule.onNodeWithTag("previewLine:0").assertTextContains("Send to Family")
        rule.onNodeWithTag("previewLine:1").assertTextContains("Dinner at eight works")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertTrue(core.approvals.single().second.selectedMessageIds.isEmpty())
    }

    private fun openRepoWrite(view: dev.reins.core.ApprovalView = TestData.repoWriteView()) {
        core.pending = listOf(TestData.pending(view.requestId, "write", 1u, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle))
        core.approval = view
        launch()
        openItem(view.requestId)
    }

    @Test
    fun aChangeToARepositoryIsNamedByTheCoreAndOffersTheKindsOfChangeToRemember() {
        openRepoWrite()
        rule.onNodeWithTag("what").assertTextContains("commit a file to GitHub", substring = true)
        awaitTag("writePreview")
        rule.onNodeWithTag("previewLine:0").assertTextContains("Commit README.md to main of octo/app")
        assertFalse(has("onceWarning"))
        assertFalse("the kinds of change are under Custom", has("classes"))
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
        awaitTag("classes")
        tap("class:issues")
        tap("rememberConfirm")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(setOf("code", "issues"), standing.scope.classes.toSet())
        assertEquals(listOf("octo/app@main"), standing.scope.resources)
    }

    @Test
    fun theLastKindOfChangeStaysTickedAndIsSentAlone() {
        openRepoWrite()
        tap("lifetime:DAY")
        tap("lifetimeCustom")
        tap("moreToggle")
        tap("class:code")
        tap("class:code")
        tap("rememberConfirm")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("code"), core.approvals.single().second.standing!!.scope.classes)
    }

    @Test
    fun aWiderPermissionIsOfferedBelowAndReplacesTheNarrowerOne() {
        openRepoWrite()
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
        awaitTag("widerCaption")
        awaitTag("resource:octo/app@main")
        rule.onNodeWithTag("resource:octo/app@main").assertIsOn()
        rule.onNodeWithTag("resource:octo/app").assertIsOff()
        rule.onNodeWithTag("resource:octo").assertIsOff()
        tap("resource:octo/app")
        rule.onNodeWithTag("resource:octo/app").assertIsOn()
        rule.onNodeWithTag("resource:octo/app@main").assertIsOff()
        tap("resource:octo")
        rule.onNodeWithTag("resource:octo").assertIsOn()
        rule.onNodeWithTag("resource:octo/app").assertIsOff()
        tap("rememberConfirm")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("octo"), core.approvals.single().second.standing!!.scope.resources)
    }

    @Test
    fun aChangeThatIsAskedForEveryTimeShowsAWarningAndOffersNothingToRemember() {
        openRepoWrite(TestData.onceOnlyWriteView())
        rule.onNodeWithTag("what").assertTextContains("delete", substring = true).assertTextContains("octo/app", substring = true)
        awaitTag("onceWarning")
        tap("help:onceWarning")
        awaitTextContaining("Removes it for good")
        awaitTag("noStanding")
        rule.onNodeWithTag("approve").assertTextContains("Hold to delete")
        rule.onAllNodes(hasTestTag("lifetime:HOUR")).assertCountEquals(0)
        rule.onAllNodes(hasTestTag("classes")).assertCountEquals(0)
        // Let go too soon: nothing happens.
        rule.onNodeWithTag("approve").performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(600)
        rule.onNodeWithTag("approve").performTouchInput { up() }
        settle()
        assertTrue(core.approvals.isEmpty())
        assertEquals(0, prompts.get())
        holdApprove()
        awaitCore { core.approvals.isNotEmpty() }
        val choice = core.approvals.single().second
        assertNull(choice.standing)
        assertTrue(choice.selectedMessageIds.isEmpty())
    }

    @Test
    fun aReadOfARepositoryCanBeRememberedForAWiderThingButHasNoKindsOfChange() {
        core.pending = listOf(TestData.pending("req12", "read", 1u, service = "github", account = "octo-cat", op = "contents_get", opTitle = "Read a file from GitHub"))
        core.approval = TestData.repoReadView()
        launch()
        openItem("req12")
        rule.onNodeWithTag("what").assertTextContains("read a file from GitHub", substring = true)
        tap("lifetime:HOUR")
        tap("lifetimeCustom")
        tap("moreToggle")
        awaitTag("allMail:HOUR")
        awaitTag("widerCaption")
        assertFalse(has("classes"))
        tap("resource:octo/app")
        tap("rememberConfirm")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(listOf("octo/app"), standing.scope.resources)
        assertTrue(standing.scope.classes.isEmpty())
    }

    @Test
    fun aWaitingRequestIsNamedByTheCore() {
        core.pending = listOf(TestData.pending("req10", "write", 1u, service = "github", account = "octo-cat", op = "file_put", opTitle = "Commit a file to GitHub"))
        launch()
        awaitTag("pending:req10")
        rule.onNodeWithTag("sentence:req10", useUnmergedTree = true).assertTextContains("Wants to commit a file to GitHub", substring = true)
    }

    @Test
    fun anActivityEntryIsNamedByTheCore() {
        core.activity = listOf(
            TestData.entry(5, "write", "sent", count = 1u, opTitle = "Delete a GitHub repository")
                .copy(service = "github", account = "octo-cat", op = "repo_delete"),
        )
        launch()
        awaitTag("entry:5")
        rule.onNodeWithTag("entryTitle:5", useUnmergedTree = true).assertTextEquals("Deleted a GitHub repository")
        tap("entry:5")
        awaitTag("detailTitle")
        rule.onNodeWithTag("detailTitle").assertTextContains("Deleted a GitHub repository")
    }

    @Test
    fun whatWasSharedFromAnotherServiceCanBeReadInTheActivity() {
        core.activity = listOf(
            TestData.entry(
                4, "read", count = 2u,
                info = TestData.info(null, TestData.sharedTexts("Dinner at eight?", "I am late"), null, null, null),
            ).copy(service = "telegram", account = "+15550100", op = "read", opTitle = "Read Telegram messages"),
        )
        launch()
        awaitTag("entry:4")
        tap("entry:4")
        awaitTag("detailText:0")
        rule.onNodeWithTag("detailText:0").assertTextContains("Dinner at eight?")
        rule.onNodeWithTag("detailText:1").assertTextContains("I am late")
        rule.onNodeWithTag("detailTitle").assertTextContains("Read Telegram messages")
    }

    // ---- settings -------------------------------------------------------------------------------------------------

    @Test
    fun settingsHoldsConnectionsIntegrationsAndTheApprovalDeviceButton() {
        launch()
        tap("tabSettings")
        awaitTag("registerPhone")
        rule.onNodeWithText("Use this phone for approvals").assertExists()
        rule.onNodeWithTag("connection:c1").assertExists()
        rule.onNodeWithTag("openIntegrations").assertExists()
        rule.onNodeWithText("1 account").assertExists()
    }

    @Test
    fun onceThisPhoneIsTheApprovalDeviceTheCardSaysSoAndOffersNoTakeover() {
        launch()
        awaitTag("noActivity")
        (context.applicationContext as ReinsApp).container.state.setApprovalDevice(true)
        tap("tabSettings")
        awaitTag("approvalChip")
        rule.onNodeWithTag("approvalChip").assertTextContains("Approval phone", substring = true)
        assertFalse(has("registerPhone"))
    }

    @Test
    fun aConnectionsIconCanBePickedAndItCanBeDisconnected() {
        launch()
        tap("tabSettings")
        tap("connection:c1")
        tap("connectionIcon")
        tap("icon:grok")
        awaitCore { core.icons["c1"] == "grok" }
        tap("connectionIcon")
        tap("icon:auto")
        awaitCore { core.icons["c1"] == "auto" }
        // Held long enough: disconnected, no dialog.
        hold("disconnect")
        awaitCore { core.revokedConnections.isNotEmpty() }
        assertEquals(listOf("c1"), core.revokedConnections.toList())
    }

    @Test
    fun aChosenIconShowsEverywhereTheConnectionAppears() {
        core.activity = listOf(TestData.entry(3))
        core.grants = listOf(TestData.grant("g1"))
        core.pending = listOf(TestData.pending("req1", "search", 2u, waitUntil = null))
        Foreground.autoPopup = false
        launch()
        awaitTag("entry:3")
        fun shown() = rule.onAllNodes(androidx.compose.ui.test.hasContentDescription("Grok"), useUnmergedTree = true).fetchSemanticsNodes().size
        assertEquals(0, shown())
        tap("tabSettings")
        tap("connection:c1")
        tap("connectionIcon")
        tap("icon:grok")
        awaitCore { core.icons["c1"] == "grok" }
        rule.onNodeWithContentDescription("Back").performClick()
        tap("tabActivity")
        awaitTag("entry:3")
        settle()
        val onActivity = shown()
        assertTrue("the activity list and the waiting request use the chosen icon, saw $onActivity", onActivity >= 2)
        tap("tabGrants")
        awaitTag("grant:g1")
        assertTrue(shown() >= 1)
    }

    @Test
    fun signingOutReturnsToSignIn() {
        core.browserLogoutUrl = "https://api.workos.com/user_management/sessions/logout?session_id=session_test"
        launch()
        tap("tabSettings")
        tap("signOut")
        rule.onAllNodes(hasText("Sign out")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitTag("welcome")
        awaitTag("continue")
        // The screen signs out at once; the browser's logout page opens when the core has finished in the background.
        awaitCore { shadowOf(context as android.app.Application).peekNextStartedActivity() != null }
        val opened = shadowOf(context as android.app.Application).nextStartedActivity
        assertEquals(core.browserLogoutUrl, opened?.data?.toString())
        assertNull(core.session)
        core.browserLogoutUrl = null
    }

    // ---- misc ----------------------------------------------------------------------------------------------------

    @Test
    fun approvalScreenIsSecureExactlyWhenTheBuildSaysSo() {
        // Off in debug builds (these tests, unless -Preins.secureScreens=true), on in release builds.
        searchRequest()
        launch()
        openItem("req1")
        assertEquals(BuildConfig.SECURE_SCREENS, secure())
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
