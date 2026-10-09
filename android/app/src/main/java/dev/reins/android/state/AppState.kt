package dev.reins.android.state

import dev.reins.android.platform.PhoneBridge
import dev.reins.core.AccountView
import dev.reins.core.AutopilotSettings
import dev.reins.core.ServiceView
import dev.reins.core.ActivityEntry
import dev.reins.core.ConnectionView
import dev.reins.core.GrantView
import dev.reins.core.McpServerView
import dev.reins.core.PendingItem
import dev.reins.core.SessionInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

sealed interface SessionState {
    data object Loading : SessionState

    data object SignedOut : SessionState

    data class SignedIn(val info: SessionInfo) : SessionState
}

/** Process-wide UI state that several screens and background paths share. */
class AppState {
    private val _session = MutableStateFlow<SessionState>(SessionState.Loading)
    val session: StateFlow<SessionState> = _session.asStateFlow()

    private val _pending = MutableStateFlow<List<PendingItem>>(emptyList())
    val pending: StateFlow<List<PendingItem>> = _pending.asStateFlow()

    /** Everything that happened, newest first. */
    private val _activity = MutableStateFlow<List<ActivityEntry>>(emptyList())
    val activity: StateFlow<List<ActivityEntry>> = _activity.asStateFlow()

    private val _grants = MutableStateFlow<List<GrantView>>(emptyList())
    val grants: StateFlow<List<GrantView>> = _grants.asStateFlow()

    /** The accounts the user connected (Gmail addresses), oldest first. */
    private val _services = MutableStateFlow<List<ServiceView>>(emptyList())
    val services: StateFlow<List<ServiceView>> = _services.asStateFlow()

    private val _accounts = MutableStateFlow<List<AccountView>>(emptyList())
    val accounts: StateFlow<List<AccountView>> = _accounts.asStateFlow()

    /** The MCP servers the user added, oldest first. */
    private val _mcpServers = MutableStateFlow<List<McpServerView>>(emptyList())
    val mcpServers: StateFlow<List<McpServerView>> = _mcpServers.asStateFlow()

    /** How the last MCP sign-in ended, shown on that server's page until it is left. */
    private val _mcpNotice = MutableStateFlow<McpNotice?>(null)
    val mcpNotice: StateFlow<McpNotice?> = _mcpNotice.asStateFlow()

    fun setMcpServers(items: List<McpServerView>) {
        _mcpServers.value = items
        dev.reins.android.design.McpNames.update(items)
    }

    fun setMcpNotice(notice: McpNotice?) {
        _mcpNotice.value = notice
    }

    /** Autopilot's modes, bypasses and model; null until first read. */
    private val _autopilot = MutableStateFlow<AutopilotSettings?>(null)
    val autopilot: StateFlow<AutopilotSettings?> = _autopilot.asStateFlow()

    fun setAutopilot(settings: AutopilotSettings?) {
        _autopilot.value = settings
    }

    private val _connections = MutableStateFlow<List<ConnectionView>>(emptyList())
    val connections: StateFlow<List<ConnectionView>> = _connections.asStateFlow()

    /** Id of the newest activity entry the user has looked at. */
    private val _seenActivityId = MutableStateFlow(0L)
    val seenActivityId: StateFlow<Long> = _seenActivityId.asStateFlow()

    /** True once the server told this phone that another phone became the approval device. */
    private val _deviceReplaced = MutableStateFlow(false)
    val deviceReplaced: StateFlow<Boolean> = _deviceReplaced.asStateFlow()

    /** This phone is the one that currently receives approval requests. */
    private val _approvalDevice = MutableStateFlow(false)
    val approvalDevice: StateFlow<Boolean> = _approvalDevice.asStateFlow()

    /**
     * Signed in, but this phone cannot open the account's keys yet: the Unlock screen shows instead of the app, and the
     * phone is not registered as the approval device (the phone that has the keys must approve this one).
     */
    private val _keysLocked = MutableStateFlow(false)
    val keysLocked: StateFlow<Boolean> = _keysLocked.asStateFlow()

    fun setKeysLocked(value: Boolean) {
        _keysLocked.value = value
    }

    /**
     * Registering this phone as the approval device was refused: another phone approves for the account. The Unlock
     * screen shows instead of the app with the two ways to take over (approve from that phone, or the recovery code),
     * until it succeeds or the user puts it off.
     */
    private val _approvalTakeover = MutableStateFlow(false)
    val approvalTakeover: StateFlow<Boolean> = _approvalTakeover.asStateFlow()

    fun setApprovalTakeover(value: Boolean) {
        _approvalTakeover.value = value
    }

    /** Why registering this phone as the approval device failed, until it succeeds. */
    private val _registrationError = MutableStateFlow<String?>(null)
    val registrationError: StateFlow<String?> = _registrationError.asStateFlow()

    fun setRegistrationError(message: String?) {
        _registrationError.value = message
    }

    /** The setup after a fresh sign-in (connect a computer, connect an AI) is showing instead of the main screen. */
    private val _setupPending = MutableStateFlow(false)
    val setupPending: StateFlow<Boolean> = _setupPending.asStateFlow()

    fun setSetupPending(value: Boolean) {
        _setupPending.value = value
    }

    /** A passwordless account must record its recovery code before the app's normal screens appear. Never persisted. */
    private val _recoveryToRecord = MutableStateFlow<String?>(null)
    val recoveryToRecord: StateFlow<String?> = _recoveryToRecord.asStateFlow()

    fun setRecoveryToRecord(code: String?) {
        _recoveryToRecord.value = code
    }

    /**
     * Before the recovery code, an account without a passkey for its vault is offered one: it opens the vault on a new
     * phone without the other phone or the code. Never persisted.
     */
    private val _vaultPasskeyOffer = MutableStateFlow(false)
    val vaultPasskeyOffer: StateFlow<Boolean> = _vaultPasskeyOffer.asStateFlow()

    /** "Use the recovery code only": the offer stays away until the account changes or the app restarts. */
    @Volatile var vaultPasskeyDeclined = false
        private set

    fun setVaultPasskeyOffer(value: Boolean) {
        _vaultPasskeyOffer.value = value && !vaultPasskeyDeclined
    }

    fun declineVaultPasskey() {
        vaultPasskeyDeclined = true
        _vaultPasskeyOffer.value = false
    }

    private val _recoveryLoadError = MutableStateFlow<String?>(null)
    val recoveryLoadError: StateFlow<String?> = _recoveryLoadError.asStateFlow()
    fun setRecoveryLoadError(error: String?) { _recoveryLoadError.value = error }

    var onAccountChange: (() -> Unit)? = null
    private val _accountEpoch = MutableStateFlow(0L)
    val accountEpoch: StateFlow<Long> = _accountEpoch.asStateFlow()
    fun isCurrent(epoch: Long) = epoch == _accountEpoch.value

    fun setSession(state: SessionState, beforePublish: () -> Unit = {}) {
        val previous = (_session.value as? SessionState.SignedIn)?.info
        val next = (state as? SessionState.SignedIn)?.info
        val changed = previous?.let { it.serverUrl to it.email } != next?.let { it.serverUrl to it.email }
        if (changed) {
            _accountEpoch.value += 1
            onAccountChange?.invoke()
        }
        if (state !is SessionState.SignedIn || (previous != null && changed)) {
            _seenActivityId.value = 0L
            _deviceReplaced.value = false
            _registrationError.value = null
            _recoveryToRecord.value = null
            _recoveryLoadError.value = null
            _vaultPasskeyOffer.value = false
            vaultPasskeyDeclined = false
            _setupPending.value = false
            _pending.value = emptyList()
            _activity.value = emptyList()
            _grants.value = emptyList()
            _accounts.value = emptyList()
            _services.value = emptyList()
            _connections.value = emptyList()
            setMcpServers(emptyList())
            _mcpNotice.value = null
            _approvalDevice.value = false
            _keysLocked.value = false
            _approvalTakeover.value = false
            _autopilot.value = null
        }
        beforePublish()
        _session.value = state
    }

    fun setPending(items: List<PendingItem>) {
        val gone = synchronized(answered) { answered.toSet() }
        _pending.value = if (gone.isEmpty()) items else items.filter { it.id !in gone }
    }

    /**
     * Answered items, kept out of [pending] even by a refresh that read the list just before the answer, so an
     * approved card never comes back for a moment. The last few are enough: the core forgets an item once answered.
     */
    private val answered = LinkedHashSet<String>()

    /** The user answered [id]: its card goes at once, without waiting for the refresh behind it. */
    fun removePending(id: String) {
        synchronized(answered) {
            answered += id
            if (answered.size > ANSWERED_KEPT) answered.remove(answered.first())
        }
        _pending.update { items -> items.filter { it.id != id } }
    }

    fun setActivity(items: List<ActivityEntry>) {
        _activity.value = items
    }

    fun setGrants(items: List<GrantView>) {
        _grants.value = items
    }

    /** The core lists every integration; what this build does not offer at all (text messages on Google Play) is left out. */
    fun setServices(items: List<ServiceView>) {
        _services.value = items.filter { PhoneBridge.offers(it.service) }
    }

    fun setAccounts(items: List<AccountView>) {
        _accounts.value = items
    }

    fun setConnections(items: List<ConnectionView>) {
        _connections.value = items
    }

    fun setSeenActivityId(id: Long) {
        if (id > _seenActivityId.value) _seenActivityId.value = id
    }

    fun setDeviceReplaced(replaced: Boolean) {
        _deviceReplaced.value = replaced
    }

    fun setApprovalDevice(value: Boolean) {
        _approvalDevice.value = value
    }

    /** Entries newer than the last one the user looked at. */
    fun unseenActivity(): Int {
        val seen = _seenActivityId.value
        return _activity.value.count { it.id > seen }
    }

    private companion object {
        const val ANSWERED_KEPT = 64
    }
}

/** A sign-in to an MCP server finished: [text] says how, [failed] when it did not work. */
data class McpNotice(val serverId: String, val text: String, val failed: Boolean)
