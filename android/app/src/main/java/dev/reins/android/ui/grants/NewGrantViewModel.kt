package dev.reins.android.ui.grants

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class NewGrantUi(
    val draft: NewGrantDraft = NewGrantDraft(),
    val busy: Boolean = false,
    val error: String? = null,
    val finished: Boolean = false,
)

class NewGrantViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(NewGrantUi())
    val ui: StateFlow<NewGrantUi> = _ui.asStateFlow()

    init {
        viewModelScope.launch { container.refreshConnections() }
    }

    fun edit(change: (NewGrantDraft) -> NewGrantDraft) {
        _ui.update { it.copy(draft = change(it.draft), error = null) }
    }

    /** Creating a permission needs the same authentication as approving one. */
    fun create(authenticator: Authenticator) {
        val current = _ui.value
        if (current.busy) return
        val built = when (val r = buildNewGrant(current.draft)) {
            is NewGrantResult.Invalid -> {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(error = r.message) }
                return
            }
            is NewGrantResult.Ok -> r
        }
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                when (authenticator.authenticate("Create grant", "")) {
                    AuthResult.Success -> {
                        container.feedback.play(Event.GrantCreated)
                        container.core.createGrant(built.connectionId, built.account, built.kind, built.standing)
                        container.refreshPending()
                        _ui.update { it.copy(busy = false, finished = true) }
                    }
                    AuthResult.Cancelled -> _ui.update { it.copy(busy = false) }
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.update { it.copy(busy = false, error = "Set a screen lock or fingerprint on this phone first.") }
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }
}
