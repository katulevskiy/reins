package dev.reins.android

import android.content.Context
import android.content.Intent
import android.view.WindowManager
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
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
        assertTrue(rule.onAllNodes(hasText("Claude: Search Gmail", substring = true)).fetchSemanticsNodes().size >= 2)
        rule.onNodeWithText("Claude: Send email", substring = true).assertExists()
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
        rule.onNodeWithText("See Gmail accounts").assertExists()
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
        rule.onNodeWithText("Already shared").assertExists()
        rule.onNodeWithTag("acct:a@gmail.com").assertIsNotEnabled()
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("only the new one is a pick", listOf("b@gmail.com"), core.approvals.single().second.selectedMessageIds)
    }

    @Test
    fun anAccountsRequestCanBeAllowedJustOnceFromMoreOptions() {
        core.pending = listOf(TestData.pending("req5", "accounts", 2u, waitUntil = 1_700_000_245))
        core.approval = TestData.accountsView()
        launch()
        openItem("req5")
        tap("moreToggle")
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
        awaitTag("sharedAccount:1")
        rule.onNodeWithText("work@corp.example").assertExists()
        rule.onNodeWithText("See Gmail accounts").assertExists()
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
    fun theSheetShowsOnlyApproveAndDenyUntilMoreIsOpened() {
        searchRequest()
        launch()
        openItem("req1")
        awaitTag("moreToggle")
        assertFalse(has("lifetime:HOUR"))
        assertFalse(has("allMail:HOUR"))
        tap("moreToggle")
        awaitTag("lifetime:HOUR")
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
    fun aStandingHourlyGrantIsUnderMoreOptions() {
        searchRequest()
        launch()
        openItem("req1")
        tap("moreToggle")
        tap("lifetime:HOUR")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(3_600UL, standing.durationSecs)
        assertTrue(standing.scope.selectedMessagesOnly)
    }

    @Test
    fun allowingAllMailForADayIsOneChipInMore() {
        searchRequest()
        launch()
        openItem("req1")
        tap("moreToggle")
        tap("allMail:DAY")
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
        tap("moreToggle")
        tap("lifetime:DAY")
        tap("similar")
        rule.onNodeWithText("Anyone at @gmail.com").performScrollTo().performClick()
        awaitTextContaining("covers millions")
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
        tap("moreToggle")
        tap("lifetime:HOUR")
        tap("domain:ann@corp.com")
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
        awaitText("PERMISSION REQUEST")
        rule.onNodeWithText("Allow").assertExists()
        tap("moreToggle")
        awaitTag("grantSlider")
        rule.onNodeWithTag("grantSlider").performSemanticsAction(SemanticsActions.SetProgress) { it(0f) }
        settle()
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
        awaitTextContaining("40 s left")
    }

    @Test
    fun afterTheWaitEndsTheSheetSaysApprovingStillWorks() {
        core.pending = listOf(TestData.pending(waitUntil = 1_700_000_050))
        core.approval = TestData.searchView(waitUntil = 1_700_000_050)
        launch()
        openItem("req1")
        awaitTag("lateBanner")
        awaitTextContaining("stopped waiting")
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
        rule.onNodeWithTag("what").assertTextContains("Push with git")
        rule.onNodeWithTag("gitRepo").assertTextContains("octo/app")
        rule.onNodeWithTag("gitPack").assertTextContains("12.4 KB", substring = true)
        rule.onNodeWithTag("gitRef:0").assertTextContains("main")
        rule.onNodeWithTag("gitChip:0").assertTextContains("Update")
        // The git section replaces the core's plain lines.
        assertFalse(has("writePreview"))
        assertFalse(has("forceWarning:0"))
        assertFalse(has("forceUnknown:0"))
        assertFalse(has("onceWarning"))

        // Commits: newest five, then the rest of those listed on request, and how many the app did not list.
        rule.onNodeWithTag("gitCommit:0:0").assertTextContains("a1b2c01").assertTextContains("Commit number 1")
            .assertTextContains("Ada Lovelace <ada@example.com>")
        assertEquals(5, count("gitCommit:0:"))
        rule.onNodeWithTag("gitMoreCommits:0").assertTextContains("and 2 more")
        rule.onNodeWithTag("gitCommitsNotListed:0").assertTextContains("2 more not listed")
        tap("gitMoreCommits:0")
        awaitTag("gitCommit:0:6")
        assertEquals(7, count("gitCommit:0:"))
        assertFalse(has("gitMoreCommits:0"))

        // Files: totals, the first eight, then the rest.
        rule.onNodeWithTag("gitTotals:0").assertTextContains("+120 −14 in 12 files")
        rule.onNodeWithTag("gitFile:0:0").assertTextContains("M").assertTextContains("src/module1/File1.kt")
            .assertTextContains("+3 −1")
        assertEquals(8, count("gitFile:0:"))
        rule.onNodeWithTag("gitMoreFiles:0").assertTextContains("and 2 more")
        rule.onNodeWithTag("gitFilesNotListed:0").assertTextContains("2 more not listed")
        tap("gitMoreFiles:0")
        awaitTag("gitFile:0:9")
        assertEquals(10, count("gitFile:0:"))

        rule.onNodeWithTag("gitNote:0").assertTextContains("Line counts are approximate")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals("req20", core.approvals.single().first)
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun aForcePushWarnsThatItRewritesHistoryAndIsAskedEveryTime() {
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)), noStanding = true))
        rule.onNodeWithTag("forceWarning:0").assertTextContains("Rewrites history (force push)")
        assertFalse(has("forceUnknown:0"))
        awaitTag("onceWarning")
        tap("moreToggle")
        awaitTag("noStanding")
        rule.onAllNodes(hasTestTag("lifetime:HOUR")).assertCountEquals(0)
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertNull(core.approvals.single().second.standing)
    }

    @Test
    fun aPushWhoseHistoryCouldNotBeCheckedIsTreatedAsAForcePush() {
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true, forceUnknown = true)), noStanding = true))
        rule.onNodeWithTag("forceUnknown:0").assertTextContains("Could not check history — treat as a force push")
        assertFalse(has("forceWarning:0"))
    }

    @Test
    fun aTagPushShowsATagChipAndANewBranchItsOwn() {
        val tag = TestData.gitRef("v1.2", kind = "tag", change = "create", commitCount = 0u, commits = emptyList(), filesChanged = 0u, files = emptyList(), additions = null, deletions = null)
        val branch = TestData.gitRef("feature/login", change = "create", commitCount = 2u, commits = (1..2).map(TestData::gitCommit), filesChanged = 1u, files = listOf(TestData.gitFile("README.md", "added", 4u, 0u)), additions = 4u, deletions = 0u)
        openGitPush(TestData.gitPushView(TestData.gitPush(tag, branch), tags = true))
        rule.onNodeWithTag("what").assertTextContains("Push tags with git")
        rule.onNodeWithTag("gitRef:0").assertTextContains("v1.2")
        rule.onNodeWithTag("gitChip:0").assertTextContains("New tag")
        assertFalse(has("gitTotals:0"))
        rule.onNodeWithTag("gitChip:1").assertTextContains("New branch")
        rule.onNodeWithTag("gitFile:1:0").assertTextContains("A")
        assertFalse(has("gitMoreCommits:1"))
        assertFalse(has("gitCommitsNotListed:1"))
    }

    @Test
    fun lineCountsAreLeftOutWhenTheyCouldNotBeCounted() {
        val files = listOf(
            TestData.gitFile("assets/logo.png", "added", null, null, binary = true),
            TestData.gitFile("data/huge.json", "modified", null, null),
        )
        openGitPush(TestData.gitPushView(TestData.gitPush(TestData.gitRef(filesChanged = 2u, files = files, additions = null, deletions = null))))
        rule.onNodeWithTag("gitTotals:0").assertTextContains("2 files changed")
        rule.onNodeWithTag("gitFile:0:0").assertTextContains("binary")
        assertFalse(has("gitMoreFiles:0"))
        assertFalse(has("gitFilesNotListed:0"))
    }

    @Test
    fun aGitFetchIsAnOrdinaryItemList() {
        val view = TestData.gitFetchView()
        core.pending = listOf(TestData.pending(view.requestId, "read", 1u, label = view.connectionLabel, service = "github", account = "octo-cat", op = view.op, opTitle = view.opTitle))
        core.approval = view
        launch()
        openItem(view.requestId)
        rule.onNodeWithTag("what").assertTextContains("Clone and fetch with git")
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
        awaitTextContaining("Reins desktop app on laptop: Clone and fetch with git")
        awaitTextContaining("Reins desktop app on laptop: Push with git")
        tap("entry:7")
        awaitTag("detailTitle")
        rule.onNodeWithTag("detailTitle").assertTextContains("Push with git")
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
        rule.onNodeWithText("Used 4 times").assertExists()
        tap("grant:g1")
        awaitTag("grantTitle")
        rule.onNodeWithText("Read emails from @bank.com").assertExists()
        rule.onNodeWithText("From @bank.com").assertExists()
        rule.onNodeWithText("Used 4 times").assertExists()
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
        rule.onNodeWithText("Expired · 3").assertExists()
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
        awaitTextContaining("can only come back as it was")
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
        tap("openSettings")
        tap("connection:c1")
        tap("icon:grok")
        awaitCore { core.icons["c1"] == "grok" }
        rule.onNodeWithContentDescription("Back").performClick()
        rule.onNodeWithContentDescription("Back").performClick()
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
        rule.onAllNodes(hasText("Delete")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitCore { core.revokedGrants.isNotEmpty() }
        assertEquals(listOf("g1"), core.revokedGrants.toList())
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
        awaitText("Fine-grained token (pick repositories)")
        awaitText("Classic token (everything, incl. gists and notifications)")
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
        rule.onNodeWithTag("what").assertTextContains("Read Telegram messages")
        awaitTag("check:100:2")
        awaitText("Looks like a code or a password")
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
        tap("moreToggle")
        tap("lifetime:HOUR")
        awaitTag("resource:100")
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
        rule.onNodeWithTag("what").assertTextContains("Get a login from the vault")
        awaitText("Password for GitHub")
        tap("moreToggle")
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
        rule.onNodeWithTag("what").assertTextContains("Send a Telegram message")
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
        rule.onNodeWithTag("what").assertTextContains("Commit a file to GitHub")
        awaitTag("writePreview")
        rule.onNodeWithTag("previewLine:0").assertTextContains("Commit README.md to main of octo/app")
        assertFalse(has("onceWarning"))
        tap("moreToggle")
        // Nothing to choose while it is only this once.
        assertFalse(has("classes"))
        tap("lifetime:HOUR")
        awaitTag("classes")
        awaitText("Allow these kinds of change")
        tap("class:issues")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        val standing = core.approvals.single().second.standing!!
        assertEquals(setOf("code", "issues"), standing.scope.classes.toSet())
        assertEquals(listOf("octo/app@main"), standing.scope.resources)
    }

    @Test
    fun theLastKindOfChangeStaysTickedAndIsSentAlone() {
        openRepoWrite()
        tap("moreToggle")
        tap("lifetime:DAY")
        tap("class:code")
        tap("class:code")
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("code"), core.approvals.single().second.standing!!.scope.classes)
    }

    @Test
    fun aWiderPermissionIsOfferedBelowAndReplacesTheNarrowerOne() {
        openRepoWrite()
        tap("moreToggle")
        tap("lifetime:HOUR")
        awaitTag("widerCaption")
        awaitText("Or a wider permission")
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
        tap("approve")
        awaitCore { core.approvals.isNotEmpty() }
        assertEquals(listOf("octo"), core.approvals.single().second.standing!!.scope.resources)
    }

    @Test
    fun aChangeThatIsAskedForEveryTimeShowsAWarningAndOffersNothingToRemember() {
        openRepoWrite(TestData.onceOnlyWriteView())
        rule.onNodeWithTag("what").assertTextContains("Delete a GitHub repository")
        awaitTag("onceWarning")
        awaitTextContaining("cannot be undone")
        tap("moreToggle")
        awaitTag("noStanding")
        rule.onAllNodes(hasTestTag("lifetime:HOUR")).assertCountEquals(0)
        rule.onAllNodes(hasTestTag("classes")).assertCountEquals(0)
        tap("approve")
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
        rule.onNodeWithTag("what").assertTextContains("Read a file from GitHub")
        tap("moreToggle")
        awaitTag("allMail:HOUR")
        tap("lifetime:HOUR")
        awaitTag("widerCaption")
        assertFalse(has("classes"))
        tap("resource:octo/app")
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
        awaitTextContaining("Claude: Commit a file to GitHub")
    }

    @Test
    fun anActivityEntryIsNamedByTheCore() {
        core.activity = listOf(
            TestData.entry(5, "write", "sent", count = 1u, opTitle = "Delete a GitHub repository")
                .copy(service = "github", account = "octo-cat", op = "repo_delete"),
        )
        launch()
        awaitTextContaining("Claude: Delete a GitHub repository")
        tap("entry:5")
        awaitTag("detailTitle")
        rule.onNodeWithTag("detailTitle").assertTextContains("Delete a GitHub repository")
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
        tap("openSettings")
        awaitTag("registerPhone")
        rule.onNodeWithText("Use this phone for approvals").assertExists()
        rule.onNodeWithTag("connection:c1").assertExists()
        rule.onNodeWithTag("openIntegrations").assertExists()
        rule.onNodeWithText("1 account connected").assertExists()
    }

    @Test
    fun onceThisPhoneIsTheApprovalDeviceTheButtonIsGrayAndSaysSo() {
        launch()
        awaitTag("noActivity")
        (context.applicationContext as ReinsApp).container.state.setApprovalDevice(true)
        tap("openSettings")
        awaitTag("registerPhone")
        rule.onNodeWithText("This phone is used for approvals").assertExists()
        rule.onNodeWithTag("registerPhone").assertIsNotEnabled()
    }

    @Test
    fun aConnectionsIconCanBePickedAndItCanBeDisconnected() {
        launch()
        tap("openSettings")
        tap("connection:c1")
        tap("icon:grok")
        awaitCore { core.icons["c1"] == "grok" }
        tap("icon:auto")
        awaitCore { core.icons["c1"] == "auto" }
        tap("disconnect")
        rule.onAllNodes(hasText("Disconnect")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
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
        tap("openSettings")
        tap("connection:c1")
        tap("icon:grok")
        awaitCore { core.icons["c1"] == "grok" }
        rule.onNodeWithContentDescription("Back").performClick()
        rule.onNodeWithContentDescription("Back").performClick()
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
        tap("openSettings")
        tap("signOut")
        rule.onAllNodes(hasText("Sign out")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitTag("welcome")
        awaitTag("continue")
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
