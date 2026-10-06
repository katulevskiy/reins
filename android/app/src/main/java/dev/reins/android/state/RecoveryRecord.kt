package dev.reins.android.state

import android.content.Context
import java.security.MessageDigest

/** Only the acknowledgement's fingerprint is persisted; the recovery secret stays in the encrypted core. */
class RecoveryRecord(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("recovery-record", Context.MODE_PRIVATE)

    fun confirmed(server: String, code: String): Boolean = prefs.getBoolean(key(server, code), false)

    /** Commit before removing the gate, so a killed process cannot lose the acknowledgement. */
    fun confirm(server: String, code: String): Boolean = prefs.edit().putBoolean(key(server, code), true).commit()

    companion object {
        internal fun key(server: String, code: String): String {
            val identity = server.trim().trimEnd('/').lowercase(java.util.Locale.ROOT) + "|" + code
            return MessageDigest.getInstance("SHA-256").digest(identity.toByteArray(Charsets.UTF_8))
                .joinToString("") { "%02x".format(it) }
        }
    }
}
