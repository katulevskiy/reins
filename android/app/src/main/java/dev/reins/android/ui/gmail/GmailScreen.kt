package dev.reins.android.ui.gmail

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.pressable
import dev.reins.android.platform.GoogleAuthorizer
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted
import dev.reins.core.AccountView
import dev.reins.core.GmailStatus

/** The Gmail accounts: add as many as you like, or remove any of them. */
@Composable
fun GmailScreen(viewModel: GmailViewModel, state: AppState, onBack: () -> Unit) {
    val c = LocalColors.current
    val accounts by state.accounts.collectAsStateWithLifecycle()
    val statuses by viewModel.statuses.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    val busy by viewModel.busy.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(viewModel) { viewModel.refresh() }

    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) {
        viewModel.consentFinished()
    }
    val chooser = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        GoogleAuthorizer.chosenAccount(result.data)?.let { account ->
            viewModel.accountChosen(account) { pending -> consent.launch(IntentSenderRequest.Builder(pending.intentSender).build()) }
        }
    }
    val gmail = accounts.filter { it.service == "gmail" }

    Screen(title = "Gmail", subtitle = "Integration", onBack = onBack) {
        Group(
            header = "Accounts",
            footer = "Your AIs see this list and pick the account a request is about. Grants belong to one account.",
        ) {
            if (gmail.isEmpty()) {
                Column(Modifier.padding(16.dp).testTag("noAccounts")) {
                    RText("No account yet", RType.sans(16f, FontWeight.Medium), c.text)
                    RText(
                        "Add a Google account so your AIs can search and send mail through this phone.",
                        RType.sans(13f, lineHeight = 18f),
                        c.secondary,
                        Modifier.padding(top = 2.dp),
                    )
                }
            }
            gmail.forEachIndexed { i, account ->
                if (i > 0) Hairline(inset = 68.dp)
                AccountRow(
                    account = account,
                    status = statuses[account.account],
                    busy = busy,
                    onReconnect = {
                        viewModel.reconnect(account.account) { pending ->
                            consent.launch(IntentSenderRequest.Builder(pending.intentSender).build())
                        }
                    },
                    onRemove = { removing = account.account },
                )
            }
        }
        Column(Modifier.padding(horizontal = 16.dp, vertical = 16.dp)) {
            CapsuleButton(
                "Add account",
                Modifier.fillMaxWidth().testTag("addAccount"),
                enabled = !busy,
                busy = busy,
                glyph = Glyph.Plus,
            ) { chooser.launch(GoogleAuthorizer.chooseAccountIntent()) }
            error?.let { Banner(untrusted(it), Modifier.padding(top = 12.dp), BannerKind.Error, tag = "accountError") }
        }
        RText(
            "Reins asks Google for access on this phone only. Nothing about your mail is stored on the server.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, bottom = 32.dp),
        )
    }

    removing?.let { address ->
        ConfirmDialog(
            title = "Remove $address?",
            text = "Your AIs lose access to this account, and the grants made for it are deleted.",
            confirmLabel = "Remove",
            onConfirm = {
                removing = null
                viewModel.remove(address)
            },
            onDismiss = { removing = null },
        )
    }
}

@Composable
private fun AccountRow(account: AccountView, status: GmailStatus?, busy: Boolean, onReconnect: () -> Unit, onRemove: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().testTag("account:${account.account}").padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BlobAvatar(account.account, size = 40.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(account.account, RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1, ltr = true)
            when (status) {
                null -> RText("Checking…", RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp))
                GmailStatus.Ready -> RText("Connected", RType.sans(13f, FontWeight.Medium), c.success, Modifier.padding(top = 2.dp))
                GmailStatus.NeedsConsent -> RText(
                    "Needs your permission again",
                    RType.sans(13f, FontWeight.Medium),
                    c.warning,
                    Modifier.padding(top = 2.dp),
                )
                is GmailStatus.Unavailable -> RText(untrusted(status.message), RType.sans(13f), c.danger, Modifier.padding(top = 2.dp), maxLines = 3)
            }
            if (status == GmailStatus.NeedsConsent) {
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
                .pressable(enabled = !busy, shape = androidx.compose.foundation.shape.CircleShape, onClick = onRemove)
                .padding(10.dp),
        ) { GlyphIcon(Glyph.Trash, c.danger, size = 20.dp) }
    }
}
