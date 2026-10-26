package dev.reins.android.ui.permissions

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.*
import dev.reins.android.platform.*
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Both entry points read the canonical records; revisions are sent back unchanged on every mutation. */
@Composable
fun PermissionsHomeScreen(container: AppContainer, authenticator: Authenticator, onBack: () -> Unit, onAccount: (String, String) -> Unit) {
    val epoch = remember { container.state.accountEpoch.value }
    var visible by remember { mutableStateOf(true) }
    DisposableEffect(Unit) { onDispose { visible = false } }
    val scope = rememberCoroutineScope()
    val c = LocalColors.current
    var accounts by remember { mutableStateOf<List<AccountView>>(emptyList()) }
    var delegations by remember { mutableStateOf<List<DelegationView>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var confirming by remember { mutableStateOf<Pair<DelegationView, DelegationChange>?>(null) }
    suspend fun reload() {
        try {
            val all = container.core.services().filter { PhoneBridge.offers(it.service) }.flatMap { it.accounts } + container.core.accounts()
            val records = container.core.delegations()
            if (container.state.isCurrent(epoch)) { accounts = all.distinctBy { it.service to it.account }; delegations = records; error = null }
        } catch (e: CancellationException) { throw e } catch (e: Exception) { if (container.state.isCurrent(epoch)) error = e.userMessage() }
    }
    LaunchedEffect(epoch) { reload() }
    // Next (Permissions 01 and 05): the accounts as quiet rows that open their rules, then each temporary permission
    // with its facts and Pause, Resume or Revoke.
    Screen("Permissions", onBack = onBack, actions = {
        RoundIconButton(Glyph.Refresh, "Reload", Modifier.testTag("reloadPermissions"), onClick = { if (!busy) scope.launch { reload() } })
    }) {
        Group(header = "Accounts", footer = "These settings also appear in Connections. Automatic permissions apply only to their selected agents and account. New agents need explicit account access.") {
            accounts.forEachIndexed { i, account ->
                if (i > 0) Hairline()
                PaneRow(
                    untrusted(account.account),
                    leading = { BrandTile(account.service, size = 32.dp) },
                    titleMono = true,
                    chevron = true,
                    first = true,
                    flat = true,
                    tag = "managePermissions:${account.service}:${account.account}",
                ) { onAccount(account.service, account.account) }
            }
        }
        Group(header = "Temporary", footer = "Open an account with supported automatic permissions above to create a temporary permission. Unknown outcomes still use the reserved budget.") {
            if (delegations.isEmpty()) RText("No temporary permissions", RType.sans(15f), c.ink2, Modifier.padding(vertical = 16.dp))
            delegations.forEachIndexed { i, record ->
                if (i > 0) Hairline(inset = 0.dp)
                Column(Modifier.padding(vertical = 12.dp).testTag("delegation:${record.id}"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        RText(untrusted(record.account), RType.row, c.ink, Modifier.weight(1f), maxLines = 1)
                        NextPill(
                            delegationStateTitle(record.state),
                            when (record.state) {
                                DelegationState.ACTIVE -> c.success
                                DelegationState.PAUSED -> c.warning
                                else -> null
                            },
                        )
                    }
                    RText(untrusted(record.title), RType.sans(14f), c.ink2)
                    RText(untrusted(record.scopeSummary), RType.sans(14f), c.ink2)
                    RText((if (record.usageVerified) "${record.used} / ${record.maxUses} uses · ${record.unresolved} unknown outcomes" else "Usage and quota unavailable") + " · Expires ${permissionExpiry(record.expiresAt)}", RType.meta, c.inkMeta)
                    RText(untrusted(record.effectiveReason), RType.meta, c.inkMeta)
                    PermissionRestrictions(record.restrictions)
                    Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (record.state == DelegationState.ACTIVE) SignalButton("Pause", Modifier.weight(1f).testTag("pauseDelegation:${record.id}"), style = ButtonStyle.Soft, size = ButtonSize.Small, glyph = Glyph.Pause, enabled = !busy) { confirming = record to DelegationChange.PAUSE }
                        if (record.state == DelegationState.PAUSED) SignalButton("Resume", Modifier.weight(1f).testTag("resumeDelegation:${record.id}"), style = ButtonStyle.Soft, size = ButtonSize.Small, glyph = Glyph.Play, enabled = !busy && record.usageVerified && record.restrictions.none { it.controller == PermissionController.CONFLICT || it.controller == PermissionController.UNSUPPORTED_ADAPTER }) { confirming = record to DelegationChange.RESUME }
                        if (record.state != DelegationState.REVOKED && record.state != DelegationState.EXPIRED) SignalButton("Revoke", Modifier.weight(1f).testTag("revokeDelegation:${record.id}"), style = ButtonStyle.DangerWash, size = ButtonSize.Small, enabled = !busy) { confirming = record to DelegationChange.REVOKE }
                    }
                }
            }
        }
        error?.let { Banner(untrusted(it), Modifier.padding(16.dp), BannerKind.Error, tag = "delegationError") }
        Spacer(Modifier.height(32.dp))
    }
    confirming?.let { (record, change) ->
        ConfirmDialog("${delegationChangeTitle(change)} temporary permission?", "${untrusted(record.scopeSummary)}\n${untrusted(record.account)}", delegationChangeTitle(change), onConfirm = {
            confirming = null
            busy = true
            scope.launch {
                val owner = OwnerPrompt(Authenticator { title, reason -> if (visible && container.state.isCurrent(epoch)) {
                    val result = authenticator.authenticate(title, reason)
                    if (visible && container.state.isCurrent(epoch)) result else AuthResult.Cancelled
                } else AuthResult.Cancelled }, "Change temporary permission")
                try {
                    container.core.changeDelegation(record.id, record.revision, change, owner)
                    if (container.state.isCurrent(epoch)) reload()
                } catch (e: CancellationException) { throw e } catch (e: Exception) {
                    if (container.state.isCurrent(epoch) && owner.last != AuthResult.Cancelled) error = e.userMessage() + " Reload before trying again."
                } finally { if (container.state.isCurrent(epoch)) busy = false }
            }
        }, onDismiss = { confirming = null })
    }
}
fun delegationStateTitle(state: DelegationState): String = when (state) {
    DelegationState.ACTIVE -> "Active"
    DelegationState.PAUSED -> "Paused"
    DelegationState.EXPIRED -> "Expired"
    DelegationState.REVOKED -> "Revoked"
}
fun delegationChangeTitle(change: DelegationChange): String = when (change) {
    DelegationChange.RESUME -> "Resume"
    DelegationChange.PAUSE -> "Pause"
    DelegationChange.REVOKE -> "Revoke"
}
