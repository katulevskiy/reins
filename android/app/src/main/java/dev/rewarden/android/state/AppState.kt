package dev.rewarden.android.state

import dev.rewarden.android.platform.PhoneBridge
import dev.rewarden.core.AccountView
import dev.rewarden.core.AutopilotSettings
import dev.rewarden.core.ServiceView
import dev.rewarden.core.ActivityEntry
import dev.rewarden.core.ConnectionView
import dev.rewarden.core.GrantView
import dev.rewarden.core.McpServerView
import dev.rewarden.core.PendingItem
import dev.rewarden.core.SessionInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

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
        dev.rewarden.android.design.McpNames.update(items)
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

    fun setSession(state: SessionState) {
        _session.value = state
        if (state !is SessionState.SignedIn) {
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
            _autopilot.value = null
        }
    }

    fun setPending(items: List<PendingItem>) {
        _pending.value = items
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
}

/** A sign-in to an MCP server finished: [text] says how, [failed] when it did not work. */
data class McpNotice(val serverId: String, val text: String, val failed: Boolean)
