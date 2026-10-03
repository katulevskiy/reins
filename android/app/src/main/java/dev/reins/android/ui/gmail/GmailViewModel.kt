package dev.reins.android.ui.gmail

import android.app.PendingIntent
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.userMessage
import dev.reins.core.GmailStatus
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** The Gmail accounts: how each one is doing, adding one, removing one. */
class GmailViewModel(private val container: AppContainer) : ViewModel() {
    /** Per account: null while it is being checked. */
    private val _statuses = MutableStateFlow<Map<String, GmailStatus>>(emptyMap())
    val statuses: StateFlow<Map<String, GmailStatus>> = _statuses.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private val _busy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    /** The account whose Google consent screen is on top, until the user comes back from it. */
    private var awaitingConsent: String? = null

    fun clearError() {
        _error.value = null
    }

    /** Checks every connected account (a network call each). */
    fun refresh() {
        viewModelScope.launch {
            val accounts = try {
                container.core.accounts().also { container.state.setAccounts(it) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _error.value = e.userMessage()
                return@launch
            }
            for (account in accounts) {
                val status = try {
                    container.core.accountStatus(account.account)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    GmailStatus.Unavailable(e.userMessage())
                }
                _statuses.value = _statuses.value + (account.account to status)
            }
        }
    }

    /** The user picked [account] in Google's account picker: ask for consent if needed, then connect it. */
    fun accountChosen(account: String, launchConsent: (PendingIntent) -> Unit) {
        if (_busy.value) return
        _busy.value = true
        _error.value = null
        viewModelScope.launch {
            try {
                val intent = container.google.consentIntent(account, "gmail")
                if (intent != null) {
                    awaitingConsent = account
                    launchConsent(intent)
                } else {
                    connect(account)
                }
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

    /** Back from Google's consent screen. */
    fun consentFinished() {
        val account = awaitingConsent ?: return
        awaitingConsent = null
        viewModelScope.launch { connect(account) }
    }

    /** Asks again for the consent of an account that lost it. */
    fun reconnect(account: String, launchConsent: (PendingIntent) -> Unit) = accountChosen(account, launchConsent)

    /** Registers [account] in the core (which checks with Gmail that the permission works). */
    suspend fun connect(account: String) {
        try {
            container.core.addAccount(account)
            container.feedback.play(Event.Connected)
            container.refreshPending()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            container.feedback.play(Event.Error)
            _error.value = e.userMessage()
        }
        refresh()
    }

    /** Disconnects [account]: Reins forgets it and Google is told to revoke the app's access. */
    fun remove(account: String) {
        if (_busy.value) return
        _busy.value = true
        _error.value = null
        viewModelScope.launch {
            try {
                container.feedback.play(Event.Revoked)
                container.core.removeAccount(account)
                _statuses.value = _statuses.value - account
                container.refreshPending()
                // Best effort: without a network the access is still gone on this phone.
                runCatching { container.google.revoke(account, "gmail") }
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
}
