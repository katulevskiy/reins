package dev.reins.android.design

import androidx.compose.ui.graphics.Color

/** Reins Signal colour tokens. Generated from signal.tokens.json. Fields are identical in both themes. */
class SignalColors(val dark: Boolean) {
    private fun c(l: Long, la: Float, d: Long, da: Float) = if (dark) Color(d or 0xFF000000).copy(alpha = da) else Color(l or 0xFF000000).copy(alpha = la)
    val paper = c(0xF6F6F8, 1.0f, 0x121215, 1.0f)
    val paperRaised = c(0xFFFFFF, 1.0f, 0x1C1C21, 1.0f)
    val paperSunken = c(0xECECF0, 1.0f, 0x26262C, 1.0f)
    val ink = c(0x16151B, 1.0f, 0xEDEDF0, 1.0f)
    val ink2 = c(0x5F5E6B, 1.0f, 0xA6A6B0, 1.0f)
    val inkMeta = c(0x6B6A78, 1.0f, 0x8E8E99, 1.0f)
    val inkFaint = c(0xA3A2AD, 1.0f, 0x63636E, 1.0f)
    val rule = c(0xE3E3EA, 1.0f, 0x2A2A30, 1.0f)
    val inverse = c(0x16151B, 1.0f, 0xEDEDF0, 1.0f)
    val onInverse = c(0xFFFFFF, 1.0f, 0x121215, 1.0f)
    val violet = c(0x5B43E8, 1.0f, 0xA196FF, 1.0f)
    val violetFill = c(0x5B43E8, 1.0f, 0x5B43E8, 1.0f)
    val success = c(0x15803D, 1.0f, 0x34D399, 1.0f)
    val danger = c(0xB91C1C, 1.0f, 0xF87171, 1.0f)
    val dangerWash = c(0xFBE3E3, 1.0f, 0xF87171, 0.14f)
    val dangerFill2 = c(0xF6C9C9, 1.0f, 0xF87171, 0.28f)
    val warning = c(0xA16207, 1.0f, 0xFACC15, 1.0f)
    val scrim = c(0x16151B, 0.32f, 0x000000, 0.55f)
    val fieldPush = c(0xC2410C, 1.0f, 0xC2410C, 1.0f)
    val fieldRead = c(0x2563EB, 1.0f, 0x2563EB, 1.0f)
    val fieldDelete = c(0xB91C1C, 1.0f, 0xB91C1C, 1.0f)
    val fieldGrant = c(0x5B43E8, 1.0f, 0x5B43E8, 1.0f)
    val fieldPair = c(0x0E7490, 1.0f, 0x0E7490, 1.0f)
    val fieldAccounts = c(0xBE185D, 1.0f, 0xBE185D, 1.0f)
    val fieldWarn = c(0xA16207, 1.0f, 0xA16207, 1.0f)
    val onField = c(0xFFFFFF, 1.0f, 0xFFFFFF, 1.0f)
    val onField2 = c(0xFFFFFF, 0.82f, 0xFFFFFF, 0.82f)
    val onFieldWash = c(0xFFFFFF, 0.18f, 0xFFFFFF, 0.18f)
    val rule2 = c(0xD3D3DC, 1.0f, 0x3A3A42, 1.0f)
    val kindPush = c(0xC2410C, 1.0f, 0xFB923C, 1.0f)
    val kindRead = c(0x2563EB, 1.0f, 0x60A5FA, 1.0f)
    val kindDelete = c(0xB91C1C, 1.0f, 0xF87171, 1.0f)
    val kindGrant = c(0x5B43E8, 1.0f, 0xA196FF, 1.0f)
    val kindPair = c(0x0E7490, 1.0f, 0x22D3EE, 1.0f)
    val kindAccounts = c(0xBE185D, 1.0f, 0xF472B6, 1.0f)
    val kindWarn = c(0xA16207, 1.0f, 0xFACC15, 1.0f)

    companion object {
        val Light = SignalColors(false)
        val Dark = SignalColors(true)
    }
}
