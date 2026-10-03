package dev.reins.android.ui.upload

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import dev.reins.core.BlobView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class UploadUi(
    val loading: Boolean = true,
    val view: BlobView? = null,
    val busy: Boolean = false,
    val error: String? = null,
    val finished: Boolean = false,
)

/** A file an AI uploaded (`reins_upload`): approved, its download link works; denied, the server deletes it. */
class UploadViewModel(private val container: AppContainer, private val id: String) : ViewModel() {
    private val _ui = MutableStateFlow(UploadUi())
    val ui: StateFlow<UploadUi> = _ui.asStateFlow()

    init {
        viewModelScope.launch {
            _ui.value = try {
                UploadUi(loading = false, view = container.core.blobView(id))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                UploadUi(loading = false, error = e.userMessage())
            }
        }
    }

    /** Approving needs the phone's own screen lock or biometrics first, as every approval does. */
    fun approve(authenticator: Authenticator) {
        val view = _ui.value.view ?: return
        if (_ui.value.busy) return
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                when (authenticator.authenticate("Approve the file", view.connectionLabel)) {
                    AuthResult.Success -> {
                        container.feedback.play(Event.UploadApproved)
                        answer(true)
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
                answer(false)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    private suspend fun answer(approve: Boolean) {
        container.core.answerBlob(id, approve)
        container.refreshPending()
        _ui.update { it.copy(busy = false, finished = true) }
    }
}
