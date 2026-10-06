package dev.reins.android.platform

import android.content.Context
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.SsoStart
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** A sign-in through the server's SSO that waits for its browser page to come back: what `ssoFinish` needs. */
data class PendingSso(val server: String, val state: String, val verifier: String)

/**
 * The SSO sign-in ("Continue") between opening its page in a Custom Tab and the browser's return to
 * `com.reins2fa.app://sso-callback`. The server, `state` and PKCE verifier are kept in the app's private storage (never
 * backed up), so a sign-in still finishes when Android stopped the app while the page was open. Each start is used
 * once, by the first callback that arrives for it; the core checks that the callback answers it (`state`).
 */
class SsoSignIn(context: Context, private val now: () -> Long = System::currentTimeMillis) {
    private val prefs = SealedPreferences(context, "sso_sign_in")

    private val _callback = MutableStateFlow<String?>(null)

    /** The callback that came back and waits for the sign-in screen to finish it. */
    val callback: StateFlow<String?> = _callback.asStateFlow()

    /** The page of [start] (for [server]) is about to open. */
    fun begin(server: String, start: SsoStart) {
        prefs.edit()
            .putString(KEY_SERVER, server)
            .putString(KEY_STATE, start.state)
            .putString(KEY_VERIFIER, start.verifier)
            .putLong(KEY_STARTED, now())
            .commit()
    }

    /** The sign-in that waits for its callback, unless it was started too long ago. */
    fun pending(): PendingSso? {
        val server = prefs.getString(KEY_SERVER, null) ?: return null
        val state = prefs.getString(KEY_STATE, null) ?: return null
        val verifier = prefs.getString(KEY_VERIFIER, null) ?: return null
        if (now() - prefs.getLong(KEY_STARTED, 0) !in 0..MAX_AGE_MILLIS) return null
        return PendingSso(server, state, verifier)
    }

    /** The browser came back with [url]: kept for the sign-in screen when it is the callback and a sign-in waits for it. */
    fun deliver(url: String): Boolean {
        if (!AccountRules.isSsoCallback(url) || pending() == null) return false
        _callback.value = url
        return true
    }

    /** The waiting sign-in and its callback, taken (and forgotten) to finish it; null when either is missing. */
    fun take(): Pair<PendingSso, String>? {
        val url = _callback.value ?: return null
        _callback.value = null
        val pending = pending()
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
        const val KEY_STARTED = "started_at"

        /** A sign-in left open longer than this is not finished by a late callback. */
        const val MAX_AGE_MILLIS = 30L * 60 * 1000
    }
}
