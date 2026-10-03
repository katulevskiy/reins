package dev.reins.android.ui.services

import android.app.PendingIntent
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.userMessage
import dev.reins.core.GmailStatus
import dev.reins.core.LoginProgress
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Where a phone-number sign-in stands. */
sealed interface LoginStep {
    /** Not started: the phone number is asked for. */
    data object Phone : LoginStep

    /** The code Telegram sent is asked for. */
    data class Code(val phone: String) : LoginStep

    /** The account has two-step verification. */
    data class Password(val hint: String?) : LoginStep
}

/** The accounts of one integration (other than Gmail): how each one is doing, adding one, removing one. */
class ServiceViewModel(private val container: AppContainer, val service: String) : ViewModel() {
    private val _statuses = MutableStateFlow<Map<String, GmailStatus>>(emptyMap())
    val statuses: StateFlow<Map<String, GmailStatus>> = _statuses.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private val _busy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    private val _login = MutableStateFlow<LoginStep>(LoginStep.Phone)
    val login: StateFlow<LoginStep> = _login.asStateFlow()

    /** The account whose Google consent screen is on top, until the user comes back from it. */
    private var awaitingConsent: String? = null

    /** Shows a failure that happened outside the view model (Android refused a permission). */
    fun fail(message: String) {
        container.feedback.play(Event.Error)
        _error.value = message
    }

    fun clearError() {
        _error.value = null
    }

    /** Runs [block] as one operation: no second one starts meanwhile, and failures are shown. */
    private fun operation(block: suspend () -> Unit) {
        if (_busy.value) return
        _busy.value = true
        _error.value = null
        viewModelScope.launch {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _error.value = e.userMessage()
            } finally {
                _busy.value = false
            }
        }
    }

    /** An account was added: it sounds, and the lists catch up. */
    private suspend fun connected() {
        container.feedback.play(Event.Connected)
        refreshed()
    }

    private suspend fun refreshed() {
        container.refreshPending()
        refreshNow()
    }

    fun refresh() {
        viewModelScope.launch { refreshNow() }
    }

    private suspend fun refreshNow() {
        val accounts = try {
            container.core.services().also { container.state.setServices(it) }.firstOrNull { it.service == service }?.accounts.orEmpty()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _error.value = e.userMessage()
            return
        }
        for (account in accounts) {
            val status = try {
                container.core.serviceAccountStatus(service, account.account)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                GmailStatus.Unavailable(e.userMessage())
            }
            _statuses.update { it + (account.account to status) }
        }
    }

    // ---- Google ----------------------------------------------------------------------------------------------

    /** The user picked [account] in Google's account picker: ask for consent if needed, then connect it. */
    fun accountChosen(account: String, launchConsent: (PendingIntent) -> Unit) = operation {
        val intent = container.google.consentIntent(account, service)
        if (intent != null) {
            awaitingConsent = account
            launchConsent(intent)
        } else {
            connect(account)
        }
    }

    /** Back from Google's consent screen. */
    fun consentFinished() {
        val account = awaitingConsent ?: return
        awaitingConsent = null
        operation { connect(account) }
    }

    /** Registers [account] in the core (which checks with Google that the permission works). */
    suspend fun connect(account: String) {
        container.core.addServiceAccount(service, account)
        connected()
    }

    // ---- an access token or the vault's master password -------------------------------------------------------

    fun addSecret(secret: String, onDone: () -> Unit = {}) = operation {
        container.core.addTokenAccount(service, secret.trim())
        connected()
        onDone()
    }

    // ---- this phone's own services ---------------------------------------------------------------------------

    /** Android granted the permissions: connect the phone's calendar, contacts or messages. */
    fun addDevice() = operation {
        container.core.addServiceAccount(service, "")
        connected()
    }

    // ---- a phone number and a code (Telegram) ----------------------------------------------------------------

    fun loginBegin(phone: String) = operation {
        container.core.loginBegin(service, phone.trim())
        _login.value = LoginStep.Code(phone.trim())
    }

    fun loginCode(code: String) = operation {
        when (val progress = container.core.loginCode(service, code.trim())) {
            is LoginProgress.NeedsPassword -> _login.value = LoginStep.Password(progress.hint)
            is LoginProgress.Done -> {
                _login.value = LoginStep.Phone
                connected()
            }
        }
    }

    fun loginPassword(password: String) = operation {
        container.core.loginPassword(service, password)
        _login.value = LoginStep.Phone
        connected()
    }

    /** Starts the sign-in over (a wrong number, a code that never came). */
    fun loginRestart() {
        _login.value = LoginStep.Phone
        _error.value = null
    }

    // ---- removing --------------------------------------------------------------------------------------------

    /** Disconnects [account]: Reins forgets it (signing it out where that applies) and Google is told to revoke. */
    fun remove(account: String, google: Boolean) = operation {
        container.feedback.play(Event.Revoked)
        container.core.removeServiceAccount(service, account)
        _statuses.update { it - account }
        refreshed()
        // Best effort: without a network the access is still gone on this phone.
        if (google) runCatching { container.google.revoke(account, service) }
    }
}
