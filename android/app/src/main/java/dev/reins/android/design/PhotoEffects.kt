package dev.reins.android.design

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sqrt

/**
 * The looks a wallpaper can take behind Activity (Settings › Wallpaper): any image, shipped or the owner's own (a
 * treated image's effects are drawn on its untreated original). Dither, Scanlines and Halftone keep the image's colours;
 * Mono is the boards' two-tone dither in the theme's light and dark.
 */
enum class PhotoEffect(val label: String) {
    /** The image as it is. */
    Original("Original"),
    Dither("Dither"),
    Mono("Mono"),
    Scanlines("Scanlines"),
    Halftone("Halftone"),
    ;

    companion object {
        fun named(name: String?): PhotoEffect? = entries.firstOrNull { it.name == name }
    }
}

/**
 * Turning an image into a wallpaper on this phone: a photo cropped to the shipped art's shape (780 × 1000, from its
 * top), given an effect, and faded into the paper at its foot ([fade] 0 none … 1 from 30 % of the way down). Pure
 * functions on pixels; nothing leaves the phone.
 *
 * The effects are the Next boards' (design/next, `source/art/fx.py`), and the iPhone draws them with the same steps
 * and numbers (`WallpaperEffects.swift`), so a wallpaper looks the same on both. Sizes are for the 780-wide art and
 * scale with the width, so a small preview looks like the wallpaper.
 */
object PhotoEffects {
    const val WIDTH = 780
    const val HEIGHT = 1000

    /** The page a theme's wallpaper fades into. */
    fun paper(dark: Boolean): Color = (if (dark) RColors.Dark else RColors.Light).paper

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

    /**
     * Where a shipped image already fades into the page, as a share of its height from the top: from [start] to [end]
     * (the boards' `fade(start, end)`). An effect fades back into the image over it, so the foot stays smooth.
     */
    class Foot(val start: Double, val end: Double)

    /**
     * The share of the height at the bottom that an effect fades into the paper on an image with no [Foot] of its own
     * (the owner's photo), whatever the added fade: an effect can turn the bottom pure black, so the art never ends on
     * a hard line. As on the iPhone (`WallpaperRender.effectFoot`).
     */
    const val EFFECT_FOOT = 0.2f

    /** Where the fade starts, as a share of the height from the top: later with less fade, none at 0 (as the iPhone's). */
    fun fadeStart(fade: Float): Float = 1f - 0.7f * fade.coerceIn(0f, 1f)

    /** [src] with [effect] and [fade], for a theme; [foot]: where [src] already fades into the page. */
    fun apply(src: Bitmap, effect: PhotoEffect, fade: Float, dark: Boolean, foot: Foot? = null): Bitmap {
        val w = src.width
        val h = src.height
        val px = IntArray(w * h)
        src.getPixels(px, 0, w, 0, 0, w, h)
        val plain = if (foot != null && effect != PhotoEffect.Original) px.copyOf() else null
        render(px, w, h, effect, dark)
        if (plain != null) footOut(px, plain, w, h, foot!!)
        // Original keeps the bottom as chosen; a shipped image has its own foot.
        val minimumFoot = if (effect == PhotoEffect.Original || foot != null) 0f else EFFECT_FOOT
        fadeOut(px, w, h, fade, paper(dark).toArgb(), minimumFoot)
        return Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }

    /** Blends [px] back into [plain] over the [foot], eased, so the effect ends where the image fades out. */
    internal fun footOut(px: IntArray, plain: IntArray, w: Int, h: Int, foot: Foot) {
        for (y in 0 until h) {
            val t = ((y.toDouble() / max(1, h - 1) - foot.start) / (foot.end - foot.start)).coerceIn(0.0, 1.0)
            if (t <= 0.0) continue
            val k = t * t * (3 - 2 * t)
            for (x in 0 until w) {
                val a = px[y * w + x]
                val b = plain[y * w + x]
                fun ch(s: Int) = floor(((a shr s) and 0xFF) * (1 - k) + ((b shr s) and 0xFF) * k + 0.5).toInt()
                px[y * w + x] = argb(ch(16), ch(8), ch(0))
            }
        }
    }

    /** [effect] on opaque ARGB pixels, in place. */
    fun render(px: IntArray, w: Int, h: Int, effect: PhotoEffect, dark: Boolean) {
        when (effect) {
            PhotoEffect.Original -> Unit
            PhotoEffect.Dither -> dither(px, w, h)
            PhotoEffect.Mono -> mono(px, w, h, dark)
            PhotoEffect.Scanlines -> scanlines(px, w, h)
            PhotoEffect.Halftone -> halftone(px, w, h, dark)
        }
    }

    /**
     * Fades into [paper] over the lower part, eased, from [fadeStart] down (and at least the bottom [minimumFoot]); the
     * more fade, the higher it starts.
     */
    private fun fadeOut(px: IntArray, w: Int, h: Int, fade: Float, paper: Int, minimumFoot: Float) {
        val start = minOf(fadeStart(fade), 1f - minimumFoot.coerceIn(0f, 1f))
        if (start >= 1f) return
        for (y in 0 until h) {
            val t = ((y.toFloat() / h - start) / (1f - start)).coerceIn(0f, 1f)
            if (t <= 0f) continue
            val k = t * t * (3 - 2 * t)
            for (x in 0 until w) px[y * w + x] = mix(px[y * w + x], paper, k)
        }
    }

    // ---- The effects ------------------------------------------------------------------------------------------------

    /** The boards' 8 × 8 Bayer matrix, in 64ths. */
    private val BAYER = intArrayOf(
        0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22,
        3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
    )

    private fun threshold(col: Int, row: Int): Double = BAYER[(row % 8) * 8 + col % 8] / 64.0

    /** A size on the 780-wide art, at this width: rounded, at least [least]. */
    private fun scaled(n: Double, w: Int, least: Int = 1): Int = max(least, floor(n * w / WIDTH + 0.5).toInt())

    /** The image in square cells of [size] px: each cell's mean colour, 0…255 a channel (cells at the edges are cut). */
    private class Cells(val size: Int, val cols: Int, val rows: Int, val r: DoubleArray, val g: DoubleArray, val b: DoubleArray) {
        /** Rec. 601 luma, 0…1, as the boards' greyscale. */
        fun luma(i: Int): Double = (0.299 * r[i] + 0.587 * g[i] + 0.114 * b[i]) / 255.0
    }

    private fun cells(px: IntArray, w: Int, h: Int, size: Int): Cells {
        val cols = (w + size - 1) / size
        val rows = (h + size - 1) / size
        val r = DoubleArray(cols * rows)
        val g = DoubleArray(cols * rows)
        val b = DoubleArray(cols * rows)
        val n = IntArray(cols * rows)
        for (y in 0 until h) {
            val row = (y / size) * cols
            for (x in 0 until w) {
                val c = px[y * w + x]
                val i = row + x / size
                r[i] += ((c shr 16) and 0xFF).toDouble()
                g[i] += ((c shr 8) and 0xFF).toDouble()
                b[i] += (c and 0xFF).toDouble()
                n[i]++
            }
        }
        for (i in n.indices) {
            r[i] /= n[i]
            g[i] /= n[i]
            b[i] /= n[i]
        }
        return Cells(size, cols, rows, r, g, b)
    }

    /** Paints each cell its colour from [colour] (by cell index). */
    private fun paint(px: IntArray, w: Int, h: Int, cells: Cells, colour: (Int) -> Int) {
        val out = IntArray(cells.cols * cells.rows) { colour(it) }
        for (y in 0 until h) {
            val row = (y / cells.size) * cells.cols
            for (x in 0 until w) px[y * w + x] = out[row + x / cells.size]
        }
    }

    /**
     * Ordered dither in colour (the boards' Ruins): 3 px cells, each channel stretched to its 1st…99th percentile,
     * lifted 10 %, then dithered on its own to three levels (0, ½, 1) by the Bayer matrix: 27 colours.
     */
    private fun dither(px: IntArray, w: Int, h: Int) {
        val cells = cells(px, w, h, scaled(3.0, w))
        for (ch in listOf(cells.r, cells.g, cells.b)) autocontrast(ch)
        fun level(v: Double, t: Double): Int {
            val q = min(1.0, floor((v * 1.1 / 255.0).coerceIn(0.0, 1.0) * 2 + t) / 2)
            return floor(q * 255 + 0.5).toInt()
        }
        paint(px, w, h, cells) { i ->
            val t = threshold(i % cells.cols, i / cells.cols)
            argb(level(cells.r[i], t), level(cells.g[i], t), level(cells.b[i], t))
        }
    }

    /** Stretches a channel so its darkest and lightest 1 % reach the ends (PIL's `autocontrast(cutoff=1)`). */
    private fun autocontrast(ch: DoubleArray) {
        val hist = IntArray(256)
        for (v in ch) hist[floor(v + 0.5).toInt().coerceIn(0, 255)]++
        val cut = ch.size * 0.01
        var lo = 0
        var sum = 0
        while (lo < 255) {
            sum += hist[lo]
            if (sum > cut) break
            lo++
        }
        var hi = 255
        sum = 0
        while (hi > 0) {
            sum += hist[hi]
            if (sum > cut) break
            hi--
        }
        if (hi <= lo) return
        for (i in ch.indices) ch[i] = (ch[i] - lo) * 255.0 / (hi - lo)
    }

    /**
     * The boards' two-tone dither (Halo, Torii dither): luma with more contrast about its mean, dithered by the Bayer
     * matrix to the theme's light and dark. Dark: 3 px cells, 1.2 contrast, 0.9 gamma, blue-white on black. Light:
     * 2 px cells, 1.4 contrast, the paper on a slate grey.
     */
    private fun mono(px: IntArray, w: Int, h: Int, dark: Boolean) {
        val cells = cells(px, w, h, if (dark) scaled(3.0, w) else scaled(2.0, w))
        val contrast = if (dark) 1.2 else 1.4
        val gamma = if (dark) 0.9 else 1.0
        val on = if (dark) argb(232, 236, 255) else argb(246, 246, 248)
        val off = if (dark) argb(0, 0, 0) else argb(70, 70, 78)
        val n = cells.cols * cells.rows
        val mean = (0 until n).sumOf { cells.luma(it) } / n
        paint(px, w, h, cells) { i ->
            val g = (mean + (cells.luma(i) - mean) * contrast).coerceIn(0.0, 1.0).pow(gamma)
            if (g > threshold(i % cells.cols, i / cells.cols)) on else off
        }
    }

    /**
     * CRT lines over the image in its colours (the boards' Eyes): every 3rd row of 3 as it is, the other two at 60 %,
     * red moved 2 px right and blue 2 px left.
     */
    private fun scanlines(px: IntArray, w: Int, h: Int) {
        val period = scaled(3.0, w, 2)
        val lit = max(1, period / 2)
        val shift = floor(2.0 * w / WIDTH + 0.5).toInt()
        val row = IntArray(w)
        for (y in 0 until h) {
            System.arraycopy(px, y * w, row, 0, w)
            val m = if (y % period < lit) 1.0 else 0.6
            for (x in 0 until w) {
                val r = (row[(x - shift).coerceIn(0, w - 1)] shr 16) and 0xFF
                val g = (row[x] shr 8) and 0xFF
                val b = row[(x + shift).coerceIn(0, w - 1)] and 0xFF
                px[y * w + x] = argb(floor(r * m + 0.5).toInt(), floor(g * m + 0.5).toInt(), floor(b * m + 0.5).toInt())
            }
        }
    }

    /**
     * Round dots on 9 px cells, on the theme's paper, in the image's colours (the boards' Figure, coloured). A dot is
     * as big as its cell is light (dark theme) or dark (light theme), after 1.4 contrast about the mean; in the dark
     * its colour is the cell's brought up to full brightness, in the light the cell's a little deeper.
     */
    private fun halftone(px: IntArray, w: Int, h: Int, dark: Boolean) {
        val cells = cells(px, w, h, scaled(9.0, w, 3))
        val size = cells.size
        val n = cells.cols * cells.rows
        val mean = (0 until n).sumOf { cells.luma(it) } / n
        px.fill(paper(dark).toArgb())
        for (i in 0 until n) {
            val g = (mean + (cells.luma(i) - mean) * 1.4).coerceIn(0.0, 1.0)
            val amount = if (dark) g else 1 - g
            val radius = size * 0.52 * amount.pow(0.8)
            if (radius <= 0.4) continue
            val r = cells.r[i] / 255.0
            val gr = cells.g[i] / 255.0
            val b = cells.b[i] / 255.0
            val k = if (dark) 1.0 / max(max(r, max(gr, b)), 0.25) else 0.85
            val dot = argb(floor(min(1.0, r * k) * 255 + 0.5).toInt(), floor(min(1.0, gr * k) * 255 + 0.5).toInt(), floor(min(1.0, b * k) * 255 + 0.5).toInt())
            disc(px, w, h, (i % cells.cols) * size + size / 2.0, (i / cells.cols) * size + size / 2.0, radius, dot)
        }
    }

    /** A filled disc, its edge softened over a pixel. */
    private fun disc(px: IntArray, w: Int, h: Int, cx: Double, cy: Double, r: Double, colour: Int) {
        val x0 = max(0, (cx - r - 1).toInt())
        val x1 = min(w - 1, (cx + r + 1).toInt())
        val y0 = max(0, (cy - r - 1).toInt())
        val y1 = min(h - 1, (cy + r + 1).toInt())
        for (y in y0..y1) {
            for (x in x0..x1) {
                val dx = x + 0.5 - cx
                val dy = y + 0.5 - cy
                val cover = (r - sqrt(dx * dx + dy * dy) + 0.5).coerceIn(0.0, 1.0)
                if (cover > 0) px[y * w + x] = mix(px[y * w + x], colour, cover.toFloat())
            }
        }
    }

    private fun argb(r: Int, g: Int, b: Int): Int = (0xFF shl 24) or (r.coerceIn(0, 255) shl 16) or (g.coerceIn(0, 255) shl 8) or b.coerceIn(0, 255)

    private fun mix(a: Int, b: Int, t: Float): Int {
        fun ch(s: Int) = (((a shr s) and 0xFF) * (1 - t) + ((b shr s) and 0xFF) * t).roundToInt().coerceIn(0, 255)
        return (0xFF shl 24) or (ch(16) shl 16) or (ch(8) shl 8) or ch(0)
    }
}
