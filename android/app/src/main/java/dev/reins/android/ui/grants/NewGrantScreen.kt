package dev.reins.android.ui.grants

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Inbox
import androidx.compose.material.icons.rounded.AlternateEmail
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.LocalColors
import dev.reins.android.design.SelectChip
import dev.reins.android.design.PageDock
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.primaryFill

/** Give an AI a permission before it asks. Rare, so it lives one tap away in Grants, not in the way. */
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

    Box(Modifier.fillMaxSize().background(c.background).imePadding()) {
        Column(Modifier.fillMaxSize()) {
            Row(
                Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars).height(56.dp).padding(start = 6.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier.size(44.dp).pressable(shape = CircleShape, label = "Cancel", onClick = onBack).semantics { contentDescription = "Cancel" }.testTag("back"),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Rounded.Close, null, Modifier.size(24.dp), tint = c.text) }
                Spacer(Modifier.width(6.dp))
                RText("New grant", RType.sans(19f, FontWeight.Medium), c.text, maxLines = 1)
            }
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                Header("For")
                if (connections.isEmpty()) RText("No AI is connected yet.", RType.sans(15f), c.secondary, Modifier.padding(horizontal = 20.dp))
                FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    connections.forEach { conn ->
                        Chip(untrusted(conn.label), draft.connectionId == conn.id, Modifier.testTag("conn:${conn.id}"), leading = {
                            ConnectionIcon(conn.id, conn.label, size = 22.dp)
                        }) { viewModel.edit { it.copy(connectionId = conn.id) } }
                    }
                }

                if (accounts.size != 1) {
                    Header("Account")
                    if (accounts.isEmpty()) {
                        RText("No Gmail account yet. Add one under Integrations.", RType.sans(15f, lineHeight = 20f), c.secondary, Modifier.padding(horizontal = 20.dp).testTag("noAccounts"))
                    }
                    FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        accounts.forEach { a ->
                            Chip(a.account, draft.account == a.account, Modifier.testTag("acct:${a.account}")) { viewModel.edit { it.copy(account = a.account) } }
                        }
                    }
                }

                Header("To")
                KindSegments(draft.send) { send ->
                    viewModel.edit { if (send) it.copy(send = true, anyMail = false) else it.copy(send = false) }
                }

                Header(if (draft.send) "Send to" else "From")
                if (!draft.send) {
                    Row(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Chip("These senders", !draft.anyMail, Modifier.testTag("scope:specific"), leading = { ChipIcon(Icons.Rounded.AlternateEmail, !draft.anyMail) }) { viewModel.edit { it.copy(anyMail = false) } }
                        Chip("All mail", draft.anyMail, Modifier.testTag("scope:all"), leading = { ChipIcon(Icons.Rounded.Inbox, draft.anyMail) }) {
                            viewModel.edit {
                                val lifetime = if (it.lifetime == NewGrantLifetime.ONE_TIME || it.lifetime == NewGrantLifetime.MONTH) NewGrantLifetime.DAY else it.lifetime
                                it.copy(anyMail = true, lifetime = lifetime)
                            }
                        }
                    }
                }
                if (!draft.anyMail) {
                    Field {
                        RTextField(
                            draft.partiesText,
                            { text -> viewModel.edit { it.copy(partiesText = text.take(1000)) } },
                            if (draft.send) "a@b.com, @b.com" else "@stripe.com, receipts@apple.com",
                            tag = "parties",
                            mono = true,
                            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                        )
                    }
                    Spacer(Modifier.height(8.dp))
                    Field {
                        RTextField(
                            draft.subject,
                            { text -> viewModel.edit { it.copy(subject = text.take(200)) } },
                            "Subject contains (optional)",
                            tag = "newSubject",
                        )
                    }
                } else {
                    RText("Up to 7 days. Never sending.", RType.sans(13.5f), c.tertiary, Modifier.padding(horizontal = 22.dp))
                }

                Header("For how long")
                val all = draft.anyMail
                DurationControl(
                    quick = listOfNotNull(
                        DurationChoice(NewGrantLifetime.ONE_TIME, "Once", "newLifetime:ONE_TIME").takeIf { !all },
                        DurationChoice(NewGrantLifetime.HOUR, "1 hour", "newLifetime:HOUR"),
                        DurationChoice(NewGrantLifetime.DAY, "1 day", "newLifetime:DAY"),
                    ),
                    more = listOfNotNull(
                        DurationChoice(NewGrantLifetime.WEEK, "7 days", "newLifetime:WEEK"),
                        DurationChoice(NewGrantLifetime.MONTH, "30 days", "newLifetime:MONTH").takeIf { !all },
                    ),
                    selected = draft.lifetime,
                    modifier = Modifier.padding(horizontal = 16.dp),
                    customTag = "newLifetime:custom",
                ) { kind -> viewModel.edit { it.copy(lifetime = kind) } }
                Spacer(Modifier.height(140.dp))
            }
        }
        PageDock(Modifier.align(Alignment.BottomCenter)) {
            ui.error?.let { Banner(it, Modifier.padding(bottom = 10.dp), BannerKind.Error) }
            val (bg, fg) = primaryFill(c)
            PillButton(
                "Create grant",
                Modifier.fillMaxWidth().testTag("createGrant"),
                icon = Icons.Rounded.Fingerprint,
                background = bg,
                foreground = fg,
                height = 56.dp,
                textSize = 16.5f,
                busy = ui.busy,
            ) { viewModel.create(authenticator) }
        }
    }
}

@Composable
private fun Header(text: String) {
    RText(text, RType.sans(14f, FontWeight.Medium), LocalColors.current.accent, Modifier.padding(start = 22.dp, top = 22.dp, bottom = 10.dp), maxLines = 1)
}

@Composable
private fun Field(content: @Composable () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .border(1.dp, if (c.dark) Color(0xFF1C1C20) else c.hairline, RoundedCornerShape(16.dp)),
    ) { content() }
}

/** A choice chip (the shared [SelectChip]): an optional face or icon, a label; the chosen one tinted with a check. */
@Composable
private fun Chip(label: String, on: Boolean, modifier: Modifier = Modifier, leading: (@Composable () -> Unit)? = null, onClick: () -> Unit) {
    SelectChip(label, on, modifier, leading = leading, onClick = onClick)
}

/** A chip's small icon: violet when chosen. */
@Composable
private fun ChipIcon(icon: ImageVector, on: Boolean) {
    val c = LocalColors.current
    Icon(icon, null, Modifier.size(15.dp), tint = if (on) (if (c.dark) Color(0xFFA99BFA) else c.accent) else c.secondary)
}

/** Read mail | Send mail. */
@Composable
private fun KindSegments(send: Boolean, onChange: (Boolean) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(Modifier.padding(horizontal = 16.dp).fillMaxWidth().height(42.dp), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
        listOf(false to ("Read mail" to Icons.Rounded.Visibility), true to ("Send mail" to Icons.AutoMirrored.Rounded.Send)).forEachIndexed { i, (value, labelIcon) ->
            val (label, icon: ImageVector) = labelIcon
            val on = value == send
            val shape = if (on) {
                RoundedCornerShape(21.dp)
            } else if (i == 0) {
                RoundedCornerShape(topStart = 21.dp, bottomStart = 21.dp, topEnd = 6.dp, bottomEnd = 6.dp)
            } else {
                RoundedCornerShape(topStart = 6.dp, bottomStart = 6.dp, topEnd = 21.dp, bottomEnd = 21.dp)
            }
            Row(
                Modifier
                    .weight(1f)
                    .height(42.dp)
                    .clip(shape)
                    .background(if (on) c.accent.copy(alpha = if (c.dark) 0.24f else 0.16f) else segmentFill(c))
                    .pressable(shape = shape, dim = 0.7f) {
                        if (!on) {
                            onChange(value)
                            feedback.play(Event.Selection)
                        }
                    }
                    .semantics { selected = on }
                    .testTag(if (value) "kind:send" else "kind:read"),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(icon, null, Modifier.size(16.dp), tint = if (on) onAccentWash(c) else c.secondary)
                Spacer(Modifier.width(6.dp))
                RText(label, RType.sans(14f, FontWeight.SemiBold), if (on) onAccentWash(c) else c.secondary, maxLines = 1)
            }
        }
    }
}
