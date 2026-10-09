package dev.reins.android.ui.pairing

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import dev.reins.core.PairingView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class PairingUi(
    val loading: Boolean = true,
    val view: PairingView? = null,
    /** The two-digit code the user tapped; the AI is connected only if it is the one their computer or browser shows. */
    val chosen: Int? = null,
    val label: String = "",
    val busy: Boolean = false,
    val error: String? = null,
    /** Approving failed because the phone has no screen lock: the sheet offers Android's settings for one. */
    val needsScreenLock: Boolean = false,
    val finished: Boolean = false,
) {
    /** Codes as unsigned values (the UniFFI `ByteArray` is signed). */
    val codes: List<Int> get() = view?.choices?.map { it.toInt() and 0xFF } ?: emptyList()
}

class PairingViewModel(private val container: AppContainer, private val pairingId: String) : ViewModel() {
    private val _ui = MutableStateFlow(PairingUi())
    val ui: StateFlow<PairingUi> = _ui.asStateFlow()

    /** The starting rule: what connecting gives the new AI (the sheet says so before the user connects). */
    val startingPolicy: StateFlow<dev.reins.core.StartingPolicy?> = container.state.startingPolicy

    init {
        viewModelScope.launch {
            try {
                val view = container.core.pairingView(pairingId)
                _ui.value = PairingUi(loading = false, view = view, label = view.clientName.take(MAX_LABEL))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.value = PairingUi(loading = false, error = e.userMessage())
            }
        }
    }

    fun choose(code: Int) = _ui.update { it.copy(chosen = code, error = null) }

    fun setLabel(label: String) = _ui.update { it.copy(label = label.take(MAX_LABEL)) }

    fun approve(authenticator: Authenticator) {
        val current = _ui.value
        val code = current.chosen
        if (current.busy) return
        if (code == null) {
            container.feedback.play(Event.Error)
            _ui.update { it.copy(error = "Tap the code your computer or browser shows.") }
            return
        }
        _ui.update { it.copy(busy = true, error = null, needsScreenLock = false) }
        viewModelScope.launch {
            try {
                // A desktop app brings its key; an AI client (Claude.ai, ChatGPT) does not.
                val title = if (current.view?.keyFingerprint != null) "Connect this computer" else "Connect this AI"
                when (authenticator.authenticate(title, current.view?.clientName.orEmpty())) {
                    AuthResult.Success -> {
                        container.feedback.play(Event.Connected)
                        container.core.answerPairing(pairingId, true, code.toUByte(), current.label.trim().ifEmpty { null })
                        // A computer: the page that connects computers says it worked.
                        if (current.view?.keyFingerprint != null) {
                            container.state.setJustPaired(current.label.trim().ifEmpty { current.view.clientName })
                        }
                        container.refreshAfterAnswer(pairingId)
                        _ui.update { it.copy(busy = false, finished = true) }
                    }
                    AuthResult.Cancelled -> _ui.update { it.copy(busy = false) }
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.update { it.copy(busy = false, needsScreenLock = true) }
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
                container.core.answerPairing(pairingId, false, null, null)
                container.refreshAfterAnswer(pairingId)
                _ui.update { it.copy(busy = false, finished = true) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    private companion object {
        const val MAX_LABEL = 64
    }
}
