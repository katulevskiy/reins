package dev.reins.android.design

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A theme's wallpaper is an image, an effect and a fade; old one-key prefs and baked photos carry over unchanged. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class WallpaperChoiceTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val prefs get() = context.getSharedPreferences("appearance", Context.MODE_PRIVATE)

    @Before
    fun clean() {
        prefs.edit().clear().commit()
        Wallpapers.forget()
        Wallpapers.madeDir(context).deleteRecursively()
        listOf(true, false).forEach {
            Wallpapers.sourceFile(context, it).delete()
            Wallpapers.bakedFile(context, it).delete()
        }
    }

    @After
    fun reset() {
        Appearance.set(context, AppearanceChoice())
        Wallpapers.forget()
    }

    private fun pixels(b: Bitmap) = IntArray(b.width * b.height).also { b.getPixels(it, 0, b.width, 0, 0, b.width, b.height) }

    private fun gradient(w: Int, h: Int): Bitmap {
        val px = IntArray(w * h) { i -> val v = (i % w) * 255 / (w - 1); (0xFF shl 24) or (v shl 16) or (v shl 8) or (255 - v) }
        return Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }

    @Test
    fun nothingKeptGivesTheDefaultsAsTheyAre() {
        val choice = Appearance.load(context)
        assertEquals(WallpaperChoice(Wallpaper.Halo), choice.wallpaperDark)
        assertEquals(WallpaperChoice(Wallpaper.ToriiDither), choice.wallpaperLight)
        assertTrue(choice.wallpaperDark.plain && choice.wallpaperLight.plain)
    }

    @Test
    fun theOldOneKeyWallpaperMovesOnceToTheImageWithNoEffect() {
        prefs.edit().putString("wallpaperDark", "Ruins").putString("wallpaperLight", "Photo").commit()
        val choice = Appearance.load(context)
        assertEquals(WallpaperChoice(Wallpaper.Ruins, PhotoEffect.Original, 0f), choice.wallpaperDark)
        assertEquals(WallpaperChoice(Wallpaper.Photo, PhotoEffect.Original, 0f), choice.wallpaperLight)
        // Read once: the old keys are gone and the new ones say the same.
        assertFalse(prefs.contains("wallpaperDark"))
        assertFalse(prefs.contains("wallpaperLight"))
        assertEquals("Ruins", prefs.getString("wallpaperDarkImage", null))
        assertEquals(choice, Appearance.load(context))
    }

    @Test
    fun imageEffectAndFadeAreKeptApart() {
        val choice = AppearanceChoice(
            wallpaperDark = WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Halftone, 0.35f),
            wallpaperLight = WallpaperChoice(Wallpaper.Photo, PhotoEffect.Scanlines, 0.8f),
        )
        Appearance.set(context, choice)
        assertEquals(choice, Appearance.load(context))
    }

    @Test
    fun aNewPhotoKeepsTheEffectAndFadesWhenTheThemeHadNoFade() {
        // No fade: the photo gets the old default, so it does not end in a hard edge.
        assertEquals(
            WallpaperChoice(Wallpaper.Photo, PhotoEffect.Dither, WallpaperChoice.PHOTO_FADE),
            WallpaperChoice(Wallpaper.Halo, PhotoEffect.Dither, 0f).withNewPhoto(),
        )
        assertEquals(WallpaperChoice(Wallpaper.Photo, PhotoEffect.Original, 0.6f), WallpaperChoice(Wallpaper.Halo).withNewPhoto())
        // A fade below one step counts as none.
        assertEquals(0.6f, WallpaperChoice(Wallpaper.Ruins, fade = 0.004f).withNewPhoto().fade)
        // A fade of its own is kept.
        assertEquals(
            WallpaperChoice(Wallpaper.Photo, PhotoEffect.Halftone, 0.25f),
            WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Halftone, 0.25f).withNewPhoto(),
        )
    }

    @Test
    fun theKeySeparatesImageEffectFadeAndTheme() {
        val base = WallpaperChoice(Wallpaper.Halo, PhotoEffect.Dither, 0.5f)
        val keys = listOf(
            Wallpapers.key(base, true),
            Wallpapers.key(base, false),
            Wallpapers.key(base.copy(image = Wallpaper.Ruins), true),
            Wallpapers.key(base.copy(effect = PhotoEffect.Halftone), true),
            Wallpapers.key(base.copy(fade = 0.6f), true),
            Wallpapers.key(base.copy(image = Wallpaper.Photo), true),
        )
        assertEquals(keys.size, keys.toSet().size)
        // The fade is kept in hundredths: the same step is the same picture.
        assertEquals(Wallpapers.key(base.copy(fade = 0.501f), true), Wallpapers.key(base.copy(fade = 0.499f), true))
        assertEquals("Halo-Dither-50-dark", Wallpapers.key(base, true))
    }

    @Test
    fun aShippedImageAsItIsIsTheAssetUnchanged() {
        for (dark in listOf(true, false)) {
            val choice = WallpaperChoice(Wallpaper.Halo)
            val shown = Wallpapers.bitmap(context, choice, dark)!!
            val asset = context.assets.open("wallpapers/halo.jpg").use { BitmapFactory.decodeStream(it) }
            assertEquals(asset.width, shown.width)
            assertTrue("Halo as it is is the asset", pixels(asset).contentEquals(pixels(shown)))
            assertSame("no picture is made for it", shown, Wallpapers.ready(context, choice, dark))
        }
        assertTrue(Wallpapers.madeDir(context).listFiles().isNullOrEmpty())
    }

    @Test
    fun anEffectOnAShippedImageIsMadeOnceAndKeptUnderItsKey() {
        val choice = WallpaperChoice(Wallpaper.Halo, PhotoEffect.Dither, 0.4f)
        assertEquals(null, Wallpapers.ready(context, choice, true))
        val made = Wallpapers.bitmap(context, choice, true)!!
        assertTrue(Wallpapers.madeFile(context, choice, true).isFile)
        assertSame(made, Wallpapers.ready(context, choice, true))
        assertSame(made, Wallpapers.bitmap(context, choice, true))
        assertFalse(pixels(made).contentEquals(pixels(Wallpapers.bitmap(context, WallpaperChoice(Wallpaper.Halo), true)!!)))
        // Read back from its file once memory lets it go.
        Wallpapers.forget()
        assertTrue(pixels(made).contentEquals(pixels(Wallpapers.bitmap(context, choice, true)!!)))
        // The other theme is its own picture.
        Wallpapers.bitmap(context, choice, false)
        assertTrue(Wallpapers.madeFile(context, choice, false).isFile)
        // The frost follows the made picture, not the asset.
        val frosted = Wallpapers.frosted(choice, true, made)
        assertNotEquals(pixels(frosted).toList(), pixels(Wallpapers.frosted(WallpaperChoice(Wallpaper.Halo), true, Wallpapers.bitmap(context, WallpaperChoice(Wallpaper.Halo), true)!!)).toList())
    }

    @Test
    fun anOldBakedPhotoIsShownAsItIs() {
        Wallpapers.bakedFile(context, true).outputStream().use { gradient(PhotoEffects.WIDTH, PhotoEffects.HEIGHT).compress(Bitmap.CompressFormat.JPEG, 88, it) }
        prefs.edit().putString("wallpaperDark", "Photo").commit()
        val choice = Appearance.load(context).wallpaperDark
        assertEquals(WallpaperChoice(Wallpaper.Photo), choice)
        assertTrue(Wallpapers.hasPhoto(context, true))
        val shown = Wallpapers.bitmap(context, choice, true)!!
        val baked = BitmapFactory.decodeFile(Wallpapers.bakedFile(context, true).absolutePath)
        assertTrue(pixels(baked).contentEquals(pixels(shown)))
        assertTrue(Wallpapers.madeDir(context).listFiles().isNullOrEmpty())
    }

    @Test
    fun aNewPhotoKeepsItsSourceAndDropsWhatWasMadeFromTheOldOne() {
        Wallpapers.bakedFile(context, true).outputStream().use { gradient(PhotoEffects.WIDTH, PhotoEffects.HEIGHT).compress(Bitmap.CompressFormat.JPEG, 88, it) }
        val dithered = WallpaperChoice(Wallpaper.Photo, PhotoEffect.Dither, 0.6f)
        val old = Wallpapers.bitmap(context, dithered, true)!!
        assertTrue(Wallpapers.madeFile(context, dithered, true).isFile)
        val version = Wallpapers.photoVersion

        Wallpapers.savePhoto(context, true, PhotoEffects.fit(gradient(1200, 900)))
        assertTrue(Wallpapers.sourceFile(context, true).isFile)
        assertFalse(Wallpapers.bakedFile(context, true).isFile)
        assertFalse(Wallpapers.madeFile(context, dithered, true).isFile)
        assertNotEquals(version, Wallpapers.photoVersion)
        assertEquals(null, Wallpapers.ready(context, dithered, true))

        // The source is the fitted photo, without any effect; effects are made from it afresh.
        val source = Wallpapers.bitmap(context, WallpaperChoice(Wallpaper.Photo), true)!!
        assertEquals(PhotoEffects.WIDTH, source.width)
        assertEquals(PhotoEffects.HEIGHT, source.height)
        val fresh = Wallpapers.bitmap(context, dithered, true)
        assertNotNull(fresh)
        assertFalse(pixels(old).contentEquals(pixels(fresh!!)))
        // The light theme has no photo of its own.
        assertFalse(Wallpapers.hasPhoto(context, false))
    }
}
