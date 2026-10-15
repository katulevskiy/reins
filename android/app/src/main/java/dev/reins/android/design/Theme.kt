package dev.reins.android.design

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.text.selection.LocalTextSelectionColors
import androidx.compose.foundation.text.selection.TextSelectionColors
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

/** Whether countdowns and other clocks tick live. Screenshot tests switch it off for a still frame. */
val LocalLiveTimers = compositionLocalOf { true }

@Composable
fun ReinsTheme(content: @Composable () -> Unit) {
    val context = LocalContext.current
    val appearance by Appearance.state(context)
    // Paper or Graphite override the system (Settings › Appearance).
    val dark = appearance.theme.dark(isSystemInDarkTheme())
    val colors = if (dark) RColors.Dark else RColors.Light
    val selection = remember(colors) { TextSelectionColors(colors.violet, colors.violet.copy(alpha = 0.3f)) }
    // The few Material 3 pieces we use (loaders, sliders, shapes) take their colours from the same palette.
    val scheme = remember(colors) { materialScheme(colors) }
    // The status and navigation bar icons follow the paper, not the system.
    val window = (context as? android.app.Activity)?.window
    if (window != null) {
        androidx.compose.runtime.SideEffect {
            androidx.core.view.WindowCompat.getInsetsController(window, window.decorView).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    val highContrast = rememberHighContrast()
    val prefs = remember(appearance, highContrast) { SignalPrefs(grain = appearance.grain && !highContrast) }
    val reduceMotion = rememberReduceMotion()
    MaterialTheme(colorScheme = scheme) {
        CompositionLocalProvider(
            LocalColors provides colors,
            LocalTextSelectionColors provides selection,
            LocalLiveTimers provides Timers.live,
            LocalSignalPrefs provides prefs,
            LocalReduceMotion provides reduceMotion,
            content = content,
        )
    }
}

private fun materialScheme(c: RColors) = if (c.dark) {
    darkColorScheme(
        primary = c.violet,
        onPrimary = c.paper,
        primaryContainer = c.violet.copy(alpha = 0.13f),
        background = c.paper,
        onBackground = c.ink,
        surface = c.paperRaised,
        onSurface = c.ink,
        surfaceVariant = c.paperSunken,
        onSurfaceVariant = c.ink2,
        outline = c.rule2,
        outlineVariant = c.rule,
        error = c.danger,
        onError = c.paper,
        scrim = c.scrim,
    )
} else {
    lightColorScheme(
        primary = c.violet,
        onPrimary = Color.White,
        primaryContainer = c.violet.copy(alpha = 0.13f),
        background = c.paper,
        onBackground = c.ink,
        surface = c.paperRaised,
        onSurface = c.ink,
        surfaceVariant = c.paperSunken,
        onSurfaceVariant = c.ink2,
        outline = c.rule2,
        outlineVariant = c.rule,
        error = c.danger,
        onError = Color.White,
        scrim = c.scrim,
    )
}
