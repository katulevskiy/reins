package dev.reins.android.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.FeedbackSettings
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.update.UpdateController
import dev.reins.android.state.SessionState
import dev.reins.android.ui.common.userMessage
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class SettingsUi(val busy: Boolean = false, val message: String? = null, val error: String? = null)

data class DeleteAccountUi(val busy: Boolean = false, val error: String? = null)

class SettingsViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(SettingsUi())
    val ui: StateFlow<SettingsUi> = _ui.asStateFlow()

    /** The in-app updater, shared with the prompt and the background check; null when Google Play updates the app. */
    val updates: UpdateController? get() = container.updates

    /** The Sounds & haptics switches, for the row that leads to them. */
    val sounds: StateFlow<FeedbackSettings> get() = container.feedbackStore.settings

    /** A confirmation shown under the approval-device card ("Server address copied."). */
    fun notice(message: String) {
        _ui.value = SettingsUi(message = message)
    }

    /** Whether the signed-in server can wake this app (false: requests arrive only while it is open); null unknown. */
    private val _serverPush = MutableStateFlow<Boolean?>(null)
    val serverPush: StateFlow<Boolean?> = _serverPush.asStateFlow()

    init {
        viewModelScope.launch { container.refreshConnections() }
        viewModelScope.launch {
            val server = (container.state.session.value as? SessionState.SignedIn)?.info?.serverUrl ?: return@launch
            _serverPush.value = try {
                container.core.serverInfo(server).pushAndroid
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                null
            }
        }
    }

    /** Whether this phone keeps the account's recovery code (accounts made with a master password have none). */
    private val _hasRecoveryCode = MutableStateFlow(false)
    val hasRecoveryCode: StateFlow<Boolean> = _hasRecoveryCode.asStateFlow()

    /** How many passkeys open the account's vault (shown where the recovery code is); null while unknown. */
    private val _vaultPasskeys = MutableStateFlow<Int?>(null)
    val vaultPasskeys: StateFlow<Int?> = _vaultPasskeys.asStateFlow()

    /** The recovery code while its sheet is open; only ever read after biometrics. */
    private val _recoveryCode = MutableStateFlow<String?>(null)
    val recoveryCode: StateFlow<String?> = _recoveryCode.asStateFlow()

    /**
     * Settings opened: offer the recovery code only when the core can give it (the code itself is not kept), and with
     * it the vault's passkeys, which only a phone that keeps the account secret can add.
     */
    fun checkRecoveryCode() {
        viewModelScope.launch {
            _hasRecoveryCode.value = try {
                container.core.accountRecoveryCode().isNotEmpty()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                false
            }
            if (!_hasRecoveryCode.value) return@launch
            _vaultPasskeys.value = try {
                container.core.vaultPasskeys().size
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                null
            }
        }
    }

    /** "Recovery code": biometrics first, then the code in a sheet. */
    fun showRecoveryCode(authenticator: Authenticator) {
        if (_ui.value.busy) return
        viewModelScope.launch {
            try {
                when (authenticator.authenticate("Show your recovery code", "It opens your account's vault")) {
                    AuthResult.Success -> _recoveryCode.value = container.core.accountRecoveryCode()
                    AuthResult.Cancelled -> Unit
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.value = SettingsUi(error = "Set a screen lock or fingerprint on this phone to see the recovery code.")
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = SettingsUi(error = e.userMessage())
            }
        }
    }

    fun hideRecoveryCode() {
        _recoveryCode.value = null
    }

    fun registerThisPhone() {
        run("This phone is now your approval device.") {
            container.registerDevice(force = true)
            container.state.setRegistrationError(null)
        }
    }

    fun signOut() {
        run(null) {
            _hasRecoveryCode.value = false
            _recoveryCode.value = null
            container.signOut()
        }
    }

    /** The "Delete account" sheet while it is open (null when closed); its error stays in the sheet. */
    private val _deletion = MutableStateFlow<DeleteAccountUi?>(null)
    val deletion: StateFlow<DeleteAccountUi?> = _deletion.asStateFlow()

    fun openDeletion() {
        if (!_ui.value.busy) _deletion.value = DeleteAccountUi()
    }

    fun closeDeletion() {
        if (_deletion.value?.busy != true) _deletion.value = null
    }

    /** Deletes the account once [typed] is its email; a refusal (a typo, another phone approves, offline) is shown. */
    fun deleteAccount(typed: String) {
        if (_deletion.value?.busy != false) return
        _deletion.value = DeleteAccountUi(busy = true)
        viewModelScope.launch {
            _deletion.value = try {
                container.deleteAccount(typed)
                _hasRecoveryCode.value = false
                _recoveryCode.value = null
                null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                DeleteAccountUi(error = e.userMessage())
            }
        }
    }

    /** Sets (or with null, clears) the icon of an AI connection. */
    fun pickIcon(connectionId: String, icon: String?) {
        viewModelScope.launch {
            try {
                container.core.setConnectionIcon(connectionId, icon)
                container.refreshConnections()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = SettingsUi(error = e.userMessage())
            }
        }
    }

    fun disconnect(connectionId: String, onDone: () -> Unit) {
        if (!_ui.value.busy) container.feedback.play(Event.Revoked)
        run(null) {
            container.core.revokeConnection(connectionId)
            container.refreshConnections()
            container.refreshAfterAnswer()
            onDone()
        }
    }

    private fun run(success: String?, block: suspend () -> Unit) {
        if (_ui.value.busy) return
        _ui.value = SettingsUi(busy = true)
        viewModelScope.launch {
            _ui.value = try {
                block()
                SettingsUi(message = success)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                SettingsUi(error = e.userMessage())
            }
        }
    }
}
