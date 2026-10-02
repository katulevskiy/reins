package dev.rewarden.android.state

import android.content.Context
import androidx.core.content.edit

/**
 * Small facts the app remembers between runs: whether this phone lost (or holds) the approval-device role, whether it
 * still waits to open the account's keys, and how far the user has read the activity list. Only touched from
 * background dispatchers.
 */
class DeviceStatusStore(context: Context) {
    private val appContext = context.applicationContext

    /** Opened on first use, which is always on a background dispatcher. */
    private val prefs by lazy { appContext.getSharedPreferences("device_status", Context.MODE_PRIVATE) }

    fun isReplaced(): Boolean = prefs.getBoolean(KEY_REPLACED, false)

    fun setReplaced(replaced: Boolean) = prefs.edit {
        putBoolean(KEY_REPLACED, replaced)
        if (replaced) putBoolean(KEY_APPROVAL_DEVICE, false)
    }

    fun isApprovalDevice(): Boolean = prefs.getBoolean(KEY_APPROVAL_DEVICE, false)

    fun setApprovalDevice(value: Boolean) = prefs.edit { putBoolean(KEY_APPROVAL_DEVICE, value) }

    /** Signed in through "Continue", but the account's keys are on another phone (or behind the recovery code). */
    fun keysLocked(): Boolean = prefs.getBoolean(KEY_KEYS_LOCKED, false)

    fun setKeysLocked(value: Boolean) = prefs.edit { putBoolean(KEY_KEYS_LOCKED, value) }

    fun seenActivityId(): Long = prefs.getLong(KEY_SEEN_ACTIVITY, 0L)

    fun setSeenActivityId(id: Long) = prefs.edit { putLong(KEY_SEEN_ACTIVITY, id) }

    /** Forgets everything about the signed-in account. */
    fun clear() = prefs.edit { clear() }

    private companion object {
        const val KEY_REPLACED = "replaced"
        const val KEY_APPROVAL_DEVICE = "approval_device"
        const val KEY_SEEN_ACTIVITY = "seen_activity"
        const val KEY_KEYS_LOCKED = "keys_locked"
    }
}
