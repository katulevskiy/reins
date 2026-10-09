package dev.reins.android.state

import android.content.Context
import android.content.SharedPreferences
import dev.reins.core.SessionInfo

/**
 * Where the app stood when it last settled, so the first frame can show it while the core opens: signed out, or signed
 * in on a server with an email (nothing secret; every list still comes from the core). Only a plain signed-in state is
 * kept, with the keys open and nothing to do first (no recovery code, setup, passkey offer or takeover): anything else
 * starts as before, from the core.
 */
class StartupSnapshot(private val prefs: SharedPreferences) {
    constructor(context: Context) : this(context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE))

    /** The state to start from, or null to wait for the core. */
    fun read(): SessionState? = when (prefs.getString(KEY_STATE, null)) {
        SIGNED_OUT -> SessionState.SignedOut
        SIGNED_IN -> {
            val server = prefs.getString(KEY_SERVER, null)
            val email = prefs.getString(KEY_EMAIL, null)
            if (server != null && email != null) SessionState.SignedIn(SessionInfo(server, email)) else null
        }
        else -> null
    }

    /** Keeps [state] for the next start when it is one to start from ([plain]), else forgets what was kept. */
    fun write(state: SessionState, plain: Boolean) {
        val edit = prefs.edit()
        when {
            state is SessionState.SignedOut -> edit.clear().putString(KEY_STATE, SIGNED_OUT)
            state is SessionState.SignedIn && plain ->
                edit.putString(KEY_STATE, SIGNED_IN).putString(KEY_SERVER, state.info.serverUrl).putString(KEY_EMAIL, state.info.email)
            else -> edit.clear()
        }
        edit.apply()
    }

    private companion object {
        const val PREFS = "startup"
        const val KEY_STATE = "state"
        const val KEY_SERVER = "server"
        const val KEY_EMAIL = "email"
        const val SIGNED_IN = "signed_in"
        const val SIGNED_OUT = "signed_out"
    }
}
