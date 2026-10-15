package dev.reins.android.design

import android.animation.ValueAnimator
import android.os.Build
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.TransformOrigin

/**
 * Reins Signal motion (`tokens/signal.tokens.json` motion-time, `screens/notes/Motion.md`): the durations and curves,
 * and their Reduce Motion forms. Under Reduce Motion folds and slides become [REDUCED_MS] cross-fades; holds are never
 * shortened (they run on elapsed time, see [HoldButton]).
 */
object Motion {
    /** Hold to confirm, everywhere. */
    const val HOLD_MS = 1_200

    /** A released hold drains, with no haptic. */
    const val HOLD_DRAIN_MS = 180

    /** A field slides down into place; the ask follows [ASK_DELAY_MS] later. */
    const val FIELD_IN_MS = 320
    const val ASK_DELAY_MS = 60

    /** Colour and hover changes. */
    const val FADE_MS = 150

    /** Moving between waiting asks cross-fades the field colour. */
    const val KIND_CHANGE_MS = 200

    /** The answered field folds into its kind tile. */
    const val FOLD_MS = 360

    /** A flicked waiting card leaves. */
    const val CARD_FLY_MS = 220

    /** The tab pill slides. */
    const val TAB_PILL_MS = 240

    /** A sheet rises over the scrim. */
    const val SHEET_RISE_MS = 280

    /** A pushed page slides in. */
    const val DRILL_IN_MS = 300

    /** A menu grows from its trigger; closes faster. */
    const val MENU_OPEN_MS = 180
    const val MENU_CLOSE_MS = 120

    /** Stats and durations roll digit by digit. */
    const val NUMBER_ROLL_MS = 240

    /** The bypass dot breathes 35 % to 100 % and back. */
    const val BREATHE_MS = 1_800

    /** What every fold and slide becomes under Reduce Motion. */
    const val REDUCED_MS = 150

    /** Field in: ease out (0.2, 0.8, 0.2, 1). */
    val EaseOut: Easing = CubicBezierEasing(0.2f, 0.8f, 0.2f, 1f)

    /** The tab pill: (0.3, 1.25, …), a slight overshoot. */
    val Overshoot: Easing = CubicBezierEasing(0.3f, 1.25f, 0.4f, 1f)

    /** Smooth in-out for breathing and cross-fades. */
    val InOut: Easing = CubicBezierEasing(0.4f, 0f, 0.6f, 1f)

    /** The next waiting card rising: response 0.32 s, damping 0.86 (stiffness = (2π / response)²). */
    fun <T> cardSpring(): FiniteAnimationSpec<T> = spring(dampingRatio = 0.86f, stiffness = 385f)

    fun <T> fieldIn(reduce: Boolean): FiniteAnimationSpec<T> = if (reduce) reduced() else tween(FIELD_IN_MS, easing = EaseOut)

    fun <T> fold(reduce: Boolean): FiniteAnimationSpec<T> = if (reduce) reduced() else tween(FOLD_MS, easing = EaseOut)

    fun <T> tabPill(reduce: Boolean): FiniteAnimationSpec<T> = if (reduce) reduced() else tween(TAB_PILL_MS, easing = Overshoot)

    fun <T> menuOpen(): FiniteAnimationSpec<T> = tween(MENU_OPEN_MS, easing = EaseOut)

    fun <T> menuClose(): FiniteAnimationSpec<T> = tween(MENU_CLOSE_MS, easing = LinearEasing)

    fun <T> fade(): FiniteAnimationSpec<T> = tween(FADE_MS)

    fun <T> reduced(): FiniteAnimationSpec<T> = tween(REDUCED_MS)

    /** A field sliding down from the top (a cross-fade under Reduce Motion). */
    fun fieldEnter(reduce: Boolean): EnterTransition =
        if (reduce) fadeIn(reduced()) else slideInVertically(fieldIn(false)) { -it } + fadeIn(tween(FIELD_IN_MS / 2))

    fun fieldExit(reduce: Boolean): ExitTransition =
        if (reduce) fadeOut(reduced()) else slideOutVertically(tween(FIELD_IN_MS, easing = EaseOut)) { -it } + fadeOut(tween(FIELD_IN_MS))

    /** A menu growing from [origin] (scale 0.92 and fade); Reduce Motion keeps the fade only. */
    fun menuEnter(origin: TransformOrigin, reduce: Boolean): EnterTransition =
        if (reduce) fadeIn(reduced()) else scaleIn(menuOpen(), initialScale = 0.92f, transformOrigin = origin) + fadeIn(menuOpen())

    fun menuExit(origin: TransformOrigin, reduce: Boolean): ExitTransition =
        if (reduce) fadeOut(menuClose()) else scaleOut(menuClose(), targetScale = 0.92f, transformOrigin = origin) + fadeOut(menuClose())
}

/** Whether the system asked for less motion ("Remove animations"): folds and slides become cross-fades. */
val LocalReduceMotion = staticCompositionLocalOf { false }

/** Reduce Motion, from [ValueAnimator.areAnimatorsEnabled], kept up to date while composed (API 33+ tells us). */
@Composable
fun rememberReduceMotion(): Boolean {
    var reduce by remember { mutableStateOf(!ValueAnimator.areAnimatorsEnabled()) }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        DisposableEffect(Unit) {
            val listener = ValueAnimator.DurationScaleChangeListener { scale -> reduce = scale == 0f }
            ValueAnimator.registerDurationScaleChangeListener(listener)
            onDispose { ValueAnimator.unregisterDurationScaleChangeListener(listener) }
        }
    }
    return reduce
}
