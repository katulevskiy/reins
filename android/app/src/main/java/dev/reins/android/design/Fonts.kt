package dev.reins.android.design

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight

/**
 * The bundled faces (`assets/fonts`): Instrument Serif for what you decide on, Instrument Sans (or Geist, the
 * Appearance alternative) for what you read, Geist Mono for what the machine says. No weight 700 anywhere.
 */
object Fonts {
    private lateinit var app: Context

    fun init(context: Context) {
        app = context.applicationContext
    }

    /** Geist instead of Instrument Sans (Appearance). Snapshot state, so text recomposes in the other face. */
    var geist by mutableStateOf(false)

    val instrumentSans: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/InstrumentSans-Regular.ttf", a, FontWeight.Normal),
            Font("fonts/InstrumentSans-Medium.ttf", a, FontWeight.Medium),
            Font("fonts/InstrumentSans-SemiBold.ttf", a, FontWeight.SemiBold),
        )
    }

    val geistSans: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/Geist.ttf", a, FontWeight.Normal),
            Font("fonts/Geist-Medium.ttf", a, FontWeight.Medium),
            Font("fonts/Geist-SemiBold.ttf", a, FontWeight.SemiBold),
        )
    }

    /** The sans in use: Instrument Sans unless Appearance picked Geist. */
    val sans: FontFamily get() = if (geist) geistSans else instrumentSans

    /** Instrument Serif, upright and italic, one weight (400): asks, titles, numbers, the mood line. Never bold. */
    val serif: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/InstrumentSerif-Regular.ttf", a, FontWeight.Normal, FontStyle.Normal),
            Font("fonts/InstrumentSerif-Italic.ttf", a, FontWeight.Normal, FontStyle.Italic),
        )
    }

    val mono: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/GeistMono.ttf", a, FontWeight.Normal),
            Font("fonts/GeistMono-Medium.ttf", a, FontWeight.Medium),
        )
    }

    /**
     * Loads every face into Compose's shared font cache, off the main thread at process start, so the first screen's
     * text does not read the font files while it draws.
     */
    fun warm() {
        val resolver = androidx.compose.ui.text.font.createFontFamilyResolver(app)
        for (weight in listOf(FontWeight.Normal, FontWeight.Medium, FontWeight.SemiBold)) {
            runCatching { resolver.resolve(sans, weight) }
        }
        for (style in listOf(FontStyle.Normal, FontStyle.Italic)) {
            runCatching { resolver.resolve(serif, FontWeight.Normal, style) }
        }
        for (weight in listOf(FontWeight.Normal, FontWeight.Medium)) {
            runCatching { resolver.resolve(mono, weight) }
        }
    }
}
