package dev.rewarden.android

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
import dev.rewarden.android.platform.GmsQrScanner
import dev.rewarden.android.platform.QrScanner
import dev.rewarden.android.platform.QrScannerProvider
import dev.rewarden.android.platform.ScanResult
import dev.rewarden.android.ui.signin.AccountRules
import dev.rewarden.core.CoreException
import java.util.concurrent.atomic.AtomicInteger
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Onboarding (welcome, new account, sign-in, the setup after it) and connecting a computer by its code or link. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class OnboardingFlowTest : FlowHarness() {
    private val password = "correct horse battery staple"
    private val expired = "This code has expired or was already used. Show a new one on your computer."
    /** The password forms are only offered for a server people run themselves. */
    private val server = "https://reins.example.com"

    /** What the next scan returns. */
    @Volatile private var scan: ScanResult = ScanResult.Cancelled
    private val scans = AtomicInteger()

    @Before
    fun resetOnboarding() {
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
        tap("useAnotherServer")
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
    fun creatingAnAccountChecksTheFormThenRegistersThePhone() {
        core.session = null
        launch()
        useOwnServer()
        tap("createAccount")
        awaitTag("create")
        rule.onNodeWithTag("create").assertIsNotEnabled()
        rule.onNodeWithTag("serverName").assertTextContains("reins.example.com", substring = true)
        assertTrue(has("noRecovery"))
        assertTrue(showsText("not even Reins", substring = true))

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

        awaitTag("setupComputer")
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
        assertFalse(has("setupComputer"))
        assertTrue(core.registrations.isEmpty())
    }

    // ---- the setup after signing in ------------------------------------------------------------------------------------

    @Test
    fun theSetupShowsTheMcpAddressAndOnlyOnce() {
        core.session = null
        launch()
        fillCreateForm()
        tap("create")
        awaitTag("setupComputer")
        rule.onNodeWithTag("setupStep").assertTextEquals("STEP 1 OF 2")
        rule.onNodeWithTag("computerHowTo").assertTextContains("rewarden login", substring = true)
        assertTrue(has("desktopDownload"))
        tap("setupNext")
        awaitTag("setupAi")
        val mcp = "$server/mcp"
        rule.onNodeWithTag("mcpUrl").assertTextEquals(mcp)
        tap("copyMcp")
        val clip = context.getSystemService(ClipboardManager::class.java).primaryClip
        assertEquals(mcp, clip?.getItemAt(0)?.text?.toString())
        tap("setupDone")
        awaitTag("noActivity")

        relaunch(Intent(context, MainActivity::class.java))
        awaitTag("noActivity")
        assertFalse(has("setupComputer"))
    }

    @Test
    fun signingInAgainToTheSameAccountSkipsTheSetup() {
        core.session = null
        launch()
        signInFromWelcome()
        awaitTag("setupComputer")
        tap("setupSkip")
        awaitTag("noActivity")
        tap("openSettings")
        tap("signOut")
        rule.onAllNodes(hasText("Sign out")).let { it[it.fetchSemanticsNodes().lastIndex] }.performClick()
        awaitTag("welcome")
        signInFromWelcome()
        awaitTag("noActivity")
        assertFalse(has("setupComputer"))
        assertEquals(2, core.logins.size)
    }

    @Test
    fun peopleWhoWereAlreadySignedInNeverSeeTheSetup() {
        launch()
        awaitTag("noActivity")
        assertFalse(has("setupComputer"))
        assertFalse(has("welcome"))
    }

    @Test
    fun theSetupScansTheComputersCode() {
        core.session = null
        scan = ScanResult.Scanned("reins://pair?code=BCDF-GHJK")
        launch()
        fillCreateForm()
        tap("create")
        awaitTag("setupComputer")
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
        awaitText("That QR code is not a Reins pairing code. Scan the one your computer shows.")
        assertTrue(core.pairingCodes.isEmpty())
        assertFalse(has("sheet"))
    }

    @Test
    fun withoutTheScannerTheCodeCanBeTyped() {
        scan = ScanResult.Unavailable("module not installed")
        openConnectComputer()
        tap("scanQr")
        awaitTag("pairCode")
        awaitText("The QR scanner is not available on this phone. Type the code your computer shows instead.")
        rule.onNodeWithTag("submitCode").assertIsNotEnabled()
        rule.onNodeWithTag("pairCode").performTextReplacement("bcdf ghjk")
        rule.onNodeWithTag("submitCode").assertIsEnabled()
        tap("submitCode")
        awaitTag("keyFingerprint")
        assertEquals(listOf("BCDF-GHJK"), core.pairingCodes.toList())
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
        assertTrue(has("setupComputer"))
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
