package dev.reins.android.design

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * Reins Signal colours (`design/signal/tokens/signal.tokens.json`), light and dark. Paper and ink flip with the theme;
 * the seven fields are the same in both. Colour only; nothing here affects layout.
 *
 * The names before Signal stay as deprecated aliases of the new tokens, so every screen recolours with no layout
 * change; phase 2 renames the callers screen by screen and the aliases go last.
 */
@Immutable
class RColors(val dark: Boolean) {
    private fun pick(light: Long, dark: Long, alpha: Float = 1f): Color = Color(argb(if (this.dark) dark else light, alpha))

    private fun pick(light: Long, lightAlpha: Float, dark: Long, darkAlpha: Float): Color =
        if (this.dark) Color(argb(dark, darkAlpha)) else Color(argb(light, lightAlpha))

    // Paper.
    /** The ground of every screen. */
    val paper = pick(0xF6F6F8, 0x121215)

    /** Sheets, inputs, header buttons, the Android nav bar. */
    val paperRaised = pick(0xFFFFFF, 0x1C1C21)

    /** Segmented wells, steppers, soft buttons, tags, a switch that is off. */
    val paperSunken = pick(0xECECF0, 0x26262C)

    // Ink.
    /** Titles, asks on paper, row titles, values. */
    val ink = pick(0x16151B, 0xEDEDF0)

    /** Row keys, body copy, values that aren't the point. */
    val ink2 = pick(0x5F5E6B, 0xA6A6B0)

    /** Small meta: senders, dates, section labels, hints. */
    val inkMeta = pick(0x6B6A78, 0x8E8E99)

    /** Chevrons, unchecked rings, the sheet grabber. Decoration only, never text. */
    val inkFaint = pick(0xA3A2AD, 0x63636E)

    /** The hairline under every row and around raised plates. */
    val rule = pick(0xE3E3EA, 0x2A2A30)

    /** Stronger hairline: outline buttons, input borders. */
    val rule2 = pick(0xD3D3DC, 0x3A3A42)

    /** The main button and the tab pill: ink turned into a plate. */
    val inverse = pick(0x16151B, 0xEDEDF0)

    /** Text and glyphs on [inverse]. */
    val onInverse = pick(0xFFFFFF, 0x121215)

    // Violet: Reins and Autopilot.
    /** Violet as text or a glyph on paper. */
    val violet = pick(0x5B43E8, 0xA196FF)

    /** Violet as a fill: switches that are on, the Auto chip, badges. White on it. */
    val violetFill = pick(0x5B43E8, 0x5B43E8)

    // Status on paper.
    val success = pick(0x15803D, 0x34D399)
    val danger = pick(0xB91C1C, 0xF87171)

    /** Behind a red button on paper (End now, Bypass, Hold to delete). */
    val dangerWash = pick(0xFBE3E3, 1f, 0xF87171, 0.14f)

    /** The progress that fills a hold button. */
    val dangerFill2 = pick(0xF6C9C9, 1f, 0xF87171, 0.28f)
    val warning = pick(0xA16207, 0xFACC15)

    /** Behind sheets; the field stays visible, dimmed. */
    val scrim = pick(0x16151B, 0.32f, 0x000000, 0.55f)

    // Fields: the same in both themes (see [Field]).
    val fieldPush = Color(0xFFC2410C)
    val fieldRead = Color(0xFF2563EB)
    val fieldDelete = Color(0xFFB91C1C)
    val fieldGrant = Color(0xFF5B43E8)
    val fieldPair = Color(0xFF0E7490)
    val fieldAccounts = Color(0xFFBE185D)
    val fieldWarn = Color(0xFFA16207)

    /** Text, glyphs and the Review button on any field. */
    val onField = Color.White

    /** Secondary text on a field (14 px and up only). */
    val onField2 = Color.White.copy(alpha = 0.82f)

    /** Translucent buttons and the close button on a field. */
    val onFieldWash = Color.White.copy(alpha = 0.18f)

    // Kind inks: a decided item's tile (the ink at 13 %, see [Field.tile]); lighter in the dark.
    val kindPush = pick(0xC2410C, 0xFB923C)
    val kindRead = pick(0x2563EB, 0x60A5FA)
    val kindDelete = pick(0xB91C1C, 0xF87171)
    val kindGrant = pick(0x5B43E8, 0xA196FF)
    val kindPair = pick(0x0E7490, 0x22D3EE)
    val kindAccounts = pick(0xBE185D, 0xF472B6)
    val kindWarn = pick(0xA16207, 0xFACC15)

    // ---- Before Signal: aliases of the tokens above, removed once phase 2 has renamed their callers. ----

    @Deprecated("Signal: paper", ReplaceWith("paper"))
    val background get() = paper

    @Deprecated("Signal: paperRaised", ReplaceWith("paperRaised"))
    val elevated get() = paperRaised

    @Deprecated("Signal: ink", ReplaceWith("ink"))
    val text get() = ink

    @Deprecated("Signal: ink2", ReplaceWith("ink2"))
    val secondary get() = ink2

    @Deprecated("Signal: inkMeta", ReplaceWith("inkMeta"))
    val tertiary get() = inkMeta

    @Deprecated("Signal: rule", ReplaceWith("rule"))
    val hairline get() = rule

    @Deprecated("Signal: violet", ReplaceWith("violet"))
    val accent get() = violet

    @Deprecated("Signal: violet at 13 %")
    val accentSoft = violet.copy(alpha = 0.13f)

    @Deprecated("Signal: paperSunken", ReplaceWith("paperSunken"))
    val controlFill get() = paperSunken

    /** Overnight runs on the violet grant field (plan: Overnight). */
    @Deprecated("Signal: kindGrant", ReplaceWith("kindGrant"))
    val night get() = kindGrant

    @Deprecated("Signal: kindRead (reads and searches are both read)", ReplaceWith("kindRead"))
    val search get() = kindRead

    @Deprecated("Signal: kindRead", ReplaceWith("kindRead"))
    val read get() = kindRead

    @Deprecated("Signal: kindPush", ReplaceWith("kindPush"))
    val send get() = kindPush

    @Deprecated("Signal: kindGrant", ReplaceWith("kindGrant"))
    val grant get() = kindGrant

    @Deprecated("Signal: kindPair", ReplaceWith("kindPair"))
    val pair get() = kindPair

    @Deprecated("Signal: kindAccounts", ReplaceWith("kindAccounts"))
    val accounts get() = kindAccounts

    /** Floating chrome (dialogs, toasts): now a raised paper plate. */
    @Deprecated("Signal: paperRaised", ReplaceWith("paperRaised"))
    val glass get() = paperRaised

    @Deprecated("Signal: rule", ReplaceWith("rule"))
    val glassEdge get() = rule

    companion object {
        fun argb(rgb: Long, alpha: Float = 1f): Int =
            ((alpha.coerceIn(0f, 1f) * 255f + 0.5f).toInt() shl 24) or (rgb and 0xFFFFFF).toInt()

        val Light = RColors(dark = false)
        val Dark = RColors(dark = true)
    }
}

val LocalColors = staticCompositionLocalOf { RColors.Dark }
