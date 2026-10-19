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
    Screen("Permissions", onBack = onBack) {
        Group(header = "Accounts", footer = "These settings also appear in Connections. Automatic permissions apply only to their selected agents and account. New agents need explicit account access.") {
            accounts.forEach { account ->
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    RText(untrusted(account.account), RType.sans(16f), c.ink)
                    CapsuleButton("View ${serviceName(account.service)} permissions", Modifier.fillMaxWidth().testTag("managePermissions:${account.service}:${account.account}"), style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { onAccount(account.service, account.account) }
                }
            }
        }
        Group(header = "Temporary permissions", footer = "Open an account with supported automatic permissions above to create a temporary permission. Unknown outcomes still use the reserved budget.") {
            if (delegations.isEmpty()) RText("No temporary permissions", RType.sans(15f), c.ink2, Modifier.padding(16.dp))
            delegations.forEach { record ->
                Column(Modifier.padding(16.dp).testTag("delegation:${record.id}"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    RText(untrusted(record.title), RType.serif(22f), c.ink)
                    RText("${delegationStateTitle(record.state)} · ${untrusted(record.account)}", RType.sans(14f), c.ink)
                    RText(untrusted(record.scopeSummary), RType.sans(14f), c.ink)
                    RText((if (record.usageVerified) "${record.used} of ${record.maxUses} uses · ${record.unresolved} unresolved outcomes" else "Usage and quota unavailable") + "\nExpires ${permissionExpiry(record.expiresAt)}", RType.sans(13f), c.ink2)
                    RText(untrusted(record.effectiveReason), RType.sans(14f), c.ink2)
                    PermissionRestrictions(record.restrictions)
                    if (record.state == DelegationState.ACTIVE) CapsuleButton("Pause", Modifier.fillMaxWidth().testTag("pauseDelegation:${record.id}"), enabled = !busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { confirming = record to DelegationChange.PAUSE }
                    if (record.state == DelegationState.PAUSED) CapsuleButton("Resume", Modifier.fillMaxWidth().testTag("resumeDelegation:${record.id}"), enabled = !busy && record.usageVerified && record.restrictions.none { it.controller == PermissionController.CONFLICT || it.controller == PermissionController.UNSUPPORTED_ADAPTER }, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { confirming = record to DelegationChange.RESUME }
                    if (record.state != DelegationState.REVOKED && record.state != DelegationState.EXPIRED) CapsuleButton("Revoke", Modifier.fillMaxWidth().testTag("revokeDelegation:${record.id}"), enabled = !busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { confirming = record to DelegationChange.REVOKE }
                }
                Hairline()
            }
        }
        error?.let { Banner(untrusted(it), Modifier.padding(16.dp), BannerKind.Error, tag = "delegationError") }
        CapsuleButton("Reload", Modifier.fillMaxWidth().padding(16.dp), enabled = !busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { scope.launch { reload() } }
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
