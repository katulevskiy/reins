package dev.rewarden.android.platform.update

import android.content.Context
import androidx.core.content.edit

/** What the updater remembers between runs. Times are unix milliseconds. */
interface UpdateStore {
    /** "Download updates automatically" in Settings; on unless the user turns it off. */
    var autoDownload: Boolean
    var lastCheckAt: Long
    var snoozedVersion: Long
    var snoozedUntil: Long

    /** The newest version a notification was posted for, so none is announced twice. */
    var notifiedVersion: Long

    /** The release (as JSON) whose APK is in the download directory and passed verification. */
    var downloaded: String?
}

/** [UpdateStore] in shared preferences, like the app's other small facts. Only touched from background dispatchers. */
class PrefsUpdateStore(context: Context) : UpdateStore {
    private val appContext = context.applicationContext
    private val prefs by lazy { appContext.getSharedPreferences("updates", Context.MODE_PRIVATE) }

    override var autoDownload: Boolean
        get() = prefs.getBoolean(AUTO, true)
        set(value) = prefs.edit { putBoolean(AUTO, value) }
    override var lastCheckAt: Long
        get() = prefs.getLong(LAST_CHECK, 0)
        set(value) = prefs.edit { putLong(LAST_CHECK, value) }
    override var snoozedVersion: Long
        get() = prefs.getLong(SNOOZED_VERSION, 0)
        set(value) = prefs.edit { putLong(SNOOZED_VERSION, value) }
    override var snoozedUntil: Long
        get() = prefs.getLong(SNOOZED_UNTIL, 0)
        set(value) = prefs.edit { putLong(SNOOZED_UNTIL, value) }
    override var notifiedVersion: Long
        get() = prefs.getLong(NOTIFIED, 0)
        set(value) = prefs.edit { putLong(NOTIFIED, value) }
    override var downloaded: String?
        get() = prefs.getString(DOWNLOADED, null)
        set(value) = prefs.edit { putString(DOWNLOADED, value) }

    private companion object {
        const val AUTO = "auto_download"
        const val LAST_CHECK = "last_check"
        const val SNOOZED_VERSION = "snoozed_version"
        const val SNOOZED_UNTIL = "snoozed_until"
        const val NOTIFIED = "notified_version"
        const val DOWNLOADED = "downloaded"
    }
}
