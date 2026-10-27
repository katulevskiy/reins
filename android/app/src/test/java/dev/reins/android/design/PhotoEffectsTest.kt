package dev.reins.android.design

import android.graphics.Bitmap
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The owner's photo becomes a wallpaper of the shipped shape, its effect in the page's two colours, faded at its foot. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class PhotoEffectsTest {
    private val paper = Color(0xFF121215)
    private val ink = Color(0xFFE6E8FF)

    private fun gradient(w: Int, h: Int): Bitmap {
        val px = IntArray(w * h) { i -> val v = (i % w) * 255 / (w - 1); (0xFF shl 24) or (v shl 16) or (v shl 8) or v }
        return Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }

    @Test
    fun aPhotoIsCroppedToTheArtsShape() {
        val fitted = PhotoEffects.fit(gradient(1200, 900))
        assertEquals(PhotoEffects.WIDTH, fitted.width)
        assertEquals(PhotoEffects.HEIGHT, fitted.height)
    }

    @Test
    fun ditherAndHalftoneUseOnlyThePagesTwoColours() {
        val src = PhotoEffects.small(gradient(400, 500), 200)
        for (effect in listOf(PhotoEffect.Dither, PhotoEffect.Halftone)) {
            val out = PhotoEffects.apply(src, effect, 0f, paper, ink)
            val px = IntArray(out.width * out.height).also { out.getPixels(it, 0, out.width, 0, 0, out.width, out.height) }
            assertTrue("$effect: only paper and ink", px.all { it == paper.toArgb() || it == ink.toArgb() })
            assertTrue("$effect: both appear", px.any { it == paper.toArgb() } && px.any { it == ink.toArgb() })
        }
    }

    @Test
    fun theFadeEndsInThePaper() {
        val src = PhotoEffects.small(gradient(400, 500), 200)
        val out = PhotoEffects.apply(src, PhotoEffect.Original, 1f, paper, ink)
        // The last row is the paper; the first is untouched.
        assertEquals(paper.toArgb(), out.getPixel(out.width - 1, out.height - 1))
        assertEquals(src.getPixel(out.width - 1, 0), out.getPixel(out.width - 1, 0))
    }
}
