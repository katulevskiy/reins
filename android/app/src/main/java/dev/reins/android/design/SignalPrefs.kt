package dev.reins.android.design

import android.app.UiModeManager
import android.content.Context
import android.content.SharedPreferences
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
import kotlin.math.roundToInt

/** What the drawing of the app follows, beyond the theme. */
@Immutable
data class SignalPrefs(
    /** The grain on colour fields: the Appearance switch, and off when the system asks for more contrast. */
    val grain: Boolean = true,
    /**
     * Frosted panes over the wallpaper. Off (solid panes, the art unchanged) only when the system asks for more
     * contrast. Battery Saver and phones without window blurs keep them: the frost is the app's own pre-blurred copy
     * of the art, not a window blur.
     */
    val frosted: Boolean = true,
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

/**
 * A theme's wallpaper: three separate choices. An [image] (shipped or the owner's photo) takes any [effect] and any
 * [fade] into the page. Original with no fade draws the image as it was; an image's own effect with no fade draws a
 * treated image's shipped file as it is ([fromShipped]).
 */
@Immutable
data class WallpaperChoice(
    val image: Wallpaper,
    val effect: PhotoEffect = PhotoEffect.Original,
    /** 0 none … 1 from 30 % of the way down ([PhotoEffects.fadeStart]). */
    val fade: Float = 0f,
) {
    /** The fade in hundredths: the step the slider keeps and the made pictures are keyed by. */
    val fadeStep: Int get() = (fade.coerceIn(0f, 1f) * 100f).roundToInt()

    /** Whether this is drawn from the shipped file of an image that ships with [effect] already in it. */
    val fromShipped: Boolean get() = image.treated && effect == image.defaultEffect

    /** The effect drawn on [Wallpapers.base]: none when the shipped file has it already. */
    val applied: PhotoEffect get() = if (fromShipped) PhotoEffect.Original else effect

    /** Whether the picture is a file as it is: nothing drawn on it and no fade of its own. */
    val plain: Boolean get() = applied == PhotoEffect.Original && fadeStep == 0

    /** The picture this is drawn on, as it is: what shows while it is made. */
    val base: WallpaperChoice get() = WallpaperChoice(image, if (fromShipped) effect else PhotoEffect.Original)

    /** [image] picked in this choice's place: with its own effect ([Wallpaper.defaultEffect]); the fade stays. */
    fun withImage(image: Wallpaper): WallpaperChoice = copy(image = image, effect = image.defaultEffect)

    /**
     * The owner's new photo in this choice's place: the effect stays; with no fade it takes [PHOTO_FADE], so a photo
     * does not end in a hard edge under the page.
     */
    fun withNewPhoto(): WallpaperChoice = copy(image = Wallpaper.Photo, fade = if (fadeStep == 0) PHOTO_FADE else fade)

    companion object {
        /** The fade a new photo gets when the theme had none. */
        const val PHOTO_FADE = 0.6f

        /** [image] as picking it gives it: with its own effect, no fade. */
        fun of(image: Wallpaper): WallpaperChoice = WallpaperChoice(image, image.defaultEffect)

        /**
         * A choice kept before the shipped images came as originals, drawn as it was: Original on a shipped image
         * meant the picture as shipped ([Wallpaper.shippedLook]: Eyes with Scanlines), and so did the old Colour (a
         * saturation lift on the picture). Torii dither is Torii with Mono. An image that ships treated was drawn as it
         * ships whatever its effect: it keeps that look ([treatedAsShipped]). Null for an image no longer known.
         */
        fun fromBefore(image: String?, effect: String?, fade: Float): WallpaperChoice? {
            val toriiDither = image == "ToriiDither"
            val picked = if (toriiDither) Wallpaper.Torii else Wallpaper.named(image) ?: return null
            val asShipped = effect == null || effect == "Original" || effect == "Colour"
            val e = when {
                toriiDither && asShipped -> PhotoEffect.Mono
                asShipped -> picked.shippedLook
                else -> PhotoEffect.named(effect) ?: picked.shippedLook
            }
            return treatedAsShipped(WallpaperChoice(picked, e, fade.coerceIn(0f, 1f)))
        }

        /**
         * A choice kept before every image took every effect: Halo, Crescent, Figure and Ruins had no effect of their
         * own and were drawn as they ship, which is now their own effect. Others are kept as they are.
         */
        fun treatedAsShipped(choice: WallpaperChoice): WallpaperChoice =
            if (choice.image.treated) choice.copy(effect = choice.image.defaultEffect) else choice
    }
}

/**
 * The Appearance choices, kept on this phone: paper or graphite, the field grain, and the wallpaper for each theme
 * (Settings › Wallpaper; Halo with Mono, as it ships, in the dark and Torii with Mono in the light by default).
 */
@Immutable
data class AppearanceChoice(
    val grain: Boolean = true,
    val theme: ThemeChoice = ThemeChoice.System,
    val wallpaperDark: WallpaperChoice = WallpaperChoice.of(Wallpaper.Halo),
    val wallpaperLight: WallpaperChoice = WallpaperChoice.of(Wallpaper.Torii),
) {
    fun wallpaper(dark: Boolean): WallpaperChoice = if (dark) wallpaperDark else wallpaperLight

    fun withWallpaper(dark: Boolean, wallpaper: WallpaperChoice): AppearanceChoice =
        if (dark) copy(wallpaperDark = wallpaper) else copy(wallpaperLight = wallpaper)
}

object Appearance {
    private const val PREFS = "appearance"

    /** Before effects were separate: the image's name alone (a photo had its effect baked in). Read once, then gone. */
    private const val OLD_WALL_DARK = "wallpaperDark"
    private const val OLD_WALL_LIGHT = "wallpaperLight"
    private const val KEY_GRAIN = "grain"
    private const val KEY_THEME = "theme"

    /**
     * How the kept wallpaper keys read: 2 since the shipped images came as originals with an effect of their own, 3
     * since every image takes every effect. Keys from before 2 are read as before ([WallpaperChoice.fromBefore]); from
     * 2, a treated image keeps its look ([WallpaperChoice.treatedAsShipped]). Both are written back as 3.
     */
    private const val KEY_LOOKS = "wallpaperLooks"
    private const val LOOKS = 3
    private var state: MutableState<AppearanceChoice>? = null

    private fun imageKey(dark: Boolean) = if (dark) "wallpaperDarkImage" else "wallpaperLightImage"

    private fun effectKey(dark: Boolean) = if (dark) "wallpaperDarkEffect" else "wallpaperLightEffect"

    private fun fadeKey(dark: Boolean) = if (dark) "wallpaperDarkFade" else "wallpaperLightFade"

    fun state(context: Context): MutableState<AppearanceChoice> = state ?: mutableStateOf(load(context)).also { state = it }

    fun set(context: Context, choice: AppearanceChoice) {
        putWallpapers(prefs(context).edit(), choice).putBoolean(KEY_GRAIN, choice.grain).putString(KEY_THEME, choice.theme.name).apply()
        state(context).value = choice
    }

    private fun putWallpapers(edit: SharedPreferences.Editor, choice: AppearanceChoice): SharedPreferences.Editor {
        for (dark in listOf(true, false)) {
            val w = choice.wallpaper(dark)
            edit.putString(imageKey(dark), w.image.name).putString(effectKey(dark), w.effect.name).putFloat(fadeKey(dark), w.fade)
        }
        return edit.putInt(KEY_LOOKS, LOOKS)
    }

    /** Sets one theme's wallpaper, keeping the rest. */
    fun setWallpaper(context: Context, dark: Boolean, wallpaper: WallpaperChoice) {
        set(context, state(context).value.withWallpaper(dark, wallpaper))
    }

    /**
     * What is kept on this phone. Wallpapers kept before the shipped images came as originals (and the old one-key
     * image, as it was: no effect), or before every image took every effect, are read as they looked
     * ([WallpaperChoice.fromBefore], [WallpaperChoice.treatedAsShipped]) and written back once.
     */
    internal fun load(context: Context): AppearanceChoice {
        val p = prefs(context)
        val old = p.contains(OLD_WALL_DARK) || p.contains(OLD_WALL_LIGHT)
        val looks = p.getInt(KEY_LOOKS, 0)
        val kept = old || p.contains(imageKey(true)) || p.contains(imageKey(false))
        val before = looks < 2 && kept
        val rewrite = looks < LOOKS && kept
        val defaults = AppearanceChoice()
        fun wallpaper(dark: Boolean): WallpaperChoice {
            val image = p.getString(imageKey(dark), null) ?: p.getString(if (dark) OLD_WALL_DARK else OLD_WALL_LIGHT, null)
            val effect = p.getString(effectKey(dark), null)
            val fade = p.getFloat(fadeKey(dark), 0f).coerceIn(0f, 1f)
            val read = if (before) {
                WallpaperChoice.fromBefore(image, effect, fade)
            } else {
                Wallpaper.named(image)?.let { WallpaperChoice(it, PhotoEffect.named(effect) ?: PhotoEffect.Original, fade) }
                    ?.let { if (looks < 3) WallpaperChoice.treatedAsShipped(it) else it }
            }
            return read ?: defaults.wallpaper(dark)
        }
        val choice = AppearanceChoice(
            wallpaperDark = wallpaper(true),
            wallpaperLight = wallpaper(false),
            grain = p.getBoolean(KEY_GRAIN, true),
            theme = ThemeChoice.entries.firstOrNull { it.name == p.getString(KEY_THEME, null) } ?: ThemeChoice.System,
        )
        if (rewrite) putWallpapers(p.edit().remove(OLD_WALL_DARK).remove(OLD_WALL_LIGHT), choice).apply()
        // Pictures made with the old effects (and from the old Eyes) are stale.
        if (before) Wallpapers.oldMadeDir(context).deleteRecursively()
        return choice
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
