package dev.reins.android.ui.settings

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import kotlin.math.roundToInt
import kotlin.math.sqrt

/** The looks a photo can take behind Activity (Settings › Wallpaper › Your photo). */
enum class PhotoEffect(val label: String) {
    Photo("Photo"),
    Dither("Dither"),
    Scanlines("Scanlines"),
    Halftone("Halftone"),
    Colour("Colour"),
}

/**
 * Turning a photo into a wallpaper on this phone: cropped to the shipped art's shape (780 × 1000, from its top), given
 * an effect drawn in the page's ink on its paper, and faded into the paper at its foot ([fade] 0 none … 1 from a third
 * of the way down). Pure functions on bitmaps; nothing leaves the phone.
 */
object PhotoEffects {
    const val WIDTH = 780
    const val HEIGHT = 1000

    /** [src] covered into 780 × 1000, cropped from the top centre. */
    fun fit(src: Bitmap): Bitmap {
        val scale = maxOf(WIDTH.toFloat() / src.width, HEIGHT.toFloat() / src.height)
        val w = (WIDTH / scale).roundToInt().coerceAtMost(src.width)
        val h = (HEIGHT / scale).roundToInt().coerceAtMost(src.height)
        val left = (src.width - w) / 2
        val out = Bitmap.createBitmap(WIDTH, HEIGHT, Bitmap.Config.ARGB_8888)
        Canvas(out).drawBitmap(src, Rect(left, 0, left + w, h), Rect(0, 0, WIDTH, HEIGHT), Paint(Paint.FILTER_BITMAP_FLAG))
        return out
    }

    /** A smaller copy for previews ([width] wide). */
    fun small(src: Bitmap, width: Int = 390): Bitmap = Bitmap.createScaledBitmap(src, width, (width * src.height.toFloat() / src.width).roundToInt().coerceAtLeast(1), true)

    private val BAYER = intArrayOf(0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5)

    fun apply(src: Bitmap, effect: PhotoEffect, fade: Float, paper: Color, ink: Color): Bitmap {
        val w = src.width
        val h = src.height
        val px = IntArray(w * h)
        src.getPixels(px, 0, w, 0, 0, w, h)
        val p = paper.toArgb()
        val k = ink.toArgb()
        val cell = (w / 195).coerceAtLeast(1) // the boards' dither dots: about 2 px on a 390-wide page
        val dot = (w / 65).coerceAtLeast(3) // halftone cells
        for (y in 0 until h) {
            for (x in 0 until w) {
                val i = y * w + x
                val c = px[i]
                val r = (c shr 16) and 0xFF
                val g = (c shr 8) and 0xFF
                val b = c and 0xFF
                val lum = (0.2126f * r + 0.7152f * g + 0.0722f * b) / 255f
                // How much ink: light pixels are ink on a dark page, dark pixels on a light one.
                val darkPage = luminance(p) < 0.5f
                val amount = if (darkPage) lum else 1f - lum
                px[i] = when (effect) {
                    PhotoEffect.Photo -> c
                    PhotoEffect.Colour -> saturate(c, 1.25f)
                    PhotoEffect.Dither -> {
                        val t = (BAYER[((y / cell) % 4) * 4 + (x / cell) % 4] + 0.5f) / 16f
                        if (amount > t) k else p
                    }
                    PhotoEffect.Scanlines -> if ((y / cell) % 3 == 2) p else mix(p, k, amount)
                    PhotoEffect.Halftone -> {
                        val cx = (x / dot) * dot + dot / 2f
                        val cy = (y / dot) * dot + dot / 2f
                        val d = sqrt((x - cx) * (x - cx) + (y - cy) * (y - cy)) / (dot / 2f)
                        if (d < sqrt(amount.coerceIn(0f, 1f)) * 1.15f) k else p
                    }
                }
                if (fade > 0f) {
                    // Fades into the paper over the lower part; the more fade, the higher it starts.
                    val start = 1f - fade * 0.66f
                    val t = ((y.toFloat() / h - start) / (1f - start)).coerceIn(0f, 1f)
                    if (t > 0f) px[i] = mix(px[i], p, t * t * (3 - 2 * t))
                }
            }
        }
        return Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }

    private fun luminance(c: Int): Float = (0.2126f * ((c shr 16) and 0xFF) + 0.7152f * ((c shr 8) and 0xFF) + 0.0722f * (c and 0xFF)) / 255f

    private fun mix(a: Int, b: Int, t: Float): Int {
        fun ch(s: Int) = (((a shr s) and 0xFF) * (1 - t) + ((b shr s) and 0xFF) * t).roundToInt().coerceIn(0, 255)
        return (0xFF shl 24) or (ch(16) shl 16) or (ch(8) shl 8) or ch(0)
    }

    private fun saturate(c: Int, s: Float): Int {
        val r = (c shr 16) and 0xFF
        val g = (c shr 8) and 0xFF
        val b = c and 0xFF
        val l = 0.2126f * r + 0.7152f * g + 0.0722f * b
        fun ch(v: Int) = (l + (v - l) * s).roundToInt().coerceIn(0, 255)
        return (0xFF shl 24) or (ch(r) shl 16) or (ch(g) shl 8) or ch(b)
    }
}
