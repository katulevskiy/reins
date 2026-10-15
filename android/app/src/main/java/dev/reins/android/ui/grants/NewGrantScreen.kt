package dev.reins.android.ui.grants

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.AskText
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.Glyph
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Toggle
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.AppState
import dev.reins.android.ui.approval.AskDock
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.untrusted

/**
 * Give an AI a permission before it asks (Reins Signal, iOS-Trust 03): a paper form that reads like a sentence, for
 * Claude, to read mail, from Stripe, for a day. Nothing is asking, so it is paper, not a field. All mail flips the
 * address box to "Up to 7 days. Never sending." Rare, so it lives one tap away on Trust.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun NewGrantScreen(viewModel: NewGrantViewModel, state: AppState, authenticator: Authenticator, onBack: () -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    // A grant made here is for mail: the vault and the other services' accounts are not offered.
    val allAccounts by state.accounts.collectAsStateWithLifecycle()
    val accounts = remember(allAccounts) { allAccounts.filter { it.service == "gmail" } }
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

    Column(Modifier.fillMaxSize().background(c.paper).imePadding()) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 22.dp)) {
            RText(
                "Cancel",
                RType.sans(15.5f),
                c.ink2,
                Modifier
                    .padding(top = 10.dp)
                    .pressable(shape = RoundedCornerShape(10.dp), label = "Cancel", onClick = onBack)
                    .semantics { contentDescription = "Cancel" }
                    .testTag("back")
                    .padding(vertical = 8.dp),
                maxLines = 1,
            )
            AskText("New grant", c.ink, Modifier.padding(top = 14.dp).semantics { heading() }, target = "grant", style = RType.serif(40f, 44f), minSize = 32.sp)

            Header("For")
            if (connections.isEmpty()) RText("No AI is connected yet.", RType.sans(15f), c.ink2)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                connections.forEach { conn ->
                    Chip(untrusted(conn.label), draft.connectionId == conn.id, Modifier.testTag("conn:${conn.id}"), leading = {
                        CompositionLocalProvider(LocalColors provides RColors.Light) { ConnectionIcon(conn.id, conn.label, size = 20.dp) }
                    }) { viewModel.edit { it.copy(connectionId = conn.id) } }
                }
            }

            if (accounts.size != 1) {
                Header("Account")
                if (accounts.isEmpty()) {
                    RText("No Gmail account yet. Add one under Connections.", RType.sans(15f, lineHeight = 20f), c.ink2, Modifier.testTag("noAccounts"))
                }
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    accounts.forEach { a ->
                        Chip(a.account, draft.account == a.account, Modifier.testTag("acct:${a.account}")) { viewModel.edit { it.copy(account = a.account) } }
                    }
                }
            }

            Header("To")
            Chooser(
                listOf(Choice(false, "Read mail", "kind:read"), Choice(true, "Send mail", "kind:send")),
                draft.send,
                { send -> viewModel.edit { if (send) it.copy(send = true, anyMail = false) else it.copy(send = false) } },
            )

            Row(verticalAlignment = Alignment.CenterVertically) {
                Header(if (draft.send) "Send to" else "From", Modifier.weight(1f))
                if (!draft.send) {
                    Row(
                        Modifier
                            .padding(top = 12.dp)
                            .pressable(shape = CircleShape) { setAllMail(viewModel, !draft.anyMail) }
                            .semantics(mergeDescendants = true) {
                                selected = draft.anyMail
                                role = Role.Switch
                            }
                            .testTag("scope:all"),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        RText("All mail", RType.sans(14f), c.ink2, maxLines = 1)
                        Toggle(draft.anyMail) { on -> setAllMail(viewModel, on) }
                    }
                }
            }
            if (!draft.anyMail) {
                RTextField(
                    draft.partiesText,
                    { text -> viewModel.edit { it.copy(partiesText = text.take(1000)) } },
                    if (draft.send) "a@b.com, @b.com" else "@stripe.com, receipts@apple.com",
                    tag = "parties",
                    mono = true,
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                )
                Spacer(Modifier.height(8.dp))
                RTextField(
                    draft.subject,
                    { text -> viewModel.edit { it.copy(subject = text.take(200)) } },
                    "Subject contains (optional)",
                    tag = "newSubject",
                )
            } else {
                RText("Up to 7 days. Never sending.", RType.meta, c.inkMeta, Modifier.testTag("allMailNote"))
            }

            Header("For how long")
            val all = draft.anyMail
            Chooser(
                listOfNotNull(
                    Choice(NewGrantLifetime.ONE_TIME, "Once", "newLifetime:ONE_TIME").takeIf { !all },
                    Choice(NewGrantLifetime.HOUR, "1 hour", "newLifetime:HOUR"),
                    Choice(NewGrantLifetime.DAY, "1 day", "newLifetime:DAY"),
                ),
                draft.lifetime,
                { kind -> viewModel.edit { it.copy(lifetime = kind) } },
                more = listOfNotNull(
                    Choice(NewGrantLifetime.WEEK, "7 days", "newLifetime:WEEK"),
                    Choice(NewGrantLifetime.MONTH, "30 days", "newLifetime:MONTH").takeIf { !all },
                ),
                moreTag = "newLifetime:custom",
            )
            Spacer(Modifier.height(32.dp))
        }
        AskDock {
            ui.error?.let { RText(it, RType.sans(13.5f, FontWeight.Medium, lineHeight = 18f), c.danger, Modifier.fillMaxWidth().testTag("newGrantError"), maxLines = 3) }
            SignalButton("Create grant", Modifier.fillMaxWidth().testTag("createGrant"), glyph = Glyph.Fingerprint, enabled = !ui.busy) { viewModel.create(authenticator) }
        }
    }
}

/** All mail on: a time limit of a week at most, never once. Off: back to the address box. */
private fun setAllMail(viewModel: NewGrantViewModel, on: Boolean) = viewModel.edit {
    if (on) {
        val lifetime = if (it.lifetime == NewGrantLifetime.ONE_TIME || it.lifetime == NewGrantLifetime.MONTH) NewGrantLifetime.DAY else it.lifetime
        it.copy(anyMail = true, lifetime = lifetime)
    } else {
        it.copy(anyMail = false)
    }
}

@Composable
private fun Header(text: String, modifier: Modifier = Modifier) {
    LabelText(text, LocalColors.current.inkMeta, modifier.padding(top = 26.dp, bottom = 10.dp))
}

/** A choice chip: the AI's mark and its name; the chosen one is inverse. */
@Composable
private fun Chip(label: String, on: Boolean, modifier: Modifier = Modifier, leading: (@Composable () -> Unit)? = null, onClick: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        modifier
            .height(40.dp)
            .clip(CircleShape)
            .background(if (on) c.inverse else c.paperRaised)
            .then(if (on) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
            .pressable(shape = CircleShape) {
                if (!on) feedback.play(Event.Selection)
                onClick()
            }
            .semantics { selected = on }
            .padding(start = if (leading != null) 8.dp else 16.dp, end = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        leading?.invoke()
        RText(label, RType.sans(14.5f, FontWeight.Medium), if (on) c.onInverse else c.ink, maxLines = 1)
    }
}
