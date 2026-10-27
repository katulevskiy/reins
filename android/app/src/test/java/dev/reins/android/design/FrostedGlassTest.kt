package dev.reins.android.design

import android.graphics.Bitmap
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlin.math.abs
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The panes' glass over the wallpaper: it fades into the paper at the art's foot, with no edge, in every piece alike. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class FrostedGlassTest {
    @Before
    fun clean() = Wallpapers.forget()

    @After
    fun reset() = Wallpapers.forget()

    /** The art's size: black with a white stripe a quarter of its width down the middle, frosted for [dark]. */
    private fun frosted(dark: Boolean): ImageBitmap {
        val w = 780
        val h = 1000
        val px = IntArray(w * h) { i -> if (i % w in 292 until 488) -1 else 0xFF000000.toInt() }
        val src = Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
        return Wallpapers.frosted(WallpaperChoice(Wallpaper.Photo), dark, src).asImageBitmap()
    }

    /** One piece of glass, [w] × [h], with the art at [art] in its coordinates. */
    private fun glass(image: ImageBitmap, w: Int, h: Int, art: Rect, paper: Color, scrim: Color = Color.Transparent): IntArray {
        val target = ImageBitmap(w, h)
        CanvasDrawScope().draw(Density(1f), LayoutDirection.Ltr, Canvas(target), Size(w.toFloat(), h.toFloat())) {
            drawFrostedArt(image, art, paper, scrim, 60f)
        }
        return IntArray(w * h).also { target.readPixels(it) }
    }

    private fun distance(a: Int, b: Int): Int =
        maxOf(abs((a shr 16 and 0xFF) - (b shr 16 and 0xFF)), abs((a shr 8 and 0xFF) - (b shr 8 and 0xFF)), abs((a and 0xFF) - (b and 0xFF)))

    @Test
    fun theCopysLastRowsArePaperAndBelowTheArtIsPaper() {
        for (dark in listOf(true, false)) {
            val paper = PhotoEffects.paper(dark)
            val p = paper.toArgb()
            // A pane 400 px tall, the art's foot 200 px down it (the art 300 px tall, its top above the pane).
            val w = 120
            val px = glass(frosted(dark), w, 400, Rect(0f, -100f, w.toFloat(), 200f), paper)
            fun row(y: Int) = (0 until w).map { px[y * w + it] }
            for (y in 197..199) row(y).forEach { assertTrue("dark $dark, row $y: the copy's last rows are paper", distance(it, p) <= 2) }
            for (y in 200 until 400) row(y).forEach { assertEquals("dark $dark, row $y: below the art is paper", p, it) }
            // Above the fade (it starts 30 % of 300 above the foot, at 110) the art shows; halfway down it is between.
            val above = row(60).maxOf { distance(it, p) }
            val half = row(155).maxOf { distance(it, p) }
            assertTrue("dark $dark: the art shows above the fade ($above)", above > 120)
            assertTrue("dark $dark: halfway down the fade ($half of $above)", half in (above / 4)..(above * 3 / 4))
        }
    }

    @Test
    fun thePiecesOfAPaneLineUp() {
        val paper = PhotoEffects.paper(true)
        val image = frosted(true)
        val w = 120
        val scrim = Color.Black.copy(alpha = 0.55f)
        val whole = glass(image, w, 400, Rect(0f, -100f, w.toFloat(), 200f), paper, scrim)
        // Two rows of a lazy list: the same art, each offset by where the row is.
        val top = glass(image, w, 160, Rect(0f, -100f, w.toFloat(), 200f), paper, scrim)
        val bottom = glass(image, w, 240, Rect(0f, -260f, w.toFloat(), 40f), paper, scrim)
        val joined = top + bottom
        for (i in whole.indices) assertTrue("pixel ${i % w}, ${i / w}", distance(whole[i], joined[i]) <= 1)
    }

    @Test
    fun theScrimDarkensTheCopysTopOnly() {
        val paper = PhotoEffects.paper(true)
        val image = frosted(true)
        val w = 120
        val art = Rect(0f, 0f, w.toFloat(), 300f)
        val plain = glass(image, w, 300, art, paper)
        val shaded = glass(image, w, 300, art, paper, Color.Black.copy(alpha = 0.55f))
        val mid = w / 2
        assertTrue("under the title it is shaded", (shaded[mid] shr 16 and 0xFF) < (plain[mid] shr 16 and 0xFF) * 0.6)
        for (y in 60 until 300) assertEquals("past the scrim, row $y", plain[y * w + mid], shaded[y * w + mid])
    }
}
