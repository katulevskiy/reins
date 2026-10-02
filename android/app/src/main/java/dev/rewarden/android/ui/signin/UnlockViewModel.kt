package dev.rewarden.android.ui.signin

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.rewarden.android.AppContainer
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.play
import dev.rewarden.android.state.SessionState
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.CoreException
import dev.rewarden.core.JoinProgress
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Where the Unlock screen is: the two ways to open the keys, waiting for the other phone, or the recovery code field. */
enum class UnlockStep { Choose, Asking, Recovery }

data class UnlockUi(
    val step: UnlockStep = UnlockStep.Choose,
    /** The code this phone shows while it asks ("482 193"); the other phone shows the same. */
    val code: String? = null,
    val busy: Boolean = false,
    /** What went wrong on the current step. */
    val error: String? = null,
    /** How the last request to the other phone ended (refused, expired), shown with the two choices. */
    val ended: String? = null,
)

/** How often the new phone asks whether the other phone answered. */
const val JOIN_POLL_MILLIS = 2_000L

/**
 * Asks [poll] after each [pause] until the other phone answered (or the request expired). A network hiccup counts as
 * "not answered yet"; every other failure ends the wait.
 */
suspend fun awaitJoin(poll: suspend () -> JoinProgress, pause: suspend () -> Unit): JoinProgress {
    while (true) {
        pause()
        val progress = try {
            poll()
        } catch (e: CoreException.Network) {
            JoinProgress.WAITING
        }
        if (progress != JoinProgress.WAITING) return progress
    }
}

/**
 * Signed in through "Continue", but the account's keys are on another phone: ask that phone for them ("Add another
 * phone"), or open them with the recovery code (or the master password of an account made with one). Once open, this
 * phone finishes the sign-in like any other and becomes the approval device. [deviceName] is what the other phone
 * shows ("Add Pixel 9?").
 *
 * The same two ways let this phone take the approval role over when the server refused it because another phone
 * approves for the account ([dev.rewarden.android.state.AppState.approvalTakeover]): the other phone's yes, or the
 * recovery code, is the proof the next registration brings.
 */
class UnlockViewModel(
    private val container: AppContainer,
    private val deviceName: String,
    private val pollMillis: Long = JOIN_POLL_MILLIS,
) : ViewModel() {
    private val _ui = MutableStateFlow(UnlockUi())
    val ui: StateFlow<UnlockUi> = _ui.asStateFlow()

    private var asking: Job? = null

    /** "Ask my other phone": sends the request, shows its code and waits for the answer. */
    fun askOtherPhone() {
        if (_ui.value.busy || asking?.isActive == true) return
        _ui.update { it.copy(busy = true, error = null, ended = null) }
        asking = viewModelScope.launch {
            try {
                val start = container.core.joinBegin(deviceName)
                _ui.value = UnlockUi(step = UnlockStep.Asking, code = start.code)
                when (awaitJoin({ container.core.joinPoll() }, { delay(pollMillis) })) {
                    JoinProgress.JOINED -> unlocked()
                    JoinProgress.DENIED -> ended("Your other phone said no. If that was a mistake, ask again.")
                    JoinProgress.EXPIRED -> ended("Your other phone did not answer in time. Ask again.")
                    JoinProgress.WAITING -> Unit
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = UnlockUi(error = e.userMessage())
            }
        }
    }

    /** Cancel while waiting: the request is withdrawn, the two choices show again. */
    fun cancelAsk() {
        asking?.cancel()
        asking = null
        _ui.value = UnlockUi()
        viewModelScope.launch { withdraw() }
    }

    fun showRecovery() {
        if (_ui.value.busy) return
        _ui.value = UnlockUi(step = UnlockStep.Recovery)
    }

    /** Back from the recovery code field to the two choices. */
    fun back() {
        if (_ui.value.busy) return
        _ui.value = UnlockUi()
    }

    /** "Unlock": the recovery code (any case, spaces and dashes) or the master password; only passed through. */
    fun unlock(codeOrPassword: String) {
        if (_ui.value.busy || codeOrPassword.isBlank()) return
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                container.core.unlockAccount(codeOrPassword.trim())
                unlocked()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    /** "Not now" while taking over: the app shows again, this phone not approving; Settings offers it again. */
    fun later() {
        if (_ui.value.busy) return
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            container.setApprovalTakeover(false)
            _ui.value = UnlockUi()
        }
    }

    /** Back to the welcome screen; an open request is withdrawn first. */
    fun signOut() {
        if (_ui.value.busy) return
        asking?.cancel()
        asking = null
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            withdraw()
            try {
                container.core.logout()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Signed out on this phone either way.
            }
            container.forgetAccount()
            container.state.setSession(SessionState.SignedOut)
            _ui.value = UnlockUi()
        }
    }

    /**
     * The keys are open on this phone: the sign-in finishes like any other. Taking over (the keys were open already):
     * this phone registers again, now with the proof, and the app carries on where the sign-in or Settings left it.
     */
    private suspend fun unlocked() {
        if (!container.state.keysLocked.value) {
            container.registerDevice(force = true)
            container.state.setRegistrationError(null)
            container.feedback.play(Event.Connected)
            container.refreshSession()
            _ui.value = UnlockUi()
            return
        }
        val info = (container.state.session.value as? SessionState.SignedIn)?.info ?: container.core.session()
        if (info == null) {
            container.setKeysLocked(false)
            container.refreshSession()
            return
        }
        container.feedback.play(Event.Connected)
        container.finishSignIn(info)
        _ui.value = UnlockUi()
    }

    private fun ended(message: String) {
        container.feedback.play(Event.Alert)
        _ui.value = UnlockUi(ended = message)
    }

    private suspend fun withdraw() {
        try {
            container.core.joinCancel()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Nothing was open, or the server forgets it when it expires.
        }
    }
}
