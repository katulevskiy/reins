package dev.reins.android.ui.vault

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import dev.reins.core.VaultItemDetail
import dev.reins.core.VaultItemInput
import dev.reins.core.VaultItemSummary
import dev.reins.core.VaultSshKey
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class VaultUi(val busy: Boolean = false, val error: String? = null)

/**
 * The vault on this phone: the list and its search, one item (its secrets only after the screen lock), and adding,
 * changing and deleting items. Every change is encrypted on the phone by the core before it reaches the server.
 */
class VaultViewModel(private val container: AppContainer) : ViewModel() {
    /** Null until first read. */
    private val _items = MutableStateFlow<List<VaultItemSummary>?>(null)
    val items: StateFlow<List<VaultItemSummary>?> = _items.asStateFlow()

    private val _query = MutableStateFlow("")
    val query: StateFlow<String> = _query.asStateFlow()

    /** The item open on the item screen; null while it loads. */
    private val _item = MutableStateFlow<VaultItemDetail?>(null)
    val item: StateFlow<VaultItemDetail?> = _item.asStateFlow()

    /** Secret values shown on the item screen, by field key; forgotten when another item opens or the screen closes. */
    private val _revealed = MutableStateFlow<Map<String, String>>(emptyMap())
    val revealed: StateFlow<Map<String, String>> = _revealed.asStateFlow()

    /** An SSH key just made on this phone, to show its public half once. */
    private val _madeKey = MutableStateFlow<VaultSshKey?>(null)
    val madeKey: StateFlow<VaultSshKey?> = _madeKey.asStateFlow()

    private val _ui = MutableStateFlow(VaultUi())
    val ui: StateFlow<VaultUi> = _ui.asStateFlow()

    private var search: Job? = null

    /** Reads the list for the current search (a network call); a failure says why and keeps the last list. */
    fun load() {
        search?.cancel()
        search = viewModelScope.launch { read(_query.value) }
    }

    /** Searches as the user types, once they pause. */
    fun search(text: String) {
        _query.value = text
        search?.cancel()
        search = viewModelScope.launch {
            delay(SEARCH_PAUSE_MS)
            read(text)
        }
    }

    private suspend fun read(query: String) {
        try {
            _items.value = container.core.vaultItems(query.trim())
            _ui.update { it.copy(error = null) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _ui.update { it.copy(error = e.userMessage()) }
        }
    }

    fun open(id: String) {
        if (_item.value?.id != id) {
            _item.value = null
            _revealed.value = emptyMap()
        }
        viewModelScope.launch {
            try {
                _item.value = container.core.vaultItem(id)
                _ui.update { it.copy(error = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.update { it.copy(error = e.userMessage()) }
            }
        }
    }

    /** Leaving the item: its revealed secrets are forgotten. */
    fun close() {
        _revealed.value = emptyMap()
        _madeKey.value = null
    }

    /**
     * The value of a secret field, after the screen lock: shown ([copy] false) or handed to [onCopy]. A value already
     * shown is copied without asking again.
     */
    fun reveal(authenticator: Authenticator, key: String, label: String, copy: Boolean, onCopy: (String) -> Unit = {}) {
        val item = _item.value ?: return
        _revealed.value[key]?.let { shown ->
            if (copy) onCopy(shown)
            return
        }
        viewModelScope.launch {
            try {
                val verb = if (copy) "Copy" else "Show"
                when (authenticator.authenticate("$verb the ${label.lowercase()}", item.name)) {
                    AuthResult.Success -> {
                        val value = container.core.vaultReveal(item.id, key)
                        if (copy) onCopy(value) else _revealed.update { it + (key to value) }
                    }
                    AuthResult.Cancelled -> Unit
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.value = VaultUi(error = "Set a screen lock or fingerprint on this phone to see the vault's secrets.")
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = VaultUi(error = e.userMessage())
            }
        }
    }

    fun hide(key: String) {
        _revealed.update { it - key }
    }

    /** Creates the item ([id] null) or changes it; [onDone] gets its id. */
    fun save(id: String?, input: VaultItemInput, onDone: (String) -> Unit) = run(onDone) {
        if (id == null) {
            container.core.vaultCreate(input)
        } else {
            container.core.vaultUpdate(id, input)
            id
        }
    }

    /** Makes an Ed25519 key on the phone and keeps it in the vault; [onDone] gets the new item's id. */
    fun generateSshKey(name: String, onDone: (String) -> Unit) = run(onDone) {
        val key = container.core.vaultGenerateSshKey(name)
        _madeKey.value = key
        key.id
    }

    fun delete(id: String, onDone: () -> Unit) = run({ onDone() }) {
        container.core.vaultDelete(id)
        _item.value = null
        id
    }

    fun clearError() {
        _ui.update { it.copy(error = null) }
    }

    private fun run(onDone: (String) -> Unit, block: suspend () -> String) {
        if (_ui.value.busy) return
        _ui.value = VaultUi(busy = true)
        viewModelScope.launch {
            try {
                val id = block()
                container.feedback.play(Event.GrantCreated)
                _ui.value = VaultUi()
                read(_query.value)
                onDone(id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = VaultUi(error = e.userMessage())
            }
        }
    }

    private companion object {
        const val SEARCH_PAUSE_MS = 250L
    }
}
