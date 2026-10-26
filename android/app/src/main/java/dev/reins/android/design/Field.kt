package dev.reins.android.design

import androidx.compose.ui.graphics.Color

/**
 * The fields (Reins Next keeps Signal's seven and adds Overnight's night): a solid colour appears only when something needs a decision or a mode runs, and its colour says what kind.: a solid colour appears only when something needs a decision, and its colour says what kind.
 * The field colour is the same in both themes; the kind ink (a decided item's tile, a glyph on paper) is lighter in the
 * dark, and the tile is that ink at 13 %.
 */
enum class Field(private val rgb: Long) {
    /** Writes or sends (push, send email, comment). */
    Push(0xC2410C),

    /** Reads or searches. */
    Read(0x2563EB),

    /** Can't be undone, pays, or bypasses. */
    Delete(0xB91C1C),

    /** Power already handed over (a grant; Overnight). */
    Grant(0x5B43E8),

    /** Connects something new (pair a phone, connect an AI). */
    Pair(0x0E7490),

    /** Changes an account (passwords, recovery, sign-in). */
    Accounts(0xBE185D),

    /** Reins is broken and needs you; Lockdown. */
    Warn(0xA16207),

    /** Overnight: risky asks wait for the morning. */
    Night(0x1E2F5C),
    ;

    /** The solid field, the same in light and dark. White text on it is 4.9:1 or more. */
    fun fill(): Color = Color(0xFF000000 or rgb)

    /** The kind's ink on paper: the field colour in light, a lighter one in dark. */
    fun kindInk(c: RColors): Color = when (this) {
        Push -> c.kindPush
        Read -> c.kindRead
        Delete -> c.kindDelete
        Grant -> c.kindGrant
        Pair -> c.kindPair
        Accounts -> c.kindAccounts
        Warn -> c.kindWarn
        Night -> c.kindNight
    }

    /** A kind tile's fill: the ink at 13 %. */
    fun tile(c: RColors): Color = kindInk(c).copy(alpha = TILE_ALPHA)

    companion object {
        const val TILE_ALPHA = 0.13f
    }
}
