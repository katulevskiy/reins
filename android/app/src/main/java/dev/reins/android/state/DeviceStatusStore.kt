package dev.reins.android.state

import android.content.Context
import dev.reins.android.platform.SealedPreferences

/**
 * Small facts the app remembers between runs: whether this phone lost (or holds) the approval-device role, whether it
 * still waits to open the account's keys, and how far the user has read the activity list. Only touched from
 * background dispatchers.
 */
class DeviceStatusStore(context: Context) {
    private val appContext = context.applicationContext

    /** Opened on first use, which is always on a background dispatcher. */
    private val prefs by lazy { SealedPreferences(appContext, "device_status") }

    fun selectAccount(info: dev.reins.core.SessionInfo) {
        val owner = java.security.MessageDigest.getInstance("SHA-256")
            .digest((info.serverUrl + "\u0000" + info.email).toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it) }
        if (prefs.getString("account", null) != owner) {
            prefs.edit().apply { clear(); putString("account", owner) }.commit()
        }
    }

    fun isReplaced(): Boolean = prefs.getBoolean(KEY_REPLACED, false)

    fun setReplaced(replaced: Boolean) = prefs.edit().apply {
        putBoolean(KEY_REPLACED, replaced)
        if (replaced) putBoolean(KEY_APPROVAL_DEVICE, false)
    }.apply()

    fun isApprovalDevice(): Boolean = prefs.getBoolean(KEY_APPROVAL_DEVICE, false)

    fun setApprovalDevice(value: Boolean) = prefs.edit().apply { putBoolean(KEY_APPROVAL_DEVICE, value) }.apply()

    /** Signed in through "Continue", but the account's keys are on another phone (or behind the recovery code). */
    fun keysLocked(): Boolean = prefs.getBoolean(KEY_KEYS_LOCKED, false)

    fun setKeysLocked(value: Boolean) = prefs.edit().apply { putBoolean(KEY_KEYS_LOCKED, value) }.apply()

    /**
     * The server refused to make this phone the approval device: another phone approves for the account, and this one
     * must be approved from it (or bring the recovery code) first.
     */
    fun needsTakeover(): Boolean = prefs.getBoolean(KEY_TAKEOVER, false)

    fun setNeedsTakeover(value: Boolean) = prefs.edit().apply { putBoolean(KEY_TAKEOVER, value) }.apply()

    fun seenActivityId(): Long = prefs.getLong(KEY_SEEN_ACTIVITY, 0L)

    fun setSeenActivityId(id: Long) = prefs.edit().apply { putLong(KEY_SEEN_ACTIVITY, id) }.apply()

    /** Forgets everything about the signed-in account. */
    fun clear() = prefs.edit().apply { clear() }.apply()

    private companion object {
        const val KEY_REPLACED = "replaced"
        const val KEY_APPROVAL_DEVICE = "approval_device"
        const val KEY_SEEN_ACTIVITY = "seen_activity"
        const val KEY_KEYS_LOCKED = "keys_locked"
        const val KEY_TAKEOVER = "needs_takeover"
    }
}
