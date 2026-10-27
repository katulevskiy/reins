package dev.reins.android.design

import android.graphics.Bitmap
import androidx.compose.ui.graphics.toArgb
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The owner's photo becomes a wallpaper of the shipped shape, faded at its foot. Dither, Scanlines and Halftone keep
 * its colours; Mono draws it in the theme's two tones.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class PhotoEffectsTest {
    private fun gradient(w: Int, h: Int): Bitmap {
        val px = IntArray(w * h) { i -> val v = (i % w) * 255 / (w - 1); (0xFF shl 24) or (v shl 16) or (v shl 8) or v }
        return Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }

    /** Hues across, light to dark down: red, yellow, green, cyan, blue, magenta. */
    private fun colourful(w: Int, h: Int): Bitmap {
        val px = IntArray(w * h) { i ->
            val x = i % w
            val y = i / w
            android.graphics.Color.HSVToColor(floatArrayOf(360f * x / w, 0.85f, 1f - 0.7f * y / h))
        }
        return Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }

    private fun pixels(b: Bitmap) = IntArray(b.width * b.height).also { b.getPixels(it, 0, b.width, 0, 0, b.width, b.height) }

    /** The picture above the bottom an effect fades into the paper ([PhotoEffects.EFFECT_FOOT]). */
    private fun aboveFoot(b: Bitmap): IntArray {
        val rows = (b.height * (1 - PhotoEffects.EFFECT_FOOT)).toInt()
        return IntArray(b.width * rows).also { b.getPixels(it, 0, b.width, 0, 0, b.width, rows) }
    }

    private fun r(c: Int) = (c shr 16) and 0xFF
    private fun g(c: Int) = (c shr 8) and 0xFF
    private fun b(c: Int) = c and 0xFF

    /** How much of a picture is in colour: the share of pixels whose channels differ by more than 40. */
    private fun coloured(px: IntArray): Double = px.count { maxOf(r(it), g(it), b(it)) - minOf(r(it), g(it), b(it)) > 40 }.toDouble() / px.size

    @Test
    fun aPhotoIsCroppedToTheArtsShape() {
        val fitted = PhotoEffects.fit(gradient(1200, 900))
        assertEquals(PhotoEffects.WIDTH, fitted.width)
        assertEquals(PhotoEffects.HEIGHT, fitted.height)
    }

    @Test
    fun ditherScanlinesAndHalftoneKeepTheColours() {
        val src = colourful(390, 500)
        assertTrue(coloured(pixels(src)) > 0.9)
        for (dark in listOf(true, false)) {
            for (effect in listOf(PhotoEffect.Dither, PhotoEffect.Scanlines, PhotoEffect.Halftone)) {
                val out = pixels(PhotoEffects.apply(src, effect, 0f, dark))
                // Halftone leaves paper between its dots; the rest is in colour.
                val least = if (effect == PhotoEffect.Halftone) 0.25 else 0.6
                assertTrue("$effect (dark $dark) is not greyscale: ${coloured(out)}", coloured(out) > least)
            }
        }
    }

    @Test
    fun theColoursStayWhereTheyWere() {
        // The red left edge stays red and the blue part stays blue, in every colour effect (halftone's dots, not
        // the paper between them).
        val src = colourful(390, 500)
        val paper = PhotoEffects.paper(true).toArgb()
        for (effect in listOf(PhotoEffect.Dither, PhotoEffect.Scanlines, PhotoEffect.Halftone)) {
            val out = PhotoEffects.apply(src, effect, 0f, true)
            fun mean(x0: Int, x1: Int, ch: (Int) -> Int): Double {
                var sum = 0.0
                var n = 0
                for (y in 20 until 200) for (x in x0 until x1) {
                    val c = out.getPixel(x, y)
                    if (c != paper) { sum += ch(c); n++ }
                }
                return sum / n
            }
            assertTrue("$effect: red stays red", mean(4, 30, ::r) > mean(4, 30, ::b) + 40)
            // 240° of 360: blue, at about two thirds across.
            assertTrue("$effect: blue stays blue", mean(250, 270, ::b) > mean(250, 270, ::r) + 40)
        }
    }

    @Test
    fun ditherUsesThreeLevelsAChannel() {
        val out = aboveFoot(PhotoEffects.apply(colourful(390, 500), PhotoEffect.Dither, 0f, true))
        val levels = out.flatMap { listOf(r(it), g(it), b(it)) }.toSet()
        assertEquals(setOf(0, 128, 255), levels)
    }

    @Test
    fun monoUsesTheThemesTwoTones() {
        val src = PhotoEffects.small(gradient(400, 500), 200)
        for ((dark, tones) in listOf(true to setOf(0xFFE8ECFF.toInt(), 0xFF000000.toInt()), false to setOf(0xFFF6F6F8.toInt(), 0xFF46464E.toInt()))) {
            val out = aboveFoot(PhotoEffects.apply(src, PhotoEffect.Mono, 0f, dark)).toSet()
            assertEquals("dark $dark", tones, out)
        }
    }

    @Test
    fun halftoneDrawsOnThePaper() {
        // A black photo: no dots in the dark, the paper all over.
        val black = Bitmap.createBitmap(IntArray(200 * 250) { 0xFF000000.toInt() }, 200, 250, Bitmap.Config.ARGB_8888)
        val out = aboveFoot(PhotoEffects.apply(black, PhotoEffect.Halftone, 0f, true)).toSet()
        assertEquals(setOf(PhotoEffects.paper(true).toArgb()), out)
    }

    @Test
    fun anEffectOnAnImageWithoutAFootStillEndsOnThePaper() {
        // A light photo (no foot of its own, as the owner's): at no added fade every effect still ends on the paper.
        val src = PhotoEffects.small(gradient(400, 500), 200)
        for (dark in listOf(true, false)) {
            val paper = PhotoEffects.paper(dark).toArgb()
            for (effect in PhotoEffect.entries.filter { it != PhotoEffect.Original }) {
                val bottom = PhotoEffects.apply(src, effect, 0f, dark).getPixel(100, src.height - 1)
                for (s in listOf(16, 8, 0)) {
                    assertTrue("$effect (dark $dark) ends on the paper, not a hard line", kotlin.math.abs(((bottom shr s) and 0xFF) - ((paper shr s) and 0xFF)) <= 3)
                }
            }
            // Original keeps the bottom as chosen: the photo's own last row.
            assertEquals(src.getPixel(199, src.height - 1), PhotoEffects.apply(src, PhotoEffect.Original, 0f, dark).getPixel(199, src.height - 1))
        }
        // A shipped image's own foot stands in for it: nothing more is faded above it.
        val foot = PhotoEffects.Foot(0.9, 0.99)
        val withFoot = PhotoEffects.apply(src, PhotoEffect.Scanlines, 0f, true, foot)
        val noFoot = PhotoEffects.apply(src, PhotoEffect.Scanlines, 0f, true)
        val row = (src.height * 0.85).toInt()
        assertTrue("the minimum foot fades only a footless image", withFoot.getPixel(199, row) != noFoot.getPixel(199, row))
    }

    @Test
    fun theFadeEndsInThePaper() {
        val src = PhotoEffects.small(gradient(400, 500), 200)
        for (dark in listOf(true, false)) {
            val out = PhotoEffects.apply(src, PhotoEffect.Original, 1f, dark)
            // The last row is the paper; the first is untouched.
            assertEquals(PhotoEffects.paper(dark).toArgb(), out.getPixel(out.width - 1, out.height - 1))
            assertEquals(src.getPixel(out.width - 1, 0), out.getPixel(out.width - 1, 0))
        }
    }
}
