package dev.reins.android.ui.signin

import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.withStyle
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.imePadding
import dev.reins.android.design.pane
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.sp

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.browser.auth.AuthTabIntent
import androidx.compose.foundation.background
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
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
import androidx.compose.runtime.LaunchedEffect
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
import kotlinx.coroutines.delay

/** How long the address must stay unchanged before the server is asked what it offers. */
private const val SERVER_CHECK_DELAY_MS = 400L

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
    // After signing out of a self-hosted server, the welcome page offers that server again.
    val lastServer by viewModel.lastServer.collectAsStateWithLifecycle()
    LaunchedEffect(lastServer) {
        val last = lastServer ?: return@LaunchedEffect
        if (!customServer && server == BuildConfig.DEFAULT_SERVER && !AccountRules.isDefaultServer(last, BuildConfig.DEFAULT_SERVER)) {
            customServer = true
            server = last
        }
    }
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
    val url = AccountRules.serverUrl(server)
    val valid = url != null
    // Asked once the address stops changing: whether "Continue" works there at all.
    val serverInfo by viewModel.serverInfo.collectAsStateWithLifecycle()
    LaunchedEffect(server) {
        delay(SERVER_CHECK_DELAY_MS)
        viewModel.checkServer(server)
    }
    val noBrowserSignIn = serverInfo?.let { (asked, info) -> asked == url && info.browserSignIn == false } == true
    // Passwords are for servers people run themselves; the hosted server signs in through "Continue" only.
    val passwordForms = (custom && valid && !AccountRules.isDefaultServer(server, BuildConfig.DEFAULT_SERVER)) || noBrowserSignIn
    // Next (Arrive 01): the theme's wallpaper fills the page; a frosted pane at the foot says it in two lines and
    // holds Continue and Other server.
    val appearance by dev.reins.android.design.Appearance.state(context)
    dev.reins.android.design.WallpaperPage(appearance.wallpaper(c.dark), Modifier.fillMaxSize().testTag("welcome"), artHeight = null) {
        Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.systemBars)) {
            Spacer(Modifier.weight(1f).heightIn(min = 280.dp))
            if (linkWaiting) Banner("Sign in, then your computer connects.", Modifier.padding(horizontal = 16.dp).padding(bottom = 10.dp), tag = "linkWaiting")
            Column(
                Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp).fillMaxWidth().pane(radius = 32.dp).padding(start = 22.dp, end = 22.dp, top = 26.dp, bottom = 12.dp).testTag("kite"),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                val headline = androidx.compose.ui.text.buildAnnotatedString {
                    append("Your AI asks,\n")
                    withStyle(androidx.compose.ui.text.SpanStyle(color = c.kindAccounts, fontWeight = FontWeight.SemiBold)) { append("You decide.") }
                }
                androidx.compose.foundation.text.BasicText(
                    headline,
                    Modifier.semantics { heading() }.testTag("welcomeHeadline"),
                    style = RType.display(48f).copy(color = c.ink, lineHeight = 38.sp),
                )
                Spacer(Modifier.height(6.dp))
                ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "signInError") }
                // A server without SSO would only answer "Continue" with an error page: it is not offered there.
                if (!noBrowserSignIn) {
                    dev.reins.android.design.SignalButton(
                        "Continue",
                        Modifier.fillMaxWidth().testTag("continue"),
                        enabled = valid && !ui.busy,
                        leading = if (ui.busy) ({ dev.reins.android.design.Spinner(c.onInverse, size = 18.dp) }) else null,
                    ) { viewModel.continueWithSso(server) { Browser.openSignIn(context, it, AccountRules.SSO_CALLBACK_SCHEME, authTab) } }
                }
                ServerChoice(server, onServer, custom, onCustom, enabled = !ui.busy)
                if (passwordForms) {
                    RText(
                        if (noBrowserSignIn) "Email and master password" else "Or with email and master password",
                        RType.sans(13.5f, lineHeight = 19f),
                        c.ink2,
                        Modifier.padding(start = 4.dp, top = 4.dp).testTag("passwordFormsNote"),
                    )
                    dev.reins.android.design.SignalButton(
                        "Sign in",
                        Modifier.fillMaxWidth().testTag("startSignIn"),
                        style = if (noBrowserSignIn) ButtonStyle.Inverse else ButtonStyle.Soft,
                        enabled = !ui.busy,
                        onClick = onSignIn,
                    )
                    dev.reins.android.design.SignalButton(
                        "Create account",
                        Modifier.fillMaxWidth().testTag("createAccount"),
                        style = ButtonStyle.Soft,
                        enabled = !ui.busy,
                        onClick = onCreate,
                    )
                }
            }
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

    // Arrive 02: the accounts tile and "New account"; the server, email and passwords as rows of one group (the
    // strength as a short meter on its row); Terms · Privacy with a switch; Create account at the foot.
    Column(Modifier.fillMaxSize().background(c.paper).windowInsetsPadding(WindowInsets.systemBars).imePadding()) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            dev.reins.android.design.BackArrow(onBack, Modifier.padding(start = 12.dp, top = 8.dp))
            Row(Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 18.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                dev.reins.android.design.RowTile(Glyph.Person, size = 44.dp, fill = c.fieldAccounts)
                RText("New account", RType.pageTitle, c.ink, Modifier.semantics { heading() }, maxLines = 1)
            }
            dev.reins.android.design.PaneGroup {
                FormRow(Glyph.Server, first = true) {
                    RText(AccountRules.displayHost(server), RType.mono(13.5f), c.ink2, Modifier.testTag("serverName"), maxLines = 1, ltr = true)
                }
                FormRow(Glyph.Mail) {
                    FormField(
                        email, onEmail, "Email", "email", !ui.busy,
                        KeyboardOptions(keyboardType = KeyboardType.Email, autoCorrectEnabled = false, capitalization = KeyboardCapitalization.None),
                    )
                }
                FormRow(Glyph.Lock, trailing = { if (password.isNotEmpty()) StrengthMeter(password) }) {
                    FormField(
                        password, { password = it }, "Master password", "password", !ui.busy,
                        KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false), password = true,
                    )
                }
                FormRow(
                    Glyph.Lock,
                    trailing = {
                        if (confirm.isNotEmpty() && confirm == password) GlyphIcon(Glyph.Check, c.success, size = 18.dp)
                    },
                ) {
                    FormField(
                        confirm, { confirm = it }, "Again", "confirmPassword", !ui.busy,
                        KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false), password = true,
                    )
                }
            }
            if (confirm.isNotEmpty() && confirm != password) {
                RText("The passwords don't match.", RType.meta, c.danger, Modifier.padding(start = 24.dp, top = 8.dp).testTag("mismatch"))
            }
            RText(
                "Nobody can reset your master password. Write it down.",
                RType.meta,
                c.inkMeta,
                Modifier.padding(start = 24.dp, end = 24.dp, top = 8.dp).testTag("noRecovery"),
            )
            dev.reins.android.design.PaneGroup(Modifier.padding(top = 14.dp)) {
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                        TextLink("Terms", "openTerms") { Browser.open(context, ReinsLinks.TERMS) }
                        RText("·", RType.sans(14f), c.inkMeta)
                        TextLink("Privacy", "openPrivacy") { Browser.open(context, ReinsLinks.PRIVACY) }
                    }
                    dev.reins.android.design.Toggle(
                        terms,
                        Modifier.semantics { contentDescription = "I accept the Terms of Service and the Privacy Policy" }.testTag("acceptTerms"),
                        enabled = !ui.busy,
                    ) { terms = it }
                }
            }
            ui.error?.let { Banner(it, Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), kind = BannerKind.Error, tag = "signInError") }
            Spacer(Modifier.height(16.dp))
        }
        CapsuleButton(
            "Create account",
            Modifier.padding(horizontal = 16.dp, vertical = 14.dp).fillMaxWidth().testTag("create"),
            style = ButtonStyle.Primary,
            enabled = ready,
            busy = ui.busy,
        ) { viewModel.createAccount(server, email, password, confirm, terms) }
    }
}

/** A row of the account form: its glyph, then the field or value, and an optional mark at the end. */
@Composable
private fun FormRow(glyph: Glyph, first: Boolean = false, trailing: (@Composable () -> Unit)? = null, content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .then(if (first) Modifier else Modifier.drawBehind { drawLine(c.rule, Offset(48.dp.toPx(), 0f), Offset(size.width, 0f), 1.dp.toPx()) })
            .heightIn(min = 56.dp)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        GlyphIcon(glyph, c.inkMeta, size = 18.dp)
        Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) { content() }
        trailing?.invoke()
    }
}

/** A bare field for a [FormRow]: the words, or the placeholder in meta. */
@Composable
private fun FormField(
    value: String,
    onChange: (String) -> Unit,
    placeholder: String,
    tag: String,
    enabled: Boolean,
    keyboardOptions: KeyboardOptions,
    password: Boolean = false,
) {
    val c = LocalColors.current
    androidx.compose.foundation.text.BasicTextField(
        value,
        onChange,
        Modifier.fillMaxWidth().testTag(tag),
        enabled = enabled,
        singleLine = true,
        textStyle = RType.sans(15.5f).copy(color = c.ink),
        cursorBrush = androidx.compose.ui.graphics.SolidColor(c.violet),
        keyboardOptions = keyboardOptions,
        visualTransformation = if (password) androidx.compose.ui.text.input.PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
        decorationBox = { inner ->
            Box(contentAlignment = Alignment.CenterStart) {
                if (value.isEmpty()) RText(placeholder, RType.sans(15.5f), c.inkFaint, maxLines = 1)
                inner()
            }
        },
    )
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

    // As New account (Arrive 02): the server, email and password as rows of one group; Sign in at the foot.
    Column(Modifier.fillMaxSize().background(c.paper).windowInsetsPadding(WindowInsets.systemBars).imePadding()) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            dev.reins.android.design.BackArrow(onBack, Modifier.padding(start = 12.dp, top = 8.dp))
            Row(Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 18.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                dev.reins.android.design.RowTile(Glyph.Person, size = 44.dp, fill = c.fieldAccounts)
                RText("Sign in", RType.pageTitle, c.ink, Modifier.semantics { heading() }, maxLines = 1)
            }
            dev.reins.android.design.PaneGroup {
                FormRow(Glyph.Server, first = true) {
                    RText(AccountRules.displayHost(server), RType.mono(13.5f), c.ink2, Modifier.testTag("serverName"), maxLines = 1, ltr = true)
                }
                FormRow(Glyph.Mail) {
                    FormField(
                        email, onEmail, "Email", "email", !ui.busy,
                        KeyboardOptions(keyboardType = KeyboardType.Email, autoCorrectEnabled = false, capitalization = KeyboardCapitalization.None),
                    )
                }
                FormRow(Glyph.Lock) {
                    FormField(
                        password, { password = it }, "Master password", "password", !ui.busy,
                        KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false), password = true,
                    )
                }
                if (ui.needsTotp) {
                    FormRow(Glyph.Token) {
                        FormField(
                            totp, { totp = it }, "Two-step code", "totp", !ui.busy,
                            KeyboardOptions(keyboardType = KeyboardType.Number, autoCorrectEnabled = false),
                        )
                    }
                }
            }
            ui.error?.let { Banner(it, Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), kind = BannerKind.Error, tag = "signInError") }
            Spacer(Modifier.height(16.dp))
        }
        CapsuleButton(
            "Sign in",
            Modifier.padding(horizontal = 16.dp, vertical = 14.dp).fillMaxWidth().testTag("signIn"),
            style = ButtonStyle.Primary,
            enabled = AccountRules.serverUrl(server) != null && email.isNotBlank() && password.isNotEmpty(),
            busy = ui.busy,
        ) { viewModel.signIn(server, email, password, totp) }
    }
}

/** Three short bars and the word (Weak, Fair, Strong) at the end of the password's row. */
@Composable
private fun StrengthMeter(password: String) {
    val c = LocalColors.current
    val strength = AccountRules.strength(password)
    val (label, tone, filled) = when (strength) {
        Strength.Weak -> Triple("Weak", c.danger, 1)
        Strength.Fair -> Triple("Fair", c.warning, 2)
        Strength.Strong -> Triple("Strong", c.success, 3)
    }
    Row(Modifier.testTag("strength"), verticalAlignment = Alignment.CenterVertically) {
        repeat(3) { i ->
            if (i > 0) Spacer(Modifier.width(3.dp))
            Box(Modifier.width(12.dp).height(3.dp).clip(CircleShape).background(if (i < filled) tone else c.paperSunken))
        }
        Spacer(Modifier.width(8.dp))
        RText(label, RType.sans(12.5f, FontWeight.SemiBold), tone, Modifier.testTag("strengthLabel"), maxLines = 1)
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
        // "Other server" alone, centred; the server in use is what a screen reader hears on it.
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 46.dp)
                .clip(CircleShape)
                .pressable(enabled = enabled, shape = CircleShape, label = "Other server") { onCustom(true) }
                .semantics(mergeDescendants = true) { contentDescription = "Other server. Now $defaultHost" }
                .testTag("useAnotherServer"),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            dev.reins.android.design.GlyphIcon(dev.reins.android.design.Glyph.Server, c.ink2, size = 18.dp)
            RText("Other server", RType.sans(16f, FontWeight.SemiBold), c.ink2, maxLines = 1)
        }
    }
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
