package dev.rewarden.android.ui.signin

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.rewarden.android.AppContainer
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.CoreException
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

class SignInViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(SignInUi())
    val ui: StateFlow<SignInUi> = _ui.asStateFlow()

    /** The password lives only in the composable and this call; it is never kept in view-model state. */
    fun signIn(server: String, email: String, password: String, totp: String) {
        if (_ui.value.busy) return
        _ui.value = _ui.value.copy(busy = true, error = null)
        viewModelScope.launch {
            try {
                container.core.login(server.trim(), email.trim(), password, totp.trim().ifEmpty { null })
                container.state.setRegistrationError(null)
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
            } catch (e: CoreException.TwoFactorRequired) {
                _ui.value = SignInUi(needsTotp = true, error = e.userMessage())
            } catch (e: Exception) {
                _ui.value = _ui.value.copy(busy = false, error = e.userMessage())
            }
        }
    }
}
