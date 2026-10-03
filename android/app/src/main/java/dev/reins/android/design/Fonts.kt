package dev.reins.android.design

import android.content.Context
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight

/** The bundled Geist faces (`assets/fonts`). */
object Fonts {
    private lateinit var app: Context

    fun init(context: Context) {
        app = context.applicationContext
    }

    val sans: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/Geist.ttf", a, FontWeight.Normal),
            Font("fonts/Geist-Medium.ttf", a, FontWeight.Medium),
            Font("fonts/Geist-SemiBold.ttf", a, FontWeight.SemiBold),
            Font("fonts/Geist-Bold.ttf", a, FontWeight.Bold),
        )
    }

    val mono: FontFamily by lazy {
        val a = app.assets
        FontFamily(
            Font("fonts/GeistMono.ttf", a, FontWeight.Normal),
            Font("fonts/GeistMono-Medium.ttf", a, FontWeight.Medium),
            Font("fonts/GeistMono-SemiBold.ttf", a, FontWeight.SemiBold),
        )
    }
}
