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
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color

/** Whether countdowns and other clocks tick live. Screenshot tests switch it off for a still frame. */
val LocalLiveTimers = compositionLocalOf { true }

@Composable
fun ReinsTheme(content: @Composable () -> Unit) {
    val colors = if (isSystemInDarkTheme()) RColors.Dark else RColors.Light
    val selection = remember(colors) { TextSelectionColors(colors.accent, colors.accent.copy(alpha = 0.3f)) }
    // The few Material 3 pieces we use (loaders, sliders, shapes) take their colours from the same palette.
    val scheme = remember(colors) { materialScheme(colors) }
    MaterialTheme(colorScheme = scheme) {
        CompositionLocalProvider(
            LocalColors provides colors,
            LocalTextSelectionColors provides selection,
            LocalLiveTimers provides Timers.live,
            content = content,
        )
    }
}

private fun materialScheme(c: RColors) = if (c.dark) {
    darkColorScheme(
        primary = c.accent,
        onPrimary = Color.White,
        primaryContainer = c.accentSoft,
        background = c.background,
        surface = c.elevated,
        onSurface = c.text,
        onSurfaceVariant = c.secondary,
        outline = c.tertiary,
        outlineVariant = c.hairline,
        error = c.danger,
    )
} else {
    lightColorScheme(
        primary = c.accent,
        onPrimary = Color.White,
        primaryContainer = c.accentSoft,
        background = c.background,
        surface = c.elevated,
        onSurface = c.text,
        onSurfaceVariant = c.secondary,
        outline = c.tertiary,
        outlineVariant = c.hairline,
        error = c.danger,
    )
}
