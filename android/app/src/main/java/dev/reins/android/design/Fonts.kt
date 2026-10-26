package dev.reins.android.design

import android.content.Context
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.AndroidFont
import androidx.compose.ui.text.font.FontLoadingStrategy
import androidx.compose.ui.text.font.FontVariation
import android.graphics.fonts.Font as PlatformFont
import android.graphics.fonts.FontFamily as PlatformFamily
import android.graphics.fonts.FontStyle as PlatformStyle

/**
 * The bundled faces (`assets/fonts`, Reins Next): Sora for all text (400, 500, 600), with Geologica 500 under it for
 * Cyrillic only (Sora has no Cyrillic, so Latin always stays Sora), and JetBrains Mono (400, 500) for what the
 * machine says: branches, hashes, addresses, codes, diffs. No weight 700 anywhere.
 */
object Fonts {
    private lateinit var app: Context

    fun init(context: Context) {
        app = context.applicationContext
    }

    private val SORA = listOf(
        FontWeight.Normal to "fonts/Sora-Regular.ttf",
        FontWeight.Medium to "fonts/Sora-Medium.ttf",
        FontWeight.SemiBold to "fonts/Sora-SemiBold.ttf",
    )
    private const val GEOLOGICA = "fonts/Geologica-Medium-Cyrillic.ttf"

    /**
     * Sora with Geologica as its fallback: for each weight, a platform typeface whose first family is that Sora and
     * whose custom fallback is Geologica, so a Cyrillic letter is drawn in Geologica inside the same line as Sora.
     * Plain Sora where the platform cannot build the chain.
     */
    val sans: FontFamily by lazy {
        runCatching {
            val assets = app.assets
            val geologica = PlatformFamily.Builder(PlatformFont.Builder(assets, GEOLOGICA).setWeight(500).build()).build()
            FontFamily(
                SORA.map { w ->
                    val sora = PlatformFamily.Builder(platformFont(w)).build()
                    val typeface = android.graphics.Typeface.CustomFallbackBuilder(sora)
                        .addCustomFallback(geologica)
                        .setStyle(PlatformStyle(w.first.weight, PlatformStyle.FONT_SLANT_UPRIGHT))
                        .build()
                    LoadedFont(typeface, w.first)
                },
            )
        }.getOrElse { soraOnly }
    }

    private fun platformFont(w: Pair<FontWeight, String>): PlatformFont = PlatformFont.Builder(app.assets, w.second).setWeight(w.first.weight).build()

    /** Sora alone, without the Cyrillic fallback. */
    val soraOnly: FontFamily by lazy {
        FontFamily(SORA.map { (weight, path) -> Font(path, app.assets, weight) })
    }

    val mono: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/JetBrainsMono-Regular.ttf", a, FontWeight.Normal),
            Font("fonts/JetBrainsMono-Medium.ttf", a, FontWeight.Medium),
        )
    }

    /**
     * Loads every face into Compose's shared font cache, off the main thread at process start, so the first screen's
     * text does not read the font files while it draws.
     */
    fun warm() {
        val resolver = androidx.compose.ui.text.font.createFontFamilyResolver(app)
        for (weight in listOf(FontWeight.Normal, FontWeight.Medium, FontWeight.SemiBold)) {
            runCatching { resolver.resolve(sans, weight, FontStyle.Normal) }
        }
        for (weight in listOf(FontWeight.Normal, FontWeight.Medium)) {
            runCatching { resolver.resolve(mono, weight) }
        }
    }
}

/** A font whose platform typeface is already built (Sora with its Cyrillic fallback, at one weight). */
private class LoadedFont(val typeface: android.graphics.Typeface, override val weight: FontWeight) :
    AndroidFont(FontLoadingStrategy.Blocking, Loader, FontVariation.Settings()) {
    override val style: FontStyle = FontStyle.Normal

    private object Loader : AndroidFont.TypefaceLoader {
        override fun loadBlocking(context: Context, font: AndroidFont): android.graphics.Typeface = (font as LoadedFont).typeface

        override suspend fun awaitLoad(context: Context, font: AndroidFont): android.graphics.Typeface = (font as LoadedFont).typeface
    }
}
