package dev.reins.android.autopilot

import android.content.Context
import dev.reins.core.AutopilotMode

/**
 * What Stop returns an Overnight to, per scope (every AI, or one), kept on this phone. The core returns to the mode
 * an Overnight interrupted when it ends by itself, but does not say which; Stop is `set_autopilot_mode` with a mode,
 * so the phone remembers it when it starts one ([AutopilotDraft.interrupted]). One started elsewhere stops to the
 * default (Manual), or to following the global mode for an AI: never to something that lets more happen.
 */
class OvernightReturns(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun started(connectionId: String?, interrupted: AutopilotDraft.Interrupted) {
        if (interrupted !is AutopilotDraft.Interrupted.Mode) return
        prefs.edit().putString(key(connectionId), interrupted.mode?.name ?: NONE).apply()
    }

    /** The mode Stop sets: null is the default (every AI) or following the global mode (an AI). */
    fun stopMode(connectionId: String?): AutopilotMode? {
        val name = prefs.getString(key(connectionId), null) ?: return null
        return AutopilotMode.entries.firstOrNull { it.name == name }?.takeUnless { AutopilotText.isTimed(it) || it == AutopilotMode.LOCKDOWN }
    }

    fun stopped(connectionId: String?) {
        prefs.edit().remove(key(connectionId)).apply()
    }

    /** Signing out forgets them with the account. */
    fun clear() {
        prefs.edit().clear().apply()
    }

    private fun key(connectionId: String?) = if (connectionId == null) GLOBAL else "c:$connectionId"

    private companion object {
        const val PREFS = "autopilot-overnight"
        const val GLOBAL = "global"
        const val NONE = "-"
    }
}
