package dev.reins.android.platform.assistant

import android.content.Context

/**
 * Whether Gemini may call Reins. Off until the user turns it on for this Reins profile.
 * Personal and a work profile share one Android user, so the key is the profile id.
 */
internal class AssistantStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    fun enabled(profileId: String): Boolean = prefs.getBoolean(key(profileId), false)

    /** Waits until the value is stored, so a reconcile that starts after this sees the click. */
    fun setEnabled(profileId: String, enabled: Boolean) {
        check(prefs.edit().putBoolean(key(profileId), enabled).commit())
    }

    private fun key(profileId: String) = "$KEY.$profileId"

    private companion object {
        const val NAME = "assistant"
        const val KEY = "gemini"
    }
}
