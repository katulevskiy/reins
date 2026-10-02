package dev.rewarden.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.rewarden.android.AppContainer
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.play
import dev.rewarden.android.platform.Foreground
import dev.rewarden.android.platform.McpSignInResult
import dev.rewarden.android.state.McpNotice
import dev.rewarden.android.state.SessionState
import dev.rewarden.android.ui.nav.DeepLink
import dev.rewarden.android.ui.nav.Route
import dev.rewarden.android.ui.nav.SheetTarget
import dev.rewarden.android.ui.nav.Tab
import dev.rewarden.core.CoreException
import dev.rewarden.core.PendingItem
import dev.rewarden.core.PendingKind
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Navigation and session lifecycle for the whole activity. */
class AppViewModel(private val container: AppContainer) : ViewModel() {
    var tab by mutableStateOf(Tab.Activity)
        private set

    /** Screens above the tabs; empty means a tab is showing. */
    val stack = mutableStateListOf<Route>()

    private val _sheet = MutableStateFlow<SheetTarget?>(null)
    val sheet: StateFlow<SheetTarget?> = _sheet.asStateFlow()

    private val _notice = MutableStateFlow<String?>(null)
    val notice: StateFlow<String?> = _notice.asStateFlow()

    /** Items already popped up once; dismissing one leaves it in the list without popping up again. */
    private val presented = mutableSetOf<String>()

    val current: Route? get() = stack.lastOrNull()

    init {
        // Not immediate: state changes must not resume collectors (e.g. the deep-link handler, which looks this
        // view model up) while the view model is still being constructed.
        viewModelScope.launch(Dispatchers.Main) { container.refreshSession() }
        viewModelScope.launch(Dispatchers.Main) {
            container.state.pending.collect { maybePresent(it) }
        }
    }

    fun selectTab(target: Tab) {
        tab = target
        stack.clear()
    }

    /** Whether the "Expired" list on the Grants tab is open; closed until the user opens it, and kept while they browse. */
    var expiredOpen by mutableStateOf(false)
        private set

    fun toggleExpired() {
        expiredOpen = !expiredOpen
    }

    /** Changes for every visit to "New grant", so the form always starts empty. */
    var newGrantToken by mutableIntStateOf(0)
        private set

    fun open(route: Route) {
        if (route == Route.NewGrant) newGrantToken++
        if (stack.lastOrNull() != route) stack.add(route)
    }

    fun back(): Boolean {
        if (stack.isEmpty()) return false
        stack.removeAt(stack.lastIndex)
        return true
    }

    fun home() {
        stack.clear()
        tab = Tab.Activity
    }

    fun dismissNotice() {
        _notice.value = null
    }

    /** Opens the sheet for [target]; it will not pop up by itself again. */
    fun openSheet(target: SheetTarget) {
        presented += target.id
        _sheet.value = target
    }

    fun closeSheet() {
        _sheet.value = null
        maybePresent(container.state.pending.value)
    }

    /** The app came to the front: pop up whatever is waiting. */
    fun onForeground() {
        maybePresent(container.state.pending.value)
    }

    private fun maybePresent(items: List<PendingItem>) {
        if (!Foreground.focused || !Foreground.autoPopup || _sheet.value != null) return
        // Drop ids that are gone so the set stays small.
        presented.retainAll(items.map { it.id }.toSet() + _sheet.value?.id.orEmpty())
        val next = items.firstOrNull { it.id !in presented } ?: return
        openSheet(next.toTarget())
    }

    /** An Autopilot notification's "Report": the activity entry it made. */
    fun openActivityEntry(id: Long) {
        viewModelScope.launch {
            container.refreshPending()
            home()
            open(Route.ActivityDetail(id))
        }
    }

    /** The bypass notification or the download's: Settings > Autopilot. */
    fun openAutopilot() {
        home()
        open(Route.Settings)
        open(Route.Autopilot)
    }

    /** A "grant ends soon" notification was tapped: show that grant, if it exists. */
    fun openGrant(id: String) {
        viewModelScope.launch {
            container.refreshPending()
            home()
            selectTab(Tab.Grants)
            if (container.state.grants.value.any { it.id == id }) open(Route.GrantDetail(id))
        }
    }

    /** Opens the sheet a notification pointed at, if (and only if) that item is really waiting. */
    fun handleDeepLink(link: DeepLink) {
        viewModelScope.launch {
            val route = try {
                link.resolve(container.core.pending())
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException) {
                null
            }
            home()
            if (route == null) {
                _notice.value = "That request is no longer waiting."
            } else {
                openSheet(route)
            }
        }
    }

    /** An MCP server's sign-in page came back to the app: finish the sign-in and show that server. */
    fun finishMcpSignIn(redirect: String) {
        viewModelScope.launch {
            val (id, notice) = when (val result = container.mcpSignIn.finish(redirect)) {
                McpSignInResult.Ignored -> return@launch
                is McpSignInResult.Done ->
                    result.server.id to McpNotice(result.server.id, "Signed in to ${result.server.name}. Its tools can be used now.", failed = false)
                is McpSignInResult.Failed ->
                    result.serverId to McpNotice(result.serverId, "Signing in did not work: ${result.message}", failed = true)
            }
            container.feedback.play(if (notice.failed) Event.Error else Event.Connected)
            container.state.setMcpNotice(notice)
            home()
            open(Route.Integrations)
            open(Route.McpServer(id))
        }
    }

    fun signedOut() {
        container.state.setSession(SessionState.SignedOut)
        home()
        _sheet.value = null
    }
}

fun PendingItem.toTarget(): SheetTarget = when (kind) {
    PendingKind.REQUEST -> SheetTarget.Approval(id)
    PendingKind.PAIRING -> SheetTarget.Pairing(id)
    PendingKind.BLOB -> SheetTarget.Upload(id)
}
