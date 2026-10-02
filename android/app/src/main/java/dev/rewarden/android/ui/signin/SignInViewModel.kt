package dev.rewarden.android.ui.signin

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.rewarden.android.AppContainer
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.play
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.CoreException
import dev.rewarden.core.SessionInfo
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class SignInUi(
    val busy: Boolean = false,
    val needsTotp: Boolean = false,
    val error: String? = null,
)

/** Signing in and creating an account. Both end the same way: this phone becomes the approval device. */
class SignInViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(SignInUi())
    val ui: StateFlow<SignInUi> = _ui.asStateFlow()

    /** The form on screen changed (welcome, create, sign in): its predecessor's error does not carry over. */
    fun clearError() {
        if (!_ui.value.busy) _ui.value = _ui.value.copy(error = null)
    }

    /** The password lives only in the composable and this call; it is never kept in view-model state. */
    fun signIn(server: String, email: String, password: String, totp: String) {
        val url = AccountRules.serverUrl(server)
        if (url == null) {
            fail("Enter the server's address, like https://reins.example.com.")
            return
        }
        submit {
            try {
                container.core.login(url, email.trim(), password, totp.trim().ifEmpty { null })
            } catch (e: CoreException.TwoFactorRequired) {
                container.feedback.play(Event.Alert)
                _ui.value = SignInUi(needsTotp = true, error = e.userMessage())
                null
            }
        }
    }

    /** As [signIn], the password is only passed through. The form has checked [AccountRules.createProblem]. */
    fun createAccount(server: String, email: String, password: String, confirm: String, termsAccepted: Boolean) {
        AccountRules.createProblem(email, password, confirm, termsAccepted)?.let {
            fail(it)
            return
        }
        val url = AccountRules.serverUrl(server)
        if (url == null) {
            fail("Enter the server's address, like https://reins.example.com.")
            return
        }
        submit { container.core.createAccount(url, email.trim(), password) }
    }

    private fun fail(message: String) {
        container.feedback.play(Event.Error)
        _ui.value = _ui.value.copy(busy = false, error = message)
    }

    /**
     * Runs [call] (null: it handled its own outcome), then makes this phone the approval device and starts the setup
     * that follows a fresh sign-in.
     */
    private fun submit(call: suspend () -> SessionInfo?) {
        if (_ui.value.busy) return
        _ui.value = _ui.value.copy(busy = true, error = null)
        viewModelScope.launch {
            try {
                val info = call() ?: return@launch
                container.beginOnboarding(info)
                container.state.setRegistrationError(null)
                container.feedback.play(Event.Connected)
                container.refreshSession()
                try {
                    container.registerDevice(force = true)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    container.state.setRegistrationError(e.userMessage())
                }
                _ui.value = SignInUi()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e.userMessage())
            }
        }
    }
}
