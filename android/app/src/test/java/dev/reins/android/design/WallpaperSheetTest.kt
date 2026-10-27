package dev.reins.android.design

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.nio.ByteBuffer
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * For design review, not a check: `-Dreins.wallpaperSheet=/dir` draws every shipped image with every effect, in both
 * themes, at full size, into `/dir/android/<image>-<effect>-<theme>.png`, and keeps each image's decoded shipped
 * pixels in `/dir/input/<image>.rgba` (RGBA, 780 × 1000) so the iPhone's pixel code can draw the same.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class WallpaperSheetTest {
    @Test
    fun drawTheSheet() {
        val dir = System.getProperty("reins.wallpaperSheet")
        assumeTrue(dir != null)
        val context: Context = ApplicationProvider.getApplicationContext()
        val out = File(dir!!, "android").apply { mkdirs() }
        val input = File(dir, "input").apply { mkdirs() }
        for (image in Wallpaper.shipped) {
            val src = context.assets.open(image.assetPath()!!).use { BitmapFactory.decodeStream(it) }.copy(Bitmap.Config.ARGB_8888, false)
            val px = IntArray(src.width * src.height).also { src.getPixels(it, 0, src.width, 0, 0, src.width, src.height) }
            val bytes = ByteBuffer.allocate(px.size * 4)
            for (c in px) bytes.put(((c shr 16) and 0xFF).toByte()).put(((c shr 8) and 0xFF).toByte()).put((c and 0xFF).toByte()).put(0xFF.toByte())
            File(input, "${image.name}.rgba").writeBytes(bytes.array())
            for (effect in PhotoEffect.entries) {
                for (dark in listOf(true, false)) {
                    // As the app draws it: a treated image's own effect is its shipped file, any other is drawn on
                    // its untreated original.
                    val choice = WallpaperChoice(image, effect)
                    val base = Wallpapers.base(context, choice, dark)!!.copy(Bitmap.Config.ARGB_8888, false)
                    val made = PhotoEffects.apply(base, choice.applied, 0f, dark, image.foot)
                    File(out, "${image.name}-${effect.name}-${if (dark) "dark" else "light"}.png").outputStream().use { made.compress(Bitmap.CompressFormat.PNG, 100, it) }
                }
            }
        }
    }
}
