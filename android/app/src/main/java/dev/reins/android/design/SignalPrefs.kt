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

/** Settings › Appearance: paper follows the system, or stays Paper (light) or Graphite (dark). */
enum class ThemeChoice(val label: String) {
    System("System"),
    Paper("Paper"),
    Graphite("Graphite"),
    ;

    /** Whether the app draws dark, given what the system asks for. */
    fun dark(systemDark: Boolean): Boolean = when (this) {
        System -> systemDark
        Paper -> false
        Graphite -> true
    }
}

/** The Appearance choices, kept on this phone: paper or graphite, the sans and the field grain. */
@Immutable
data class AppearanceChoice(val geist: Boolean = false, val grain: Boolean = true, val theme: ThemeChoice = ThemeChoice.System)

object Appearance {
    private const val PREFS = "appearance"
    private const val KEY_SANS = "sans"
    private const val KEY_GRAIN = "grain"
    private const val KEY_THEME = "theme"
    private var state: MutableState<AppearanceChoice>? = null

    fun state(context: Context): MutableState<AppearanceChoice> = state ?: mutableStateOf(load(context)).also {
        state = it
        Fonts.geist = it.value.geist
    }

    fun set(context: Context, choice: AppearanceChoice) {
        prefs(context).edit()
            .putString(KEY_SANS, if (choice.geist) "geist" else "instrument")
            .putBoolean(KEY_GRAIN, choice.grain)
            .putString(KEY_THEME, choice.theme.name)
            .apply()
        Fonts.geist = choice.geist
        state(context).value = choice
    }

    private fun load(context: Context): AppearanceChoice {
        val p = prefs(context)
        return AppearanceChoice(
            geist = p.getString(KEY_SANS, null) == "geist",
            grain = p.getBoolean(KEY_GRAIN, true),
            theme = ThemeChoice.entries.firstOrNull { it.name == p.getString(KEY_THEME, null) } ?: ThemeChoice.System,
        )
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
