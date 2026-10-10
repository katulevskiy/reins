package dev.reins.android

import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.GmsQrScanner
import dev.reins.android.platform.QrScanner
import dev.reins.android.platform.QrScannerProvider
import dev.reins.android.platform.ScanResult
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.CoreException
import java.util.concurrent.atomic.AtomicInteger
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Onboarding (welcome, new account, sign-in, the setup after it) and connecting a computer by its code or link. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class OnboardingFlowTest : FlowHarness() {
    private val password = "correct horse battery staple"
    private val expired = "Code expired. Show a new one on your computer."
    /** The password forms are only offered for a server people run themselves. */
    private val server = "https://reins.example.com"

    /** What the next scan returns. */
    @Volatile private var scan: ScanResult = ScanResult.Cancelled
    private val scans = AtomicInteger()

    @Before
    fun resetOnboarding() {
        dev.reins.android.TestNativeKeys.install()
        core.resetOnboarding()
        core.logins.clear()
        core.registrations.clear()
        core.pairingAnswers.clear()
        scan = ScanResult.Cancelled
        scans.set(0)
        QrScannerProvider.factory = {
            QrScanner {
                scans.incrementAndGet()
                scan
            }
        }
    }

    @After
    fun restoreScanner() {
        QrScannerProvider.factory = { GmsQrScanner(it) }
    }

    private fun viewLink(uri: String) = Intent(Intent.ACTION_VIEW, Uri.parse(uri)).setClass(context, MainActivity::class.java)

    /** "Use another server" on the welcome page, with [server] typed in. */
    private fun useOwnServer() {
        // After a sign-out the welcome page already shows the last server's address.
        if (has("useAnotherServer")) tap("useAnotherServer")
        awaitTag("server")
        rule.onNodeWithTag("server").performTextReplacement(server)
    }

    private fun fillCreateForm(email: String = "new@example.com") {
        useOwnServer()
        tap("createAccount")
        awaitTag("create")
        rule.onNodeWithTag("email").performTextReplacement(email)
        rule.onNodeWithTag("password").performTextReplacement(password)
        rule.onNodeWithTag("confirmPassword").performTextReplacement(password)
        tap("acceptTerms")
    }

    private fun signInFromWelcome(email: String = "me@example.com") {
        useOwnServer()
        tap("startSignIn")
        awaitTag("signIn")
        rule.onNodeWithTag("email").performTextReplacement(email)
        rule.onNodeWithTag("password").performTextReplacement(password)
        tap("signIn")
    }

    private fun openConnectComputer() {
        launch()
        tap("openSettings")
        tap("connectComputer")
        awaitTag("scanQr")
    }

    // ---- welcome, new account, sign-in -------------------------------------------------------------------------------

    @Test
    fun signedOutTheAppWelcomesWithContinue() {
        core.session = null
        launch()
        awaitTag("welcome")
        rule.onNodeWithTag("continue").assertIsEnabled()
        rule.onNodeWithTag("continue").assertTextEquals("Continue")
        rule.onNodeWithTag("serverName").assertTextContains(AccountRules.displayHost(BuildConfig.DEFAULT_SERVER), substring = true)
        assertFalse(has("createAccount"))
        assertFalse(has("startSignIn"))
        assertFalse(has("server"))
    }

    @Test
    fun thePasswordFormsAreOnlyOfferedForAnotherServer() {
        core.session = null
        launch()
        tap("useAnotherServer")
        awaitTag("server")
        // "https://" alone is no address yet.
        rule.onNodeWithTag("continue").assertIsNotEnabled()
        assertFalse(has("startSignIn"))
        rule.onNodeWithTag("server").performTextReplacement("reins.example.com")
        rule.onNodeWithTag("continue").assertIsEnabled()
        awaitTag("startSignIn")
        assertTrue(has("createAccount"))
        // Typing the hosted server's address does not bring them back for it.
        rule.onNodeWithTag("server").performTextReplacement(BuildConfig.DEFAULT_SERVER)
        awaitGone("startSignIn")
        assertFalse(has("createAccount"))
        tap("defaultServer")
        awaitGone("server")
        assertFalse(has("startSignIn"))
    }

    @Test
    fun notNowOnTheSetupsNotificationsIsAnAnswerActivityDoesNotAskAgain() {
        shadowOf(context as android.app.Application).denyPermissions(android.Manifest.permission.POST_NOTIFICATIONS)
        core.session = null
        launch()
        signInFromWelcome()
        setupTo("setupNotifications")
        assertFalse(dev.reins.android.platform.NotificationAccess.asked(context))
        tap("notificationsLater")
        awaitTag("setupIntegrations")
        assertTrue(dev.reins.android.platform.NotificationAccess.asked(context))
    }

    @Test
    fun theSetupSaysBeforeTheFirstRequestThatApprovingNeedsAScreenLock() {
        dev.reins.android.platform.ScreenLock.check = { false }
        core.session = null
        launch()
        signInFromWelcome()
        setupTo("setupNotifications")
        awaitTag("screenLockOff")
    }

    @Test
    fun aServerWithoutBrowserSignInOffersOnlyTheMasterPassword() {
        core.session = null
        core.serverInfos[server] = dev.reins.core.ServerInfo(false, false, false)
        launch()
        useOwnServer()
        // Asked once the address settles: no "Continue", which would end on an error page there.
        awaitGone("continue")
        awaitText("Email and master password")
        assertTrue(has("startSignIn") && has("createAccount"))
        // A server that says nothing (an older one) keeps every way in.
        rule.onNodeWithTag("server").performTextReplacement("https://old.example.com")
        awaitTag("continue")
        assertTrue(has("startSignIn"))
    }

    @Test
    fun afterSigningOutTheWelcomePageOffersTheSameServerAgain() {
        core.session = null
        container.onboarding.lastServer = server
        launch()
        awaitTag("server")
        rule.onNodeWithTag("server").assertTextContains(server)
        awaitTag("startSignIn")
    }

    @Test
    fun creatingAnAccountChecksTheFormThenRegistersThePhone() {
        core.session = null
        launch()
        useOwnServer()
        tap("createAccount")
        awaitTag("create")
        rule.onNodeWithTag("create").assertIsNotEnabled()
        rule.onNodeWithTag("serverName").assertTextContains("reins.example.com", substring = true)
        assertTrue(has("noRecovery"))
        assertTrue(showsText("Nobody can reset your master password", substring = true))

        rule.onNodeWithTag("email").performTextReplacement("new@example.com")
        rule.onNodeWithTag("password").performTextReplacement("too short")
        awaitTag("strengthLabel")
        rule.onNodeWithTag("strengthLabel").assertTextEquals("Weak")
        rule.onNodeWithTag("password").performTextReplacement(password)
        rule.onNodeWithTag("strengthLabel").assertTextEquals("Strong")

        rule.onNodeWithTag("confirmPassword").performTextReplacement("correct horse battery")
        awaitTag("mismatch")
        rule.onNodeWithTag("create").assertIsNotEnabled()
        rule.onNodeWithTag("confirmPassword").performTextReplacement(password)
        awaitGone("mismatch")
        // Everything but the Terms.
        rule.onNodeWithTag("create").assertIsNotEnabled()
        tap("acceptTerms")
        rule.onNodeWithTag("create").assertIsEnabled()
        tap("create")

        awaitTag("setup")
        assertEquals(listOf(server, "new@example.com", password), core.createdAccounts.single())
        assertTrue(core.logins.isEmpty())
        awaitCore { container.state.approvalDevice.value }
        assertEquals(1, core.registrations.size)
    }

    @Test
    fun aRefusedSignUpSaysWhyAndStaysOnTheForm() {
        core.session = null
        core.createAccountError = CoreException.Invalid("An account with this email already exists. Sign in instead.")
        launch()
        fillCreateForm()
        tap("create")
        awaitText("An account with this email already exists. Sign in instead.")
        assertTrue(has("create"))
        assertFalse(has("setup"))
        assertTrue(core.registrations.isEmpty())
    }

    // ---- the setup after signing in ------------------------------------------------------------------------------------

    @Test
    fun theSetupWalksThroughEveryPageAndShowsOnlyOnce() {
        core.session = null
        launch()
        fillCreateForm()
        tap("create")
        awaitTag("setupWelcome")
        assertTrue(has("requestFlow"))
        tap("setupNext")
        awaitTag("setupNotifications")
        if (has("notificationsLater")) tap("notificationsLater") else tap("setupNext")
        awaitTag("setupIntegrations")
        assertTrue(has("setupService:gmail"))
        assertTrue(has("setupService:mcp"))
        tap("setupNext")
        awaitTag("setupRules")
        // Someone who just installed Reins starts with the recommended rule.
        awaitCore { core.startingPolicy == dev.reins.core.StartingPolicy.READS_FOR_A_DAY }
        tap("setupNext")
        awaitTag("setupAutopilot")
        assertTrue(has("modelCard"))
        tap("setupNext")
        awaitTag("setup")
        rule.onNodeWithTag("computerHowTo").assertTextContains("reins login", substring = true)
        assertTrue(has("desktopDownload"))
        tap("setupNext")
        awaitTag("setupAi")
        val mcp = "$server/mcp"
        rule.onNodeWithTag("mcpUrl").assertTextEquals(mcp)
        tap("copyMcp")
        val clip = context.getSystemService(ClipboardManager::class.java).primaryClip
        assertEquals(mcp, clip?.getItemAt(0)?.text?.toString())
        tap("setupNext")
        awaitTag("setupFinished")
        tap("setupDone")
        awaitTag("noActivity")

        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("noActivity")
        assertFalse(has("setup"))
    }

    @Test
    fun anIntegrationOpenedFromTheSetupComesBackToTheSamePage() {
        core.session = null
        launch()
        fillCreateForm()
        tap("create")
        setupTo("setupIntegrations")
        tap("setupService:gmail")
        awaitGone("setup")
        pressBack()
        awaitTag("setupIntegrations")
    }

    @Test
    fun theTourCanBeTakenAgainFromSettings() {
        launch()
        awaitTag("noActivity")
        tap("openSettings")
        tap("takeTour")
        awaitTag("setupWelcome")
        tap("setupSkip")
        awaitTag("noActivity")
    }

    @Test
    fun signingInAgainToTheSameAccountSkipsTheSetup() {
        core.session = null
        launch()
        signInFromWelcome()
        awaitTag("setup")
        tap("setupSkip")
        awaitTag("noActivity")
        tap("openSettings")
        tap("signOut")
        rule.onAllNodes(hasText("Sign out")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitTag("welcome")
        signInFromWelcome()
        awaitTag("noActivity")
        assertFalse(has("setup"))
        assertEquals(2, core.logins.size)
    }

    @Test
    fun peopleWhoWereAlreadySignedInNeverSeeTheSetup() {
        launch()
        awaitTag("noActivity")
        assertFalse(has("setup"))
        assertFalse(has("welcome"))
    }

    @Test
    fun theSetupScansTheComputersCode() {
        core.session = null
        scan = ScanResult.Scanned("reins://pair?code=BCDF-GHJK")
        launch()
        fillCreateForm()
        tap("create")
        setupTo("setupComputer")
        tap("scanQr")
        awaitTag("keyFingerprint")
        assertEquals(1, scans.get())
        assertEquals(listOf("BCDF-GHJK"), core.pairingCodes.toList())
    }

    // ---- connecting a computer -----------------------------------------------------------------------------------------

    @Test
    fun scanningAComputersCodeOpensThePairingSheet() {
        scan = ScanResult.Scanned("https://app.reins2fa.com/pair?code=bcdf-ghjk")
        openConnectComputer()
        tap("scanQr")
        awaitTag("keyFingerprint")
        assertEquals(listOf("BCDF-GHJK"), core.pairingCodes.toList())
        rule.onNodeWithTag("keyFingerprint").assertTextContains("4821 9930", substring = true)
        // The usual sheet: the number the computer shows, then biometrics.
        tap("code:42")
        tap("approve")
        awaitCore { core.pairingAnswers.isNotEmpty() }
        val answer = core.pairingAnswers.single()
        assertEquals("pair-code", answer[0])
        assertEquals(true, answer[1])
        assertEquals(42.toUByte(), answer[2])
        assertEquals(1, prompts.get())
    }

    @Test
    fun anExpiredCodeSaysToShowANewOne() {
        core.pairingByCodeError = CoreException.NotFound()
        scan = ScanResult.Scanned("BCDF-GHJK")
        openConnectComputer()
        tap("scanQr")
        awaitText(expired)
        assertFalse(has("sheet"))
    }

    @Test
    fun aQrCodeThatIsNotAPairingCodeIsRefused() {
        scan = ScanResult.Scanned("https://evil.example.com/login?code=BCDF-GHJK")
        openConnectComputer()
        tap("scanQr")
        awaitText("Not a Reins code. Scan the one your computer shows.")
        assertTrue(core.pairingCodes.isEmpty())
        assertFalse(has("sheet"))
    }

    @Test
    fun withoutTheScannerTheCodeCanBeTyped() {
        scan = ScanResult.Unavailable("module not installed")
        openConnectComputer()
        tap("scanQr")
        awaitTag("pairCode")
        awaitText("No scanner here. Type the code instead.")
        rule.onNodeWithTag("submitCode").assertIsNotEnabled()
        rule.onNodeWithTag("pairCode").performTextReplacement("bcdf ghjk")
        rule.onNodeWithTag("submitCode").assertIsEnabled()
        tap("submitCode")
        awaitTag("keyFingerprint")
        assertEquals(listOf("BCDF-GHJK"), core.pairingCodes.toList())
    }

    @Test
    fun aComputerThatPairedIsConfirmedOnThePage() {
        scan = ScanResult.Unavailable("module not installed")
        openConnectComputer()
        tap("scanQr")
        rule.onNodeWithTag("pairCode").performTextReplacement("bcdf ghjk")
        tap("submitCode")
        awaitTag("keyFingerprint")
        tap("code:42")
        rule.onNodeWithTag("label").performTextReplacement("Work laptop")
        // The server stores the connection when the phone approves.
        core.connections = listOf(TestData.computer(label = "Work laptop"))
        tap("approve")
        awaitCore { core.pairingAnswers.isNotEmpty() }
        // Back on the page that started it: the result, with the name.
        awaitTag("computerConnected")
        awaitText("Work laptop")
        // And Settings lists it at once.
        awaitCore { container.state.connections.value.any { it.id == "d1" } }
    }

    @Test
    fun aCancelledScanDoesNothing() {
        openConnectComputer()
        tap("scanQr")
        awaitCore { scans.get() == 1 }
        assertFalse(has("connectError"))
        assertFalse(has("pairCode"))
        assertTrue(core.pairingCodes.isEmpty())
    }

    // ---- links ---------------------------------------------------------------------------------------------------------

    @Test
    fun anAppLinkOpensThePairingItNames() {
        launch(viewLink("https://app.reins2fa.com/pair?code=BCDF-GHJK"))
        awaitTag("keyFingerprint")
        assertEquals(listOf("BCDF-GHJK"), core.pairingCodes.toList())
    }

    @Test
    fun aLinkOpenedWhileSignedOutIsUsedRightAfterSigningIn() {
        core.session = null
        launch(viewLink("reins://pair?code=bcdf-ghjk"))
        awaitTag("linkWaiting")
        assertTrue(core.pairingCodes.isEmpty())
        signInFromWelcome()
        awaitTag("keyFingerprint")
        assertEquals(listOf("BCDF-GHJK"), core.pairingCodes.toList())
        // Behind the sheet: the setup of a fresh sign-in.
        assertTrue(has("setup"))
    }

    @Test
    fun aLinkWithoutAWellFormedCodeOpensNothing() {
        launch(viewLink("https://app.reins2fa.com/pair?code=AAAA-AAAA"))
        awaitTag("noActivity")
        settle()
        assertTrue(core.pairingCodes.isEmpty())
        assertFalse(has("sheet"))
    }

    @Test
    fun anExpiredLinkSaysSoOnTheMainScreen() {
        core.pairingByCodeError = CoreException.NotFound()
        launch(viewLink("https://app.reins2fa.com/pair?code=BCDF-GHJK"))
        awaitText(expired)
        assertFalse(has("sheet"))
    }
}
