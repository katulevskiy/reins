package dev.reins.android.ui.permissions

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AccountPermissionPreset
import dev.reins.core.AccountPermissionsDraft
import dev.reins.core.AccountPermissionsView
import dev.reins.core.PermissionCapability
import dev.reins.core.DelegationDraft
import dev.reins.core.PermissionMode
import dev.reins.core.PermissionPreview
import dev.reins.core.PermissionScopeOptions
import dev.reins.core.PermissionScopeSelection
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class AccountPermissionsUi(
    val account: AccountPermissionsView? = null,
    val draft: AccountPermissionsDraft? = null,
    val preview: PermissionPreview? = null,
    val options: PermissionScopeOptions? = null,
    val optionsLoading: Boolean = false,
    val loading: Boolean = true,
    val busy: Boolean = false,
    val error: String? = null,
    val changedElsewhere: Boolean = false,
    val saved: Boolean = false,
    val issued: dev.reins.core.DelegationView? = null,
    val hasChosenPreset: Boolean = false,
)

/** Edits typed core drafts. Preset rules, admission, validation and effective policy stay in the core. */
class AccountPermissionsViewModel(
    private val container: AppContainer,
    val service: String,
    val account: String,
    val temporary: Boolean = false,
) : ViewModel() {
    private val epoch = container.state.accountEpoch.value
    private val _ui = MutableStateFlow(AccountPermissionsUi())
    val ui = _ui.asStateFlow()
    val canEdit: Boolean get() = _ui.value.account?.availablePresets?.isNotEmpty() == true
    private var optionsRevision = 0L
    private var visible = true
    private var activation = 0L

    init { reload() }

    private fun current() = visible && container.state.isCurrent(epoch)
    fun activate() {
        if (!visible) { activation++; visible = true; _ui.value = AccountPermissionsUi() }
    }
    fun deactivate() { visible = false; optionsRevision++ }

    /** Reload is explicit after a CAS conflict: a new revision never silently rebases the user's draft. */
    fun reload() = operation { valid ->
        val view = container.core.accountPermissions(service, account)
        if (view.availablePresets.isEmpty()) {
            if (valid()) _ui.value = AccountPermissionsUi(account = view, loading = false)
            return@operation
        }
        val draft = if (temporary) container.core.delegationDraft(service, account).asAccountDraft() else container.core.accountPermissionDraft(service, account, AccountPermissionPreset.CUSTOM)
        if (valid()) _ui.value = if (view.revision == draft.expectedRevision) {
            AccountPermissionsUi(account = view, draft = draft, loading = false)
        } else {
            AccountPermissionsUi(account = view, loading = false, changedElsewhere = true, error = "Permissions changed while loading. Reload to see the current settings.")
        }
    }

    fun preset(preset: AccountPermissionPreset) {
        if (!canEdit) return
        // Customize opens the current core-produced draft; it must not discard an unsaved preset or admission.
        if (preset == AccountPermissionPreset.CUSTOM && _ui.value.hasChosenPreset) {
            edit(customize = true) { it }
            return
        }
        operation { valid ->
            val draft = container.core.accountPermissionDraft(service, account, preset)
            if (valid()) _ui.update {
                if (it.draft?.expectedRevision != draft.expectedRevision) it.copy(changedElsewhere = true, preview = null)
                else it.copy(draft = draft, preview = null, saved = false, hasChosenPreset = true)
            }
        }
    }

    private fun edit(customize: Boolean = false, change: (AccountPermissionsDraft) -> AccountPermissionsDraft) {
        val old = _ui.value
        if (!current() || !canEdit || old.busy || old.changedElsewhere) return
        val draft = old.draft ?: return
        val changed = change(draft)
        val next = if (customize) changed.copy(preset = AccountPermissionPreset.CUSTOM) else changed
        _ui.update { it.copy(draft = next, preview = null, saved = false, error = null, hasChosenPreset = true) }
    }

    fun include(capability: PermissionCapability, selected: Boolean) = edit { draft ->
        val rule = _ui.value.account?.capabilities?.firstOrNull { it.capability == capability } ?: return@edit draft
        draft.copy(rules = draft.rules.filterNot { it.capability == capability } + if (selected) listOf(dev.reins.core.CapabilityPermissionInput(capability, PermissionMode.AUTO, rule.scope)) else emptyList())
    }

    fun mode(capability: PermissionCapability, mode: PermissionMode) = edit(customize = true) { draft ->
        draft.copy(rules = draft.rules.map { if (it.capability == capability) it.copy(mode = mode) else it })
    }

    fun scope(capability: PermissionCapability, scope: PermissionScopeSelection) = edit(customize = true) { draft ->
        draft.copy(rules = draft.rules.map { if (it.capability == capability) it.copy(scope = scope) else it })
    }

    fun agent(id: String, selected: Boolean) = edit { draft ->
        val ids = if (selected) (draft.agentIds + id).distinct() else draft.agentIds - id
        val existing = _ui.value.account?.agents.orEmpty().filter { it.hasAccountAccess }.map { it.connectionId }.toSet()
        // Checking a previously unadmitted agent is explicit admission, shown again by core preview and owner check.
        draft.copy(agentIds = ids, grantAccountAccess = ids.any { it !in existing })
    }

    fun expiresAt(value: Long?) = edit { it.copy(expiresAt = value) }
    fun remainingUses(value: ULong?) = edit { it.copy(maxUses = value) }
    fun discardPreview() { _ui.update { it.copy(preview = null) } }
    fun closeOptions() { optionsRevision++; _ui.update { it.copy(options = null, optionsLoading = false) } }

    /** Search and typed address normalization both return selectable canonical IDs from the account-bound core. */
    fun scopeOptions(capability: PermissionCapability, query: String) {
        if (!current() || !canEdit) return
        val revision = ++optionsRevision
        _ui.update { it.copy(optionsLoading = true) }
        viewModelScope.launch {
            try {
                kotlinx.coroutines.delay(250)
                val options = container.core.permissionScopeOptions(service, account, capability, query, 30u)
                if (current() && revision == optionsRevision) _ui.update { it.copy(options = options, optionsLoading = false, error = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (current() && revision == optionsRevision) _ui.update { it.copy(optionsLoading = false, error = e.userMessage()) }
            }
        }
    }

    fun preview() = operation { valid ->
        if (!canEdit) return@operation
        val draft = _ui.value.draft ?: return@operation
        val preview = if (temporary) container.core.previewDelegation(draft.asDelegationDraft()) else container.core.previewAccountPermissions(draft)
        if (valid()) _ui.update { it.copy(preview = preview) }
    }

    /** Only the exact draft the owner previewed is submitted. Editing clears the preview. */
    fun save(authenticator: Authenticator) {
        if (!canEdit) return
        val state = _ui.value
        val draft = state.draft ?: return
        if (state.preview?.revision != draft.expectedRevision || state.changedElsewhere) return
        val token = activation
        val owner = OwnerPrompt(Authenticator { title, reason ->
            if (current() && activation == token) {
                val result = authenticator.authenticate(title, reason)
                if (current() && activation == token) result else AuthResult.Cancelled
            } else AuthResult.Cancelled
        }, "Save account permissions")
        operation(owner) { valid ->
            if (temporary) {
                val issued = container.core.issueDelegation(draft.asDelegationDraft(), owner)
                if (valid()) _ui.update { it.copy(preview = null, saved = true, issued = issued, error = null) }
            } else {
                val saved = container.core.saveAccountPermissions(draft, owner)
                if (valid()) _ui.update { it.copy(account = saved, preview = null, saved = true, error = null) }
            }
        }
    }

    private fun operation(owner: OwnerPrompt? = null, block: suspend (() -> Boolean) -> Unit) {
        if (!current() || _ui.value.busy) return
        val token = activation
        val valid = { current() && activation == token }
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                block(valid)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (valid()) failed(e, owner, valid)
            } finally {
                if (valid()) _ui.update { it.copy(busy = false, loading = false) }
            }
        }
    }

    private suspend fun failed(error: Exception, owner: OwnerPrompt?, valid: () -> Boolean) {
        val message = when (owner?.last) {
            AuthResult.Cancelled -> null
            AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone to change permissions."
            else -> error.userMessage()
        }
        val latest = try { container.core.accountPermissions(service, account) } catch (e: CancellationException) { throw e } catch (_: Exception) { null }
        if (!valid()) return
        _ui.update { state -> state.copy(
            error = message,
            preview = null,
            changedElsewhere = state.draft != null && latest != null && latest.revision != state.draft.expectedRevision,
        ) }
    }
}

private fun DelegationDraft.asAccountDraft() = AccountPermissionsDraft(service, account, expectedRevision, AccountPermissionPreset.CUSTOM, agentIds, grantAccountAccess, expiresAt, maxUses, rules)
private fun AccountPermissionsDraft.asDelegationDraft(): DelegationDraft {
    val expiry = expiresAt ?: throw dev.reins.core.CoreException.Invalid("Set an expiry for this temporary permission.")
    val uses = maxUses ?: throw dev.reins.core.CoreException.Invalid("Set a use limit for this temporary permission.")
    return DelegationDraft(service, account, expectedRevision, agentIds, grantAccountAccess, rules, expiry, uses)
}
