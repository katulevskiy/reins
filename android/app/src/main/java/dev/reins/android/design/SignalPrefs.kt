package dev.reins.android.design

import android.app.UiModeManager
import android.content.Context
import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat

/** What the drawing of the app follows, beyond the theme. */
@Immutable
data class SignalPrefs(
    /** The grain on colour fields: the Appearance switch, and off when the system asks for more contrast. */
    val grain: Boolean = true,
)

val LocalSignalPrefs = staticCompositionLocalOf { SignalPrefs() }

/** The Appearance choices, kept on this phone: the sans and the field grain. */
@Immutable
data class AppearanceChoice(val geist: Boolean = false, val grain: Boolean = true)

object Appearance {
    private const val PREFS = "appearance"
    private const val KEY_SANS = "sans"
    private const val KEY_GRAIN = "grain"
    private var state: MutableState<AppearanceChoice>? = null

    fun state(context: Context): MutableState<AppearanceChoice> = state ?: mutableStateOf(load(context)).also {
        state = it
        Fonts.geist = it.value.geist
    }

    fun set(context: Context, choice: AppearanceChoice) {
        prefs(context).edit().putString(KEY_SANS, if (choice.geist) "geist" else "instrument").putBoolean(KEY_GRAIN, choice.grain).apply()
        Fonts.geist = choice.geist
        state(context).value = choice
    }

    private fun load(context: Context): AppearanceChoice {
        val p = prefs(context)
        return AppearanceChoice(geist = p.getString(KEY_SANS, null) == "geist", grain = p.getBoolean(KEY_GRAIN, true))
    }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}

/** Whether the system asks for more contrast (API 34+: [UiModeManager.getContrast] above 0), kept up to date. */
@Composable
fun rememberHighContrast(): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return false
    val context = LocalContext.current
    val ui = remember(context) { context.getSystemService(UiModeManager::class.java) }
    var high by remember(ui) { mutableStateOf((ui?.contrast ?: 0f) > 0f) }
    DisposableEffect(ui) {
        if (ui == null) return@DisposableEffect onDispose {}
        val listener = UiModeManager.ContrastChangeListener { contrast -> high = contrast > 0f }
        ui.addContrastChangeListener(ContextCompat.getMainExecutor(context), listener)
        onDispose { ui.removeContrastChangeListener(listener) }
    }
    return high
}
