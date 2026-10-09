package dev.reins.android.ui.signin

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.SsoPurpose
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.mcp.webPage
import dev.reins.core.AccountKeys
import dev.reins.core.CoreException
import dev.reins.core.SessionInfo
import dev.reins.core.SsoOutcome
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers

data class SignInUi(
    val busy: Boolean = false,
    val needsTotp: Boolean = false,
    val error: String? = null,
)

/**
 * Signing in ("Continue" through the server's SSO, or email and master password) and creating an account. They end
 * the same way when this phone can open the account's keys: it becomes the approval device. "Continue" to an account
 * whose keys are on another phone ends on the Unlock screen instead.
 */
class SignInViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(SignInUi())
    val ui: StateFlow<SignInUi> = _ui.asStateFlow()

    init {
        // The browser came back from the sign-in page (also after Android restarted the app meanwhile).
        viewModelScope.launch { container.ssoSignIn.callback.filterNotNull().collect { finishSso() } }
    }

    /** The form on screen changed (welcome, create, sign in): its predecessor's error does not carry over. */
    fun clearError() {
        if (!_ui.value.busy) _ui.value = _ui.value.copy(error = null)
    }

    /**
     * "Continue": starts the server's sign-in and opens its page with [open] (a Custom Tab; false when nothing can show
     * it). The browser comes back through `SsoRedirectActivity`; closing the page without signing in changes nothing.
     */
    fun continueWithSso(server: String, open: (String) -> Boolean) {
        val url = AccountRules.serverUrl(server)
        if (url == null) {
            fail("Enter the server's address, like https://reins.example.com.")
            return
        }
        if (_ui.value.busy) return
        _ui.value = _ui.value.copy(busy = true, error = null)
        viewModelScope.launch {
            try {
                val start = container.core.ssoBegin(url)
                if (!webPage(start.url)) {
                    fail("The server's sign-in page is not a web address.")
                    return@launch
                }
                container.ssoSignIn.begin(url, start)
                if (!open(start.url)) {
                    container.ssoSignIn.clear()
                    fail("No browser on this phone can show the sign-in page.")
                    return@launch
                }
                _ui.value = SignInUi()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e.userMessage())
            }
        }
    }

    /**
     * Hands the callback to the core for the waiting sign-in; nothing happens when none waits. A vault reset's callback
     * is the Unlock screen's ([UnlockViewModel]).
     */
    private fun finishSso() {
        val (pending, callback) = container.ssoSignIn.take(SsoPurpose.SignIn) ?: return
        _ui.value = _ui.value.copy(busy = true, error = null)
        container.appScope.launch(Dispatchers.Main) {
            try {
                container.finishSsoSignIn(container.core.ssoFinish(pending.server, callback, pending.state, pending.verifier))
                _ui.value = SignInUi()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e.userMessage())
            }
        }
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
        container.appScope.launch(Dispatchers.Main) {
            try {
                val info = call() ?: return@launch
                container.feedback.play(Event.Connected)
                container.finishSignIn(info)
                _ui.value = SignInUi()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                fail(e.userMessage())
            }
        }
    }
}

/**
 * Where a sign-in through the server's SSO ends: "Continue" (`ssoFinish`), or the sign-in that confirmed a vault reset
 * (`resetAccount`, always [AccountKeys.CREATED]). Keys made or opened here: this phone finishes the sign-in like any
 * other (a new account's recovery code is recorded first). Keys on another phone: the Unlock screen.
 */
internal suspend fun AppContainer.finishSsoSignIn(outcome: SsoOutcome) {
    when (outcome.keys) {
        AccountKeys.CREATED, AccountKeys.UNLOCKED -> {
            feedback.play(Event.Connected)
            finishSignIn(outcome.session)
        }
        // Not the approval device yet: the phone that has the keys has to approve this one first.
        AccountKeys.LOCKED -> {
            setKeysLocked(true)
            refreshSession()
        }
    }
}
