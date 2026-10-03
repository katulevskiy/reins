package dev.reins.android.ui.grants

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SelectChip
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted

/** Give an AI a permission before it asks. Rare, so it lives one tap away in Grants, not in the way. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun NewGrantScreen(viewModel: NewGrantViewModel, state: AppState, authenticator: Authenticator, onBack: () -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val accounts by state.accounts.collectAsStateWithLifecycle()
    val draft = ui.draft
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    // With a single AI there is nothing to choose.
    LaunchedEffect(connections) {
        if (draft.connectionId == null && connections.size == 1) viewModel.edit { it.copy(connectionId = connections.first().id) }
    }
    // With a single account there is nothing to choose either.
    LaunchedEffect(accounts) {
        if (draft.account == null && accounts.size == 1) viewModel.edit { it.copy(account = accounts.first().account) }
    }

    Screen(title = "New grant", subtitle = "Allow something before it is asked", onBack = onBack) {
        Label("For which AI?")
        if (connections.isEmpty()) {
            RText("No AI is connected yet.", RType.sans(15f), c.secondary, Modifier.padding(horizontal = 20.dp))
        }
        FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            connections.forEach { conn ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    SelectChip(untrusted(conn.label), draft.connectionId == conn.id, Modifier.testTag("conn:${conn.id}")) {
                        viewModel.edit { it.copy(connectionId = conn.id) }
                    }
                }
            }
        }

        Label("On which account?")
        if (accounts.isEmpty()) {
            RText("No Gmail account is connected yet. Add one under Integrations.", RType.sans(15f, lineHeight = 20f), c.secondary, Modifier.padding(horizontal = 20.dp).testTag("noAccounts"))
        }
        FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            accounts.forEach { a ->
                SelectChip(a.account, draft.account == a.account, Modifier.testTag("acct:${a.account}")) {
                    viewModel.edit { it.copy(account = a.account) }
                }
            }
        }

        Label("It may")
        Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SelectChip("Read emails", !draft.send, Modifier.testTag("kind:read")) { viewModel.edit { it.copy(send = false) } }
            SelectChip("Send emails", draft.send, Modifier.testTag("kind:send")) { viewModel.edit { it.copy(send = true, anyMail = false) } }
        }

        if (!draft.send) {
            Label("Which emails?")
            Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SelectChip("Specific senders", !draft.anyMail, Modifier.testTag("scope:specific")) { viewModel.edit { it.copy(anyMail = false) } }
                SelectChip("All mail", draft.anyMail, Modifier.testTag("scope:all")) {
                    viewModel.edit {
                        val lifetime = if (it.lifetime == NewGrantLifetime.ONE_TIME || it.lifetime == NewGrantLifetime.MONTH) NewGrantLifetime.DAY else it.lifetime
                        it.copy(anyMail = true, lifetime = lifetime)
                    }
                }
            }
        }
        if (!draft.anyMail) {
            Label(if (draft.send) "Send to" else "From")
            RTextField(
                draft.partiesText,
                { text -> viewModel.edit { it.copy(partiesText = text.take(1000)) } },
                if (draft.send) "a@b.com, @b.com" else "alerts@bank.com, @bank.com",
                Modifier.padding(horizontal = 16.dp),
                tag = "parties",
                mono = true,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
            )
            RText("Addresses, or a whole domain with @.", RType.sans(12.5f), c.tertiary, Modifier.padding(start = 20.dp, top = 6.dp))
            Label("Subject contains (optional)")
            RTextField(
                draft.subject,
                { text -> viewModel.edit { it.copy(subject = text.take(200)) } },
                "statement",
                Modifier.padding(horizontal = 16.dp),
                tag = "newSubject",
            )
        } else {
            RText(
                "Every email, for as long as you choose (at most 7 days). Sending is never included.",
                RType.sans(14f, lineHeight = 19f),
                c.secondary,
                Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
            )
        }

        Label("For how long?")
        FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            NewGrantLifetime.entries.forEach { kind ->
                val allowed = !draft.anyMail || (kind != NewGrantLifetime.ONE_TIME && kind != NewGrantLifetime.MONTH)
                if (allowed) {
                    SelectChip(kind.label, draft.lifetime == kind, Modifier.testTag("newLifetime:${kind.name}")) {
                        viewModel.edit { it.copy(lifetime = kind) }
                    }
                }
            }
        }
        if (draft.lifetime == NewGrantLifetime.ONE_TIME) {
            RText(
                "It covers one request and stays until then, whenever the AI gets around to asking.",
                RType.sans(13f, lineHeight = 18f),
                c.tertiary,
                Modifier.padding(start = 20.dp, end = 20.dp, top = 8.dp),
            )
        }

        ui.error?.let { Banner(it, Modifier.padding(16.dp), BannerKind.Error) }
        Spacer(Modifier.height(8.dp))
        CapsuleButton(
            "Create grant",
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("createGrant"),
            style = ButtonStyle.Accent,
            busy = ui.busy,
        ) { viewModel.create(authenticator) }
        Spacer(Modifier.height(32.dp))
    }
}

@Composable
private fun Label(text: String) {
    RText(text.uppercase(), RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp), LocalColors.current.secondary, Modifier.padding(start = 20.dp, top = 22.dp, bottom = 8.dp))
}
