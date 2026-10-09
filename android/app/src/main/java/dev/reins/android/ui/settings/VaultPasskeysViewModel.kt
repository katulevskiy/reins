package dev.reins.android.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.PasskeyPrompt
import dev.reins.android.platform.addVaultPasskey
import dev.reins.android.ui.common.userMessage
import dev.reins.core.VaultPasskeyView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class VaultPasskeysUi(val busy: Boolean = false, val error: String? = null)

/**
 * The passkeys that open the account's vault on a new phone: the list in Settings, removing one, and "Add a passkey"
 * (there, and offered before the recovery code after signing in). [deviceName] names a new one ("Pixel 9").
 */
class VaultPasskeysViewModel(private val container: AppContainer, private val deviceName: String) : ViewModel() {
    /** Null until first read. */
    private val _passkeys = MutableStateFlow<List<VaultPasskeyView>?>(null)
    val passkeys: StateFlow<List<VaultPasskeyView>?> = _passkeys.asStateFlow()

    private val _ui = MutableStateFlow(VaultPasskeysUi())
    val ui: StateFlow<VaultPasskeysUi> = _ui.asStateFlow()

    /** Reads the list (a network call); a failure says why and keeps the last one. */
    fun load() {
        viewModelScope.launch {
            try {
                _passkeys.value = container.core.vaultPasskeys()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.update { it.copy(error = e.userMessage()) }
            }
        }
    }

    /**
     * "Add a passkey": the platform's passkey UI makes one ([prompt]), the core keeps the vault's copy for it. Closing
     * the prompt changes nothing; a password manager without PRF says so and adds nothing. Once added, the offer
     * before the recovery code is done too.
     */
    fun add(prompt: PasskeyPrompt) {
        if (_ui.value.busy) return
        _ui.value = VaultPasskeysUi(busy = true)
        viewModelScope.launch {
            try {
                val list = addVaultPasskey(container.core, prompt, deviceName)
                if (list != null) {
                    _passkeys.value = list
                    container.feedback.play(Event.GrantCreated)
                    container.state.setVaultPasskeyOffer(false)
                }
                _ui.value = VaultPasskeysUi()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = VaultPasskeysUi(error = e.userMessage())
            }
        }
    }

    /** The passkey stops opening the vault; it stays in the password manager until deleted there. */
    fun remove(credentialId: ByteArray) {
        if (_ui.value.busy) return
        _ui.value = VaultPasskeysUi(busy = true)
        viewModelScope.launch {
            try {
                container.feedback.play(Event.Revoked)
                _passkeys.value = container.core.removeVaultPasskey(credentialId)
                _ui.value = VaultPasskeysUi()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = VaultPasskeysUi(error = e.userMessage())
            }
        }
    }

    /** "Use the recovery code only": the code shows next, and the offer does not come back for this sign-in. */
    fun decline() {
        if (_ui.value.busy) return
        container.state.declineVaultPasskey()
    }
}
