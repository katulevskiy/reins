package dev.reins.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.Foreground
import dev.reins.android.platform.McpSignInResult
import dev.reins.android.state.McpNotice
import dev.reins.android.state.SessionState
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.nav.DeepLink
import dev.reins.android.ui.nav.Route
import dev.reins.android.ui.nav.SheetTarget
import dev.reins.android.ui.nav.Tab
import dev.reins.android.ui.pairing.PairingCode
import dev.reins.core.CoreException
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import kotlin.coroutines.cancellation.CancellationException
import dev.reins.android.ui.signin.SetupPage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Connecting a computer by the code it shows: looking it up, why that failed, and whether the code field is open. */
data class ConnectUi(
    val busy: Boolean = false,
    val error: String? = null,
    /** The field to type the code in (always offered; opened by itself when the scanner cannot run). */
    val manual: Boolean = false,
)

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

    private val _connect = MutableStateFlow(ConnectUi())
    val connect: StateFlow<ConnectUi> = _connect.asStateFlow()

    /** A pairing link opened while signed out: it is used as soon as someone signs in. */
    private val _waitingCode = MutableStateFlow<String?>(null)
    val waitingCode: StateFlow<String?> = _waitingCode.asStateFlow()

    val current: Route? get() = stack.lastOrNull()

    init {
        container.state.onAccountChange = ::resetAccountNavigation
        // Not immediate: state changes must not resume collectors (e.g. the deep-link handler, which looks this
        // view model up) while the view model is still being constructed.
        viewModelScope.launch(Dispatchers.Main) { container.refreshSession() }
        viewModelScope.launch(Dispatchers.Main) {
            container.state.pending.collect { maybePresent(it) }
        }
        viewModelScope.launch(Dispatchers.Main) {
            combine(container.state.session, container.state.keysLocked, ::Pair).collect { (session, locked) ->
                if (session is SessionState.SignedOut) {
                    // The next sign-in starts on the main screen, not where the last one signed out.
                    home()
                    _sheet.value = null
                    _connect.value = ConnectUi()
                }
                // A waiting link is used once the app itself shows (not on the Unlock screen).
                if (session !is SessionState.SignedIn || locked) return@collect
                val code = _waitingCode.value ?: return@collect
                _waitingCode.value = null
                connectWithCode(code, fromLink = true)
            }
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

    /** A line on the main screen about something that just happened ("Pixel 9 can open your account now."). */
    fun showNotice(message: String) {
        _notice.value = message
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
        resetAccountNavigation()
    }

    fun resetAccountNavigation() {
        home()
        _sheet.value = null
        _connect.value = ConnectUi()
    }

    // ---- connecting a computer by its code -----------------------------------------------------------------------

    /** What Google's scanner read. Anything but a pairing code (or a link to one) is refused. */
    fun connectScanned(text: String) {
        val code = PairingCode.parse(text)
        if (code == null) {
            failConnect("That QR code is not a Reins pairing code. Scan the one your computer shows.")
            return
        }
        connectWithCode(code, fromLink = false)
    }

    /** What the user typed into the code field. */
    fun connectTyped(text: String) {
        val code = PairingCode.parse(text)
        if (code == null) {
            failConnect("Enter the 8 letters your computer shows, like BCDF-GHJK.")
            return
        }
        connectWithCode(code, fromLink = false)
    }

    /** The scanner could not run: say so and open the field to type the code. */
    fun scannerUnavailable() {
        failConnect("The QR scanner is not available on this phone. Type the code your computer shows instead.", openField = true)
    }

    fun showCodeField() = _connect.update { it.copy(manual = true) }

    /** The connect screens closed: their error does not wait for the next visit. */
    fun resetConnect() {
        if (!_connect.value.busy) _connect.value = ConnectUi()
    }

    /**
     * A `/pair` link (an App Link or `reins://pair`), already reduced to a well-formed code. Signed in, it opens the
     * pairing at once; signed out (or on the Unlock screen), it waits for the sign-in to finish.
     */
    fun openPairingLink(code: String) {
        if (container.state.session.value is SessionState.SignedIn && !container.state.keysLocked.value) {
            connectWithCode(code, fromLink = true)
        } else {
            _waitingCode.value = code
        }
    }

    /**
     * Asks the server (through the core) for the pairing [code] stands for and opens the usual pairing sheet for it:
     * the user still taps the number the computer shows, names it and confirms with biometrics.
     */
    private fun connectWithCode(code: String, fromLink: Boolean) {
        if (_connect.value.busy) return
        _connect.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val view = container.core.pairingByCode(code)
                _connect.value = ConnectUi()
                if (fromLink) home()
                // Open first, so the refresh below cannot pop up some other waiting item ahead of it.
                openSheet(SheetTarget.Pairing(view.id))
                container.refreshPending()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val message = if (e is CoreException.NotFound) CODE_EXPIRED else e.userMessage()
                failConnect(message)
                if (fromLink) {
                    // A link has no connect screen behind it: the main screen says what happened.
                    home()
                    _notice.value = message
                }
            }
        }
    }

    private fun failConnect(message: String, openField: Boolean = false) {
        container.feedback.play(Event.Error)
        _connect.update { it.copy(busy = false, error = message, manual = it.manual || openField) }
    }

    /**
     * Which page of the setup after signing in shows. Kept here rather than in the screen, so opening an integration
     * from the setup and coming back lands on the same page.
     */
    var setupPage by mutableStateOf(SetupPage.Welcome)

    /** The setup after signing in was finished or skipped. */
    fun finishSetup() {
        viewModelScope.launch {
            container.finishOnboarding()
            _connect.value = ConnectUi()
            setupPage = SetupPage.Welcome
            home()
        }
    }

    /** Settings > "Take the tour": the setup once more, from its first page. */
    fun replaySetup() {
        setupPage = SetupPage.Welcome
        home()
        container.state.setSetupPending(true)
    }

    private companion object {
        const val CODE_EXPIRED = "This code has expired or was already used. Show a new one on your computer."
    }
}

fun PendingItem.toTarget(): SheetTarget = when (kind) {
    PendingKind.REQUEST -> SheetTarget.Approval(id)
    PendingKind.PAIRING -> SheetTarget.Pairing(id)
    PendingKind.BLOB -> SheetTarget.Upload(id)
    PendingKind.JOIN -> SheetTarget.Join(id)
}
