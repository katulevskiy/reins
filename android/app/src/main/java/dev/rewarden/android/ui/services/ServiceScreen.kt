package dev.rewarden.android.ui.services

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.BlobAvatar
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.ConfirmDialog
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RTextField
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.design.ServiceAvatar
import dev.rewarden.android.design.pressable
import dev.rewarden.android.platform.GoogleAuthorizer
import dev.rewarden.android.platform.PhoneBridge
import dev.rewarden.android.state.AppState
import dev.rewarden.android.ui.common.untrusted
import dev.rewarden.core.AccountView
import dev.rewarden.core.GmailStatus
import dev.rewarden.core.ServiceView

/** The accounts of one integration and the way to add another, which depends on the kind of service. */
@Composable
fun ServiceScreen(viewModel: ServiceViewModel, state: AppState, onBack: () -> Unit) {
    val c = LocalColors.current
    val services by state.services.collectAsStateWithLifecycle()
    val statuses by viewModel.statuses.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    val busy by viewModel.busy.collectAsStateWithLifecycle()
    val login by viewModel.login.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(viewModel) { viewModel.refresh() }

    val service = services.firstOrNull { it.service == viewModel.service }
    if (service == null) {
        Screen(title = "Integration", onBack = onBack) { Banner("This integration is not available.", kind = BannerKind.Error) }
        return
    }
    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { viewModel.consentFinished() }
    val chooser = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        GoogleAuthorizer.chosenAccount(result.data)?.let { account ->
            viewModel.accountChosen(account) { pending -> consent.launch(IntentSenderRequest.Builder(pending.intentSender).build()) }
        }
    }
    val permissions = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        if (granted.values.all { it }) viewModel.addDevice() else viewModel.fail("Android did not allow it. You can allow it in the phone's settings for Rewarden.")
    }

    Screen(title = service.name, subtitle = "Integration", onBack = onBack) {
        Group(header = "Accounts", footer = accountsFooter(service)) {
            if (service.accounts.isEmpty()) {
                Column(Modifier.padding(16.dp).testTag("noAccounts")) {
                    RText("Not connected", RType.sans(16f, FontWeight.Medium), c.text)
                    RText(intro(service), RType.sans(13f, lineHeight = 18f), c.secondary, Modifier.padding(top = 2.dp))
                }
            }
            service.accounts.forEachIndexed { i, account ->
                if (i > 0) Hairline(inset = 68.dp)
                AccountRow(
                    service = service,
                    account = account,
                    status = statuses[account.account],
                    busy = busy,
                    onReconnect = {
                        when (service.kind) {
                            "google" -> viewModel.accountChosen(account.account) { pending ->
                                consent.launch(IntentSenderRequest.Builder(pending.intentSender).build())
                            }
                            "device" -> permissions.launch(PhoneBridge.permissionsOf(service.service).toTypedArray())
                            else -> Unit
                        }
                    },
                    onRemove = { removing = account.account },
                )
            }
        }
        if (service.available) {
            AddAccount(
                service = service,
                viewModel = viewModel,
                busy = busy,
                login = login,
                onChooseGoogle = { chooser.launch(GoogleAuthorizer.chooseAccountIntent()) },
                onAllowDevice = { permissions.launch(PhoneBridge.permissionsOf(service.service).toTypedArray()) },
            )
        } else {
            Banner(service.note ?: "Not available in this build.", Modifier.padding(16.dp), BannerKind.Warning, tag = "unavailable")
        }
        error?.let { Banner(untrusted(it), Modifier.padding(horizontal = 16.dp), BannerKind.Error, tag = "accountError") }
        RText(fineprint(service), RType.sans(13.5f, lineHeight = 19f), c.tertiary, Modifier.padding(start = 32.dp, end = 32.dp, top = 16.dp, bottom = 32.dp))
    }

    removing?.let { account ->
        ConfirmDialog(
            title = "Remove ${if (service.kind == "device") service.name else account}?",
            text = "Your AIs lose access to this, and the grants made for it are deleted.",
            confirmLabel = "Remove",
            onConfirm = {
                removing = null
                viewModel.remove(account, google = service.kind == "google")
            },
            onDismiss = { removing = null },
        )
    }
}

private fun intro(service: ServiceView): String = when (service.service) {
    "gcalendar" -> "Add a Google account so your AIs can see your events and add new ones."
    "gcontacts" -> "Add a Google account so your AIs can look up your contacts."
    "telegram" -> "Sign in with your own Telegram account, the way you do in the Telegram app. It is not a bot."
    "github" -> "Connect a GitHub access token so your AIs can work with your repositories: read code and issues, and, when you approve, change them."
    "gitlab", "codeberg", "bitbucket" ->
        "Connect a ${service.name} access token so git on your computer can clone, fetch and, when you approve each push, push through the Rewarden desktop app."
    "device_calendar" -> "Let your AIs see and add events in the calendars on this phone."
    "device_contacts" -> "Let your AIs look up the contacts on this phone."
    "sms" -> "Let your AIs read and send text messages from this phone."
    "vault" -> "Let your AIs ask for a login from your password vault, one field at a time."
    else -> "Connect an account so your AIs can use it."
}

private fun accountsFooter(service: ServiceView): String = when (service.kind) {
    "device" -> "This works with what is on this phone. You approve what your AIs see or do."
    "vault" -> "Passwords, one-time codes and usernames are asked for every time and never remembered."
    else -> "Your AIs see this list and pick the account a request is about. Grants belong to one account."
}

private fun fineprint(service: ServiceView): String = when (service.service) {
    "telegram" ->
        "Rewarden signs in as you on this phone only; the session is kept encrypted here and never sent to the server. Telegram may limit accounts that are used by automation, so Rewarden only acts when you approve."
    "github" -> "The token is kept encrypted on this phone. Every change is shown to you first, and dangerous ones are asked for every time."
    "gitlab", "codeberg", "bitbucket" ->
        "The token is kept encrypted on this phone and never leaves it. Each push is shown to you branch by branch before it goes to ${service.name}."
    "sms" -> "Messages that look like login codes are hidden from lists and are never shared unless you tick them one by one."
    "vault" -> "Your master password is used once to unlock the vault key, which is then kept encrypted on this phone. The password itself is not kept."
    "device_calendar", "device_contacts" -> "Android asks you to allow this. You can take the permission back in the phone's settings at any time."
    else -> "Rewarden asks Google for access on this phone only. Nothing is stored on the server."
}

@Composable
private fun AddAccount(
    service: ServiceView,
    viewModel: ServiceViewModel,
    busy: Boolean,
    login: LoginStep,
    onChooseGoogle: () -> Unit,
    onAllowDevice: () -> Unit,
) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 16.dp), verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(10.dp)) {
        when (service.kind) {
            "google" -> CapsuleButton("Add account", Modifier.fillMaxWidth().testTag("addAccount"), enabled = !busy, glyph = Glyph.Plus, onClick = onChooseGoogle)
            "device" -> if (service.accounts.isEmpty()) {
                CapsuleButton("Allow on this phone", Modifier.fillMaxWidth().testTag("allowDevice"), enabled = !busy, glyph = Glyph.Phone, onClick = onAllowDevice)
            }
            "token" -> when (val host = GitHosts.of(service.service)) {
                null -> GithubConnect(busy) { viewModel.addSecret(it) }
                else -> GitHostConnect(host, busy) { viewModel.addSecret(it) }
            }
            "vault" -> if (service.accounts.isEmpty()) {
                SecretForm(
                    placeholder = "Master password",
                    button = "Unlock and connect",
                    busy = busy,
                    hint = "The vault of the account this phone is signed in with.",
                ) { viewModel.addSecret(it) }
            }
            "telegram" -> TelegramLogin(viewModel, busy, login)
            else -> RText("This kind of integration cannot be added here.", RType.sans(14f), c.secondary)
        }
    }
}

/** A token is recognised by its prefix, so nothing else that happens to be on the clipboard is ever used. */
fun looksLikeGithubToken(text: String?): Boolean {
    val t = text?.trim() ?: return false
    return t.length in 30..255 && Regex("^(github_pat_|ghp_|gho_|ghu_|ghs_)[A-Za-z0-9_]+$").matches(t)
}

/** GitHub's own page for a new fine-grained token, filled in with what Rewarden needs. */
fun githubTokenUrl(suffix: Int = (100_000..999_999).random()): String = GITHUB_TOKEN_URL.replace("name=Rewarden&", "name=Rewarden-$suffix&")

/**
 * The fine-grained token page. GitHub refuses a second token with the same name, so [githubTokenUrl] gives each one its
 * own number. Write access where the tools change something, read for the alert lists, metadata is always read.
 */
const val GITHUB_TOKEN_URL =
    "https://github.com/settings/personal-access-tokens/new?name=Rewarden&description=Lets+my+AI+work+with+my+repositories+through+the+Rewarden+app&expires_in=180" +
        "&metadata=read&contents=write&issues=write&pull_requests=write&actions=write&workflows=write&administration=write" +
        "&repository_hooks=write&secrets=write&variables=write&environments=write&checks=write&statuses=write" +
        "&security_events=read&vulnerability_alerts=read&secret_scanning_alerts=read"

/** The classic token page: one token for everything, including the account-level things fine-grained tokens cannot do. */
fun githubClassicTokenUrl(suffix: Int = (100_000..999_999).random()): String = GITHUB_CLASSIC_TOKEN_URL.replace("description=Rewarden&", "description=Rewarden-$suffix&")

const val GITHUB_CLASSIC_TOKEN_URL =
    "https://github.com/settings/tokens/new?description=Rewarden&scopes=repo,workflow,gist,notifications,read:org,admin:repo_hook,delete_repo"

internal fun clipboardText(context: android.content.Context): String? = try {
    val manager = context.getSystemService(android.content.ClipboardManager::class.java)
    manager?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()
} catch (e: RuntimeException) {
    null
}

/**
 * GitHub in two taps: open GitHub's token page (already filled in), press Generate and Copy there, come back. The copied
 * token is picked up from the clipboard and connected; pasting by hand is still possible.
 */
@Composable
private fun GithubConnect(busy: Boolean, onToken: (String) -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    var waiting by remember { mutableStateOf(false) }
    var found by remember { mutableStateOf<String?>(null) }
    var manual by remember { mutableStateOf(false) }
    androidx.lifecycle.compose.LifecycleEventEffect(androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
        val copied = clipboardText(context)?.trim()
        if (looksLikeGithubToken(copied)) {
            if (waiting && !busy) {
                waiting = false
                // Do not leave the token lying on the clipboard.
                runCatching { context.getSystemService(android.content.ClipboardManager::class.java)?.clearPrimaryClip() }
                onToken(copied!!)
            } else {
                found = copied
            }
        }
    }
    fun open(url: String) {
        waiting = true
        context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
    }
    CapsuleButton("Fine-grained token (pick repositories)", Modifier.fillMaxWidth().testTag("openGithub"), enabled = !busy, glyph = Glyph.Key) {
        open(githubTokenUrl())
    }
    RText(
        "GitHub opens with the token already set up. Under Repository access choose the repositories to allow, tap Generate token, then the copy button. Come back here and it connects by itself. It only reaches the repositories you choose, and cannot do gists or notifications.",
        RType.sans(13f, lineHeight = 18f),
        c.secondary,
        Modifier.padding(horizontal = 4.dp),
    )
    CapsuleButton("Classic token (everything, incl. gists and notifications)", Modifier.fillMaxWidth().testTag("openGithubClassic"), style = ButtonStyle.Secondary, enabled = !busy, maxLines = 2) {
        open(githubClassicTokenUrl())
    }
    RText(
        "A classic token reaches every repository of your account and organizations, plus gists and notifications. Tap Generate token at the bottom of the page, copy it, and come back.",
        RType.sans(13f, lineHeight = 18f),
        c.secondary,
        Modifier.padding(horizontal = 4.dp),
    )
    found?.let { token ->
        CapsuleButton("Use the token I copied", Modifier.fillMaxWidth().testTag("useCopied"), style = ButtonStyle.Accent, enabled = !busy) {
            found = null
            runCatching { context.getSystemService(android.content.ClipboardManager::class.java)?.clearPrimaryClip() }
            onToken(token)
        }
    }
    if (!manual) {
        CapsuleButton("I already have a token", Modifier.fillMaxWidth().testTag("pasteManually"), style = ButtonStyle.Ghost, enabled = !busy) { manual = true }
    } else {
        SecretForm(placeholder = "Access token", button = "Connect", busy = busy, hint = "A fine-grained or classic token. It is kept encrypted on this phone.", onSubmit = onToken)
    }
}

/** One secret typed in and sent once; it is cleared as soon as it is sent. */
@Composable
internal fun SecretForm(placeholder: String, button: String, busy: Boolean, hint: String, onSubmit: (String) -> Unit) {
    val c = LocalColors.current
    var secret by remember { mutableStateOf("") }
    RTextField(
        secret, { secret = it }, placeholder, tag = "secret", enabled = !busy, password = true, mono = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
    )
    RText(hint, RType.sans(13f, lineHeight = 18f), c.secondary, Modifier.padding(horizontal = 4.dp))
    CapsuleButton(button, Modifier.fillMaxWidth().testTag("connectSecret"), enabled = !busy && secret.isNotBlank(), busy = busy) {
        val sent = secret
        secret = ""
        onSubmit(sent)
    }
}

/** Phone number → code → (password): the same steps as signing in to Telegram anywhere else. */
@Composable
private fun TelegramLogin(viewModel: ServiceViewModel, busy: Boolean, login: LoginStep) {
    val c = LocalColors.current
    var phone by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    when (login) {
        LoginStep.Phone -> {
            RTextField(
                phone, { phone = it }, "Phone number, like +1 555 010 0100", tag = "phone", enabled = !busy, mono = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Phone, autoCorrectEnabled = false),
            )
            CapsuleButton("Send me a code", Modifier.fillMaxWidth().testTag("sendCode"), enabled = !busy && phone.count(Char::isDigit) >= 7, busy = busy) {
                viewModel.loginBegin(phone)
            }
        }
        is LoginStep.Code -> {
            RText("Telegram sent a code to ${login.phone}. It arrives in the Telegram app on your other devices, or as a text.", RType.sans(13.5f, lineHeight = 19f), c.secondary)
            RTextField(
                code, { code = it.filter(Char::isDigit).take(8) }, "Login code", tag = "code", enabled = !busy, mono = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, autoCorrectEnabled = false),
            )
            CapsuleButton("Sign in", Modifier.fillMaxWidth().testTag("submitCode"), enabled = !busy && code.length >= 4, busy = busy) {
                val sent = code
                code = ""
                viewModel.loginCode(sent)
            }
            CapsuleButton("Use another number", Modifier.fillMaxWidth().testTag("restartLogin"), style = ButtonStyle.Ghost, enabled = !busy, onClick = viewModel::loginRestart)
        }
        is LoginStep.Password -> {
            RText(
                "This account has two-step verification. Enter its Telegram password" + (login.hint?.takeIf { it.isNotBlank() }?.let { " (hint: ${untrusted(it)})" } ?: "") + ".",
                RType.sans(13.5f, lineHeight = 19f),
                c.secondary,
            )
            RTextField(
                password, { password = it }, "Telegram password", tag = "tgPassword", enabled = !busy, password = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
            )
            CapsuleButton("Sign in", Modifier.fillMaxWidth().testTag("submitPassword"), enabled = !busy && password.isNotEmpty(), busy = busy) {
                val sent = password
                password = ""
                viewModel.loginPassword(sent)
            }
        }
    }
}

@Composable
private fun AccountRow(
    service: ServiceView,
    account: AccountView,
    status: GmailStatus?,
    busy: Boolean,
    onReconnect: () -> Unit,
    onRemove: () -> Unit,
) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().testTag("account:${account.account}").padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (service.kind == "device") ServiceAvatar(service.service, size = 40.dp) else BlobAvatar(account.account, size = 40.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(if (service.kind == "device") service.name else account.account, RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1, ltr = true)
            when (status) {
                null -> RText("Checking…", RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp))
                GmailStatus.Ready -> RText("Connected", RType.sans(13f, FontWeight.Medium), c.success, Modifier.padding(top = 2.dp))
                GmailStatus.NeedsConsent -> RText(needsAgain(service), RType.sans(13f, FontWeight.Medium), c.warning, Modifier.padding(top = 2.dp))
                is GmailStatus.Unavailable -> RText(untrusted(status.message), RType.sans(13f), c.danger, Modifier.padding(top = 2.dp), maxLines = 3)
            }
            if (status == GmailStatus.NeedsConsent && (service.kind == "google" || service.kind == "device")) {
                CapsuleButton(
                    "Allow again",
                    Modifier.padding(top = 8.dp).testTag("reconnect:${account.account}"),
                    style = ButtonStyle.Secondary,
                    compact = true,
                    enabled = !busy,
                    onClick = onReconnect,
                )
            }
        }
        Spacer(Modifier.width(8.dp))
        Row(
            Modifier
                .testTag("removeAccount:${account.account}")
                .pressable(enabled = !busy, shape = CircleShape, onClick = onRemove)
                .padding(10.dp),
        ) { GlyphIcon(Glyph.Trash, c.danger, size = 20.dp) }
    }
}

private fun needsAgain(service: ServiceView): String = when (service.kind) {
    "device" -> "Needs Android's permission again"
    "telegram" -> "Signed out: remove it and sign in again"
    "token" -> "The token no longer works: remove it and add a new one"
    "vault" -> "Remove it and enter the master password again"
    else -> "Needs your permission again"
}
