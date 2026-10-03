package dev.rewarden.android.design

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * Rewarden Light / Rewarden Dark: cool neutrals and one violet accent (the same family as the other apps by this
 * author). Colour only; nothing here affects layout.
 */
@Immutable
class RColors(val dark: Boolean) {
    private fun pick(light: Long, dark: Long, alpha: Float = 1f): Color = Color(argb(if (this.dark) dark else light, alpha))

    val background = pick(0xF3F3F5, 0x060606)
    val elevated = pick(0xFFFFFF, 0x111113)
    val text = pick(0x27272C, 0xE8E8EA)
    val secondary = pick(0x62626A, 0xA9A9AE)
    val tertiary = pick(0x97979F, 0x6B6B72)
    val hairline = pick(0xE2E2E6, 0x1E1E22)
    val accent = pick(0x5B43E8, 0x8B7CF6)
    val accentSoft = pick(0x5B43E8, 0x8B7CF6, 0.12f)

    /** Translucent control fill that reads on any surface in both modes. */
    val controlFill = pick(0x27272C, 0xE8E8EA, 0.075f)
    val danger = pick(0xDC2626, 0xF87171)
    val success = pick(0x15803D, 0x34D399)
    val warning = pick(0xA16207, 0xFACC15)

    // What an operation is: each kind has its own colour so a list reads at a glance.
    val search = pick(0x2563EB, 0x60A5FA)
    val read = pick(0x0F766E, 0x2DD4BF)
    val send = pick(0xC2410C, 0xFB923C)
    val grant = accent
    val pair = pick(0x0E7490, 0x22D3EE)
    val accounts = pick(0xBE185D, 0xF472B6)

    /** Floating chrome (dialogs, toasts): a near-opaque plate with a hairline edge. */
    val glass = if (dark) Color(0xFC1C1C1F.toInt()) else Color(0xFDFFFFFF.toInt())
    val glassEdge = if (dark) Color.White.copy(alpha = 0.08f) else Color.Black.copy(alpha = 0.07f)
    val scrim = Color.Black.copy(alpha = if (dark) 0.55f else 0.32f)

    companion object {
        fun argb(rgb: Long, alpha: Float = 1f): Int =
            ((alpha.coerceIn(0f, 1f) * 255f + 0.5f).toInt() shl 24) or (rgb and 0xFFFFFF).toInt()

        val Light = RColors(dark = false)
        val Dark = RColors(dark = true)
    }
}

val LocalColors = staticCompositionLocalOf { RColors.Dark }
