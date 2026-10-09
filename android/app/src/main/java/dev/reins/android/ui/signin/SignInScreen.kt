package dev.reins.android.ui.signin

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.browser.auth.AuthTabIntent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.BuildConfig
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.CheckRow
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Cue
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.cueUnlessRecent
import dev.reins.android.platform.Browser
import dev.reins.android.ui.common.ReinsLinks

/** The pages of the signed-out app. */
enum class OnboardingPage { Welcome, Create, SignIn }

/**
 * Signed out: a welcome with one button, "Continue", which signs in (or up) on the server's own sign-in page in the
 * browser. It uses [BuildConfig.DEFAULT_SERVER] (app.reins2fa.com) unless the user opens "Use another server", which
 * also offers creating an account or signing in with email and master password on that server (never on the hosted
 * one). [linkWaiting]: a pairing link was opened and waits for the sign-in.
 */
@Composable
fun OnboardingScreen(viewModel: SignInViewModel, linkWaiting: Boolean = false) {
    val feedback = LocalFeedback.current
    var page by rememberSaveable { mutableStateOf(OnboardingPage.Welcome) }
    var server by rememberSaveable { mutableStateOf(BuildConfig.DEFAULT_SERVER) }
    // A build without a usable default shows the field from the start.
    var customServer by rememberSaveable { mutableStateOf(AccountRules.serverUrl(BuildConfig.DEFAULT_SERVER) == null) }
    var email by rememberSaveable { mutableStateOf("") }
    val go = { next: OnboardingPage ->
        viewModel.clearError()
        if (next == OnboardingPage.Welcome) feedback.cueUnlessRecent(Cue.Close) else feedback.cue(Cue.Open)
        page = next
    }
    BackHandler(enabled = page != OnboardingPage.Welcome) { go(OnboardingPage.Welcome) }
    when (page) {
        OnboardingPage.Welcome -> WelcomePage(
            viewModel = viewModel,
            linkWaiting = linkWaiting,
            server = server,
            onServer = { server = it },
            custom = customServer,
            onCustom = { custom ->
                viewModel.clearError()
                customServer = custom
                server = if (custom) "https://" else BuildConfig.DEFAULT_SERVER
            },
            onCreate = { go(OnboardingPage.Create) },
            onSignIn = { go(OnboardingPage.SignIn) },
        )
        OnboardingPage.Create -> CreateAccountPage(viewModel, server, email, { email = it }, onBack = { go(OnboardingPage.Welcome) })
        OnboardingPage.SignIn -> SignInPage(viewModel, server, email, { email = it }, onBack = { go(OnboardingPage.Welcome) })
    }
}

@Composable
private fun WelcomePage(
    viewModel: SignInViewModel,
    linkWaiting: Boolean,
    server: String,
    onServer: (String) -> Unit,
    custom: Boolean,
    onCustom: (Boolean) -> Unit,
    onCreate: () -> Unit,
    onSignIn: () -> Unit,
) {
    val c = LocalColors.current
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    // The sign-in page in an Auth Tab comes back here; closing it without signing in changes nothing.
    val authTab = rememberLauncherForActivityResult(AuthTabIntent.AuthenticateUserResultContract()) { result ->
        if (result.resultCode == AuthTabIntent.RESULT_OK) result.resultUri?.let { viewModel.authTabReturned(it.toString()) }
    }
    val valid = AccountRules.serverUrl(server) != null
    // Passwords are for servers people run themselves; the hosted server signs in through "Continue" only.
    val passwordForms = custom && valid && !AccountRules.isDefaultServer(server, BuildConfig.DEFAULT_SERVER)
    Screen(title = null, modifier = Modifier.testTag("welcome")) {
        Column(Modifier.padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(72.dp))
            Box(
                Modifier.size(64.dp).background(c.accentSoft, RoundedCornerShape(20.dp)),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.ShieldCheck, c.accent, size = 34.dp) }
            Spacer(Modifier.height(6.dp))
            RText("Reins", RType.sans(38f, FontWeight.SemiBold), c.text)
            RText(
                "Your AI assistants and agents ask this phone before they read, send or change anything, and you decide with a tap.",
                RType.sans(16.5f, lineHeight = 23f),
                c.secondary,
            )
            if (linkWaiting) {
                Banner("Continue to sign in, and your computer connects right after.", Modifier.padding(top = 6.dp), tag = "linkWaiting")
            }
            Spacer(Modifier.height(36.dp))
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "signInError") }
            CapsuleButton(
                "Continue",
                Modifier.fillMaxWidth().testTag("continue"),
                style = ButtonStyle.Primary,
                enabled = valid,
                busy = ui.busy,
            ) { viewModel.continueWithSso(server) { Browser.openSignIn(context, it, AccountRules.SSO_CALLBACK_SCHEME, authTab) } }
            if (!passwordForms) {
                RText(
                    "Sign in or create an account on the secure sign-in page.",
                    RType.sans(13.5f, lineHeight = 19f),
                    c.tertiary,
                )
            }
            ServerChoice(server, onServer, custom, onCustom, enabled = !ui.busy)
            if (passwordForms) {
                RText(
                    "Or with email and master password on this server:",
                    RType.sans(13.5f, lineHeight = 19f),
                    c.secondary,
                    Modifier.padding(start = 4.dp, top = 8.dp),
                )
                CapsuleButton("Sign in", Modifier.fillMaxWidth().testTag("startSignIn"), style = ButtonStyle.Secondary, enabled = !ui.busy, onClick = onSignIn)
                CapsuleButton(
                    "Create account",
                    Modifier.fillMaxWidth().testTag("createAccount"),
                    style = ButtonStyle.Secondary,
                    enabled = !ui.busy,
                    onClick = onCreate,
                )
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun CreateAccountPage(
    viewModel: SignInViewModel,
    server: String,
    email: String,
    onEmail: (String) -> Unit,
    onBack: () -> Unit,
) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val c = LocalColors.current
    val context = LocalContext.current
    // Passwords live only here and in the call; never saved state.
    var password by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var terms by rememberSaveable { mutableStateOf(false) }
    val ready = AccountRules.createProblem(email, password, confirm, terms) == null && AccountRules.serverUrl(server) != null

    Screen(title = "Create account", onBack = onBack) {
        Column(Modifier.padding(horizontal = 20.dp).padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            RText(
                "One account for this phone, your computer and your AI apps.",
                RType.sans(15f, lineHeight = 21f),
                c.secondary,
                Modifier.padding(start = 4.dp, bottom = 4.dp),
            )
            EmailField(email, onEmail, enabled = !ui.busy)
            RTextField(
                password, { password = it }, "Master password", tag = "password", enabled = !ui.busy, password = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
            )
            if (password.isNotEmpty()) StrengthMeter(password)
            RTextField(
                confirm, { confirm = it }, "Master password again", tag = "confirmPassword", enabled = !ui.busy, password = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
            )
            if (confirm.isNotEmpty() && confirm != password) {
                RText("The passwords don't match.", RType.sans(13.5f), c.danger, Modifier.padding(start = 4.dp).testTag("mismatch"))
            }
            Banner(
                "Nobody can recover or reset your master password, not even Reins. If you forget it, you lose the account " +
                    "and everything in it. Write it down and keep it somewhere safe.",
                kind = BannerKind.Warning,
                tag = "noRecovery",
            )
            CheckRow(terms, { terms = it }, Modifier.fillMaxWidth().padding(top = 4.dp, start = 4.dp), enabled = !ui.busy, tag = "acceptTerms") {
                RText("I accept the Terms of Service and the Privacy Policy.", RType.sans(15f, lineHeight = 21f), c.text, Modifier.weight(1f))
            }
            Row(Modifier.padding(start = 36.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                TextLink("Terms of Service", "openTerms") { Browser.open(context, ReinsLinks.TERMS) }
                TextLink("Privacy Policy", "openPrivacy") { Browser.open(context, ReinsLinks.PRIVACY) }
            }
            ServerLine(server)
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "signInError") }
            Spacer(Modifier.height(4.dp))
            CapsuleButton(
                "Create account",
                Modifier.fillMaxWidth().testTag("create"),
                style = ButtonStyle.Primary,
                enabled = ready,
                busy = ui.busy,
            ) { viewModel.createAccount(server, email, password, confirm, terms) }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun SignInPage(
    viewModel: SignInViewModel,
    server: String,
    email: String,
    onEmail: (String) -> Unit,
    onBack: () -> Unit,
) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val c = LocalColors.current
    var password by remember { mutableStateOf("") }
    var totp by remember { mutableStateOf("") }

    Screen(title = "Sign in", onBack = onBack) {
        Column(Modifier.padding(horizontal = 20.dp).padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            RText(
                "Sign in with your Reins account. This phone then approves what your AI assistants ask for.",
                RType.sans(15f, lineHeight = 21f),
                c.secondary,
                Modifier.padding(start = 4.dp, bottom = 4.dp),
            )
            EmailField(email, onEmail, enabled = !ui.busy)
            RTextField(
                password, { password = it }, "Master password", tag = "password", enabled = !ui.busy, password = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
            )
            if (ui.needsTotp) {
                RTextField(
                    totp, { totp = it }, "Two-step code", tag = "totp", enabled = !ui.busy, mono = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, autoCorrectEnabled = false),
                )
            }
            ServerLine(server)
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "signInError") }
            Spacer(Modifier.height(4.dp))
            CapsuleButton(
                "Sign in",
                Modifier.fillMaxWidth().testTag("signIn"),
                style = ButtonStyle.Primary,
                enabled = AccountRules.serverUrl(server) != null && email.isNotBlank() && password.isNotEmpty(),
                busy = ui.busy,
            ) { viewModel.signIn(server, email, password, totp) }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun EmailField(email: String, onEmail: (String) -> Unit, enabled: Boolean) {
    RTextField(
        email, onEmail, "Email", tag = "email", enabled = enabled,
        keyboardOptions = KeyboardOptions(
            keyboardType = KeyboardType.Email,
            autoCorrectEnabled = false,
            capitalization = KeyboardCapitalization.None,
        ),
    )
}

/** Three bars and a word, with one line of advice. */
@Composable
private fun StrengthMeter(password: String) {
    val c = LocalColors.current
    val strength = AccountRules.strength(password)
    val (label, tone, filled) = when (strength) {
        Strength.Weak -> Triple("Weak", c.danger, 1)
        Strength.Fair -> Triple("Fair", c.warning, 2)
        Strength.Strong -> Triple("Strong", c.success, 3)
    }
    Column(Modifier.padding(horizontal = 4.dp).testTag("strength")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            repeat(3) { i ->
                if (i > 0) Spacer(Modifier.width(6.dp))
                Box(Modifier.weight(1f).height(4.dp).clip(CircleShape).background(if (i < filled) tone else c.controlFill))
            }
            Spacer(Modifier.width(12.dp))
            RText(label, RType.sans(13.5f, FontWeight.SemiBold), tone, Modifier.testTag("strengthLabel"))
        }
        RText(AccountRules.strengthHint(password), RType.sans(13f), c.secondary, Modifier.padding(top = 6.dp))
    }
}

/**
 * The server: app.reins2fa.com, said in one line, with "Use another server" for people who run their own. Opening it
 * shows the address field.
 */
@Composable
private fun ServerChoice(server: String, onServer: (String) -> Unit, custom: Boolean, onCustom: (Boolean) -> Unit, enabled: Boolean) {
    val c = LocalColors.current
    val defaultHost = AccountRules.displayHost(BuildConfig.DEFAULT_SERVER)
    val hasDefault = AccountRules.serverUrl(BuildConfig.DEFAULT_SERVER) != null
    if (custom) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            RTextField(
                server, onServer, "https://reins.example.com", tag = "server", enabled = enabled,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
            )
            Row(Modifier.padding(start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                RText("For a server you run yourself.", RType.sans(13f), c.tertiary, Modifier.weight(1f))
                if (hasDefault) TextLink("Use $defaultHost", "defaultServer", enabled) { onCustom(false) }
            }
        }
    } else {
        Row(Modifier.padding(start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            RText("Server: $defaultHost", RType.sans(13f), c.tertiary, Modifier.weight(1f).testTag("serverName"), maxLines = 1, ltr = true)
            TextLink("Use another server", "useAnotherServer", enabled) { onCustom(true) }
        }
    }
}

/** Which server a password form uses (chosen on the welcome page). */
@Composable
private fun ServerLine(server: String) {
    RText(
        "Server: ${AccountRules.displayHost(server)}",
        RType.sans(13f),
        LocalColors.current.tertiary,
        Modifier.padding(start = 4.dp).testTag("serverName"),
        maxLines = 1,
        ltr = true,
    )
}

/** A small accent-coloured text button. */
@Composable
internal fun TextLink(text: String, tag: String, enabled: Boolean = true, onClick: () -> Unit) {
    RText(
        text,
        RType.sans(13.5f, FontWeight.Medium),
        LocalColors.current.accent,
        Modifier
            .testTag(tag)
            .pressable(enabled = enabled, shape = RoundedCornerShape(8.dp), onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 8.dp),
        maxLines = 1,
    )
}
