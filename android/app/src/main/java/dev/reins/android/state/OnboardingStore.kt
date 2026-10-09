package dev.reins.android.state

import android.content.Context
import androidx.core.content.edit
import dev.reins.core.SessionInfo

/**
 * Whether the short setup after signing in (connect a computer, connect Claude or ChatGPT) is still to be shown, per
 * account. It is only ever started by a sign-in or a new account in the onboarding screens, so people who were signed
 * in before it existed never see it, and it shows once per account. Kept apart from [DeviceStatusStore], which signing
 * out clears. Only touched from background dispatchers.
 */
class OnboardingStore(context: Context) {
    private val appContext = context.applicationContext

    private val prefs by lazy {
        appContext.getSharedPreferences("onboarding", Context.MODE_PRIVATE).also { prefs ->
            prefs.all.keys.filter { it.contains('@') }.forEach { prefs.edit().remove(it).apply() }
        }
    }

    /** Marks the setup as waiting for [account], unless it was already done for it. */
    fun begin(account: SessionInfo) {
        val key = key(account)
        if (prefs.getBoolean(DONE + key, false)) return
        prefs.edit { putBoolean(PENDING + key, true) }
    }

    fun isPending(account: SessionInfo): Boolean = prefs.getBoolean(PENDING + key(account), false)

    fun finish(account: SessionInfo) = prefs.edit {
        val key = key(account)
        remove(PENDING + key)
        putBoolean(DONE + key, true)
    }

    /** The server of the latest sign-in: the welcome screen offers it again after signing out (a self-hosted one). */
    var lastServer: String?
        get() = prefs.getString(LAST_SERVER, null)
        set(value) = prefs.edit { if (value == null) remove(LAST_SERVER) else putString(LAST_SERVER, value) }

    private fun key(account: SessionInfo): String {
        val value = account.serverUrl.trim().trimEnd('/').lowercase(java.util.Locale.ROOT) + " " + account.email.trim().lowercase(java.util.Locale.ROOT)
        return java.security.MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    private companion object {
        const val PENDING = "pending:"
        const val DONE = "done:"
        const val LAST_SERVER = "last-server"
    }
}
