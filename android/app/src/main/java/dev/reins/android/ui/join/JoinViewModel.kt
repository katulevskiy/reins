package dev.reins.android.ui.join

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.JoinView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class JoinUi(
    val loading: Boolean = true,
    val view: JoinView? = null,
    val busy: Boolean = false,
    val error: String? = null,
    /** Answered; [notice] says what an approval did. */
    val finished: Boolean = false,
    val notice: String? = null,
)

/**
 * On the approval device: another phone signed in to the account and asks for its keys. Approving (after biometrics)
 * seals the account's secret to that phone, so it can open the vault and take over approvals once it is set up.
 */
class JoinViewModel(private val container: AppContainer, private val joinId: String) : ViewModel() {
    private val _ui = MutableStateFlow(JoinUi())
    val ui: StateFlow<JoinUi> = _ui.asStateFlow()

    init {
        viewModelScope.launch {
            try {
                _ui.value = JoinUi(loading = false, view = container.core.joinView(joinId))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.value = JoinUi(loading = false, error = e.userMessage())
            }
        }
    }

    fun approve(authenticator: Authenticator) {
        val current = _ui.value
        val view = current.view ?: return
        if (current.busy) return
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val device = untrusted(view.deviceName)
                when (authenticator.authenticate("Add $device", "It can open your account's vault")) {
                    AuthResult.Success -> {
                        container.feedback.play(Event.Connected)
                        container.core.answerJoin(joinId, true)
                        container.refreshPending()
                        _ui.update {
                            it.copy(busy = false, finished = true, notice = "$device can open your account now. It approves from now on once it's set up.")
                        }
                    }
                    AuthResult.Cancelled -> _ui.update { it.copy(busy = false) }
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.update { it.copy(busy = false, error = "Set a screen lock or fingerprint on this phone to approve.") }
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

    fun deny() {
        if (_ui.value.busy) return
        _ui.update { it.copy(busy = true, error = null) }
        container.feedback.play(Event.Denied)
        viewModelScope.launch {
            try {
                container.core.answerJoin(joinId, false)
                container.refreshPending()
                _ui.update { it.copy(busy = false, finished = true) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }
}
