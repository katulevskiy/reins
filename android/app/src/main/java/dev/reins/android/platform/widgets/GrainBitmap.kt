package dev.reins.android.platform.widgets

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import kotlin.random.Random

/**
 * The field's grain for a widget, pre-baked into a bitmap: widgets are RemoteViews, so no shader runs there. The fill,
 * then a fixed scatter of light and dark specks (the same seed every time, so it never shimmers between updates).
 * Small and tiled by the launcher's scaling; cached per colour.
 */
object GrainBitmap {
    private const val SIZE = 160
    private val cache = HashMap<Int, Bitmap>()

    fun of(color: Int): Bitmap = synchronized(cache) {
        cache.getOrPut(color) {
            val bitmap = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bitmap)
            canvas.drawColor(color)
            val random = Random(SEED)
            val paint = Paint()
            repeat(SIZE * SIZE / 9) {
                val light = random.nextBoolean()
                paint.color = if (light) 0x14FFFFFF else 0x14000000
                val x = random.nextInt(SIZE).toFloat()
                val y = random.nextInt(SIZE).toFloat()
                canvas.drawRect(x, y, x + 1f, y + 1f, paint)
            }
            bitmap
        }
    }

    private const val SEED = 0x5E1A
}
