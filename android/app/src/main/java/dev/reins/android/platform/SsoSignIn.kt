package dev.reins.android.platform

import android.content.Context
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.SsoStart
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What a sign-in through the server's SSO is for, and so which call its callback goes to. */
enum class SsoPurpose {
    /** "Continue" on the welcome page: `ssoFinish`. */
    SignIn,

    /** "Reset the vault" on the Unlock screen, confirmed by signing in again: `resetAccount`. */
    Reset,
}

/** A sign-in through the server's SSO that waits for its browser page to come back: what `ssoFinish` needs. */
data class PendingSso(
    val server: String,
    val state: String,
    val verifier: String,
    val purpose: SsoPurpose = SsoPurpose.SignIn,
)

/**
 * The SSO sign-in ("Continue", or the one that confirms a vault reset) between opening its page in a Custom Tab and the
 * browser's return to `com.reins2fa.app://sso-callback`. The server, `state`, PKCE verifier and [SsoPurpose] are kept
 * in the app's private storage (never backed up), so a sign-in still finishes when Android stopped the app while the
 * page was open. Each start is used once, by the first callback that arrives for it; the core checks that the callback
 * answers it (`state`).
 */
class SsoSignIn(context: Context, private val now: () -> Long = System::currentTimeMillis) {
    private val prefs = SealedPreferences(context, "sso_sign_in")

    private val _callback = MutableStateFlow<String?>(null)

    /** The callback that came back and waits for the screen of its [SsoPurpose] to finish it. */
    val callback: StateFlow<String?> = _callback.asStateFlow()

    /** The page of [start] (for [server]) is about to open; a callback left from an earlier start is forgotten. */
    fun begin(server: String, start: SsoStart, purpose: SsoPurpose = SsoPurpose.SignIn) {
        _callback.value = null
        prefs.edit()
            .putString(KEY_SERVER, server)
            .putString(KEY_STATE, start.state)
            .putString(KEY_VERIFIER, start.verifier)
            .putString(KEY_PURPOSE, purpose.name)
            .putLong(KEY_STARTED, now())
            .commit()
    }

    /** The sign-in that waits for its callback, unless it was started too long ago. */
    fun pending(): PendingSso? {
        val server = prefs.getString(KEY_SERVER, null) ?: return null
        val state = prefs.getString(KEY_STATE, null) ?: return null
        val verifier = prefs.getString(KEY_VERIFIER, null) ?: return null
        // A start kept before the purpose was recorded was a "Continue".
        val purpose = prefs.getString(KEY_PURPOSE, null)
            ?.let { name -> SsoPurpose.entries.firstOrNull { it.name == name } ?: return null }
            ?: SsoPurpose.SignIn
        if (now() - prefs.getLong(KEY_STARTED, 0) !in 0..MAX_AGE_MILLIS) return null
        return PendingSso(server, state, verifier, purpose)
    }

    /** The browser came back with [url]: kept for the sign-in screen when it is the callback and a sign-in waits for it. */
    fun deliver(url: String): Boolean {
        if (!AccountRules.isSsoCallback(url) || pending() == null) return false
        _callback.value = url
        return true
    }

    /**
     * The waiting sign-in and its callback, taken (and forgotten) to finish it when it was started for [purpose]; null
     * when either is missing. A callback for the other purpose stays for the screen that finishes it.
     */
    fun take(purpose: SsoPurpose): Pair<PendingSso, String>? {
        val url = _callback.value ?: return null
        val pending = pending()
        if (pending != null && pending.purpose != purpose) return null
        _callback.value = null
        clear()
        return pending?.let { it to url }
    }

    fun clear() {
        prefs.edit().clear().commit()
    }

    private companion object {
        const val KEY_SERVER = "server"
        const val KEY_STATE = "state"
        const val KEY_VERIFIER = "verifier"
        const val KEY_PURPOSE = "purpose"
        const val KEY_STARTED = "started_at"

        /** A sign-in left open longer than this is not finished by a late callback. */
        const val MAX_AGE_MILLIS = 30L * 60 * 1000
    }
}
