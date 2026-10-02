package dev.rewarden.android.ui.mcp

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.rewarden.android.AppContainer
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.play
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.McpAddStep
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Opens a web page; false when nothing on the phone can. */
typealias PageOpener = (String) -> Boolean

/** The MCP servers screens: adding a server, signing in to it, its tools, removing it. */
class McpViewModel(private val container: AppContainer) : ViewModel() {
    private val _busy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    /** The server whose sign-in page was opened, until the page comes back (or another operation starts). */
    private val _signingIn = MutableStateFlow<String?>(null)
    val signingIn: StateFlow<String?> = _signingIn.asStateFlow()

    fun clearError() {
        _error.value = null
    }

    /** Runs [block] as one operation: no second one starts meanwhile, and failures are shown in plain words. */
    private fun operation(block: suspend () -> Unit) {
        if (_busy.value) return
        _busy.value = true
        _error.value = null
        viewModelScope.launch {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _error.value = e.userMessage()
            } finally {
                _busy.value = false
            }
        }
    }

    private suspend fun reload() {
        container.state.setMcpServers(container.core.mcpServers())
    }

    fun refreshList() {
        viewModelScope.launch {
            try {
                reload()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _error.value = e.userMessage()
            }
        }
    }

    /**
     * Adds the server at [url]: with [token] as its access token when one is given, else by asking it (it may want a
     * sign-in, whose page [open] shows). [onAdded] gets the new server's id once it is usable.
     */
    fun add(url: String, name: String, token: String, open: PageOpener, onAdded: (String) -> Unit) = operation {
        _signingIn.value = null
        val address = url.trim()
        val label = name.trim().ifEmpty { null }
        if (token.isNotBlank()) {
            val server = container.core.mcpAddWithToken(address, token.trim(), label)
            container.feedback.play(Event.Connected)
            reload()
            onAdded(server.id)
        } else {
            step(container.core.mcpAdd(address, label), open, onAdded)
        }
    }

    /** Connects to the server again; a sign-in that ended opens its page again. */
    fun refresh(id: String, open: PageOpener) = operation {
        container.feedback.play(Event.Refresh)
        _signingIn.value = null
        step(container.core.mcpRefresh(id), open) {}
    }

    private suspend fun step(step: McpAddStep, open: PageOpener, onAdded: (String) -> Unit) {
        when (step) {
            is McpAddStep.Added -> {
                container.feedback.play(Event.Connected)
                reload()
                onAdded(step.server.id)
            }
            is McpAddStep.NeedsSignIn -> {
                reload()
                if (!webPage(step.authorizeUrl)) {
                    container.feedback.play(Event.Error)
                    _error.value = "This server's sign-in page is not a web page, so it was not opened."
                    return
                }
                container.mcpSignIn.begin(step.serverId)
                if (open(step.authorizeUrl)) {
                    _signingIn.value = step.serverId
                } else {
                    container.feedback.play(Event.Error)
                    _error.value = "No browser on this phone can open the sign-in page."
                }
            }
        }
    }

    fun setHeavy(id: String, tool: String, heavy: Boolean) = operation {
        container.core.mcpSetHeavy(id, tool, heavy)
        reload()
    }

    fun remove(id: String, onDone: () -> Unit) = operation {
        container.feedback.play(Event.Revoked)
        container.core.mcpRemove(id)
        reload()
        container.refreshPending()
        onDone()
    }
}
