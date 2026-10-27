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

/**
 * A theme's wallpaper is an image, an effect and a fade. Every image takes every effect; picking an image gives it its
 * own effect; old one-key prefs, choices kept before the images came as originals (or before every image took effects),
 * and baked photos carry over looking as they did.
 */
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
    fun nothingKeptGivesTheDefaults() {
        val choice = Appearance.load(context)
        // Dark: Halo as it ships (its own effect, Mono, is in the file).
        assertEquals(WallpaperChoice(Wallpaper.Halo, PhotoEffect.Mono), choice.wallpaperDark)
        assertTrue(choice.wallpaperDark.plain)
        // Light: Torii with the boards' two-tone dither, the old Torii dither.
        assertEquals(WallpaperChoice(Wallpaper.Torii, PhotoEffect.Mono), choice.wallpaperLight)
    }

    @Test
    fun theOldOneKeyWallpaperMovesOnceLookingAsItDid() {
        prefs.edit().putString("wallpaperDark", "Ruins").putString("wallpaperLight", "Photo").commit()
        val choice = Appearance.load(context)
        assertEquals(WallpaperChoice(Wallpaper.Ruins, PhotoEffect.Dither, 0f), choice.wallpaperDark)
        assertEquals(WallpaperChoice(Wallpaper.Photo, PhotoEffect.Original, 0f), choice.wallpaperLight)
        // Read once: the old keys are gone and the new ones say the same.
        assertFalse(prefs.contains("wallpaperDark"))
        assertFalse(prefs.contains("wallpaperLight"))
        assertEquals("Ruins", prefs.getString("wallpaperDarkImage", null))
        assertEquals(choice, Appearance.load(context))
        // Eyes shipped with its scanlines in it; Torii dither is Torii with Mono.
        prefs.edit().clear().putString("wallpaperDark", "Eyes").putString("wallpaperLight", "ToriiDither").commit()
        assertEquals(WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Scanlines), Appearance.load(context).wallpaperDark)
        assertEquals(WallpaperChoice(Wallpaper.Torii, PhotoEffect.Mono), Appearance.load(context).wallpaperLight)
    }

    @Test
    fun choicesKeptBeforeTheOriginalsLookAsTheyDid() {
        fun keep(dark: Boolean, image: String, effect: String, fade: Float = 0f) {
            val t = if (dark) "Dark" else "Light"
            prefs.edit().clear()
                .putString("wallpaper${t}Image", image).putString("wallpaper${t}Effect", effect).putFloat("wallpaper${t}Fade", fade)
                .commit()
        }
        val cases = listOf(
            // Original meant the picture as shipped: Eyes had its scanlines, Torii was the photo.
            Triple("Eyes", "Original", WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Scanlines, 0.3f)),
            Triple("Torii", "Original", WallpaperChoice(Wallpaper.Torii, PhotoEffect.Original, 0.3f)),
            Triple("Halo", "Original", WallpaperChoice(Wallpaper.Halo, PhotoEffect.Mono, 0.3f)),
            // Torii dither is Torii with Mono; an effect on it now goes on the photo.
            Triple("ToriiDither", "Original", WallpaperChoice(Wallpaper.Torii, PhotoEffect.Mono, 0.3f)),
            Triple("ToriiDither", "Halftone", WallpaperChoice(Wallpaper.Torii, PhotoEffect.Halftone, 0.3f)),
            // Colour (a saturation lift) is gone: the picture as shipped.
            Triple("Eyes", "Colour", WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Scanlines, 0.3f)),
            Triple("Photo", "Colour", WallpaperChoice(Wallpaper.Photo, PhotoEffect.Original, 0.3f)),
            // An image that ships treated was drawn as it ships, whatever the effect: its own effect now.
            Triple("Halo", "Dither", WallpaperChoice(Wallpaper.Halo, PhotoEffect.Mono, 0.3f)),
            Triple("Crescent", "Halftone", WallpaperChoice(Wallpaper.Crescent, PhotoEffect.Scanlines, 0.3f)),
            // Effects on an image that came as it was stay.
            Triple("Hands", "Dither", WallpaperChoice(Wallpaper.Hands, PhotoEffect.Dither, 0.3f)),
            Triple("Photo", "Scanlines", WallpaperChoice(Wallpaper.Photo, PhotoEffect.Scanlines, 0.3f)),
        )
        for (dark in listOf(true, false)) {
            for ((image, effect, expected) in cases) {
                keep(dark, image, effect, 0.3f)
                assertEquals("$image $effect", expected, Appearance.load(context).wallpaper(dark))
            }
        }
        // Moved once: written back as read, and read the same after.
        Wallpapers.oldMadeDir(context).mkdirs()
        keep(true, "Eyes", "Original")
        val moved = Appearance.load(context)
        assertEquals(WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Scanlines), moved.wallpaperDark)
        assertEquals("Scanlines", prefs.getString("wallpaperDarkEffect", null))
        assertEquals(moved, Appearance.load(context))
        assertFalse("pictures made with the old effects go", Wallpapers.oldMadeDir(context).exists())
        // Eyes as it is, picked since, stays as it is.
        Appearance.set(context, moved.withWallpaper(true, WallpaperChoice(Wallpaper.Eyes)))
        assertEquals(WallpaperChoice(Wallpaper.Eyes), Appearance.load(context).wallpaperDark)
    }

    @Test
    fun pickingAnImageGivesItsOwnEffectAndKeepsTheFade() {
        val current = WallpaperChoice(Wallpaper.Halo, PhotoEffect.Original, 0.4f)
        assertEquals(WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Scanlines, 0.4f), current.withImage(Wallpaper.Eyes))
        assertEquals(WallpaperChoice(Wallpaper.Torii, PhotoEffect.Mono, 0.4f), current.withImage(Wallpaper.Torii))
        assertEquals(WallpaperChoice(Wallpaper.Ruins, PhotoEffect.Dither, 0.4f), WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Dither, 0.4f).withImage(Wallpaper.Ruins))
        assertEquals(WallpaperChoice(Wallpaper.Photo, PhotoEffect.Original, 0.4f), current.withImage(Wallpaper.Photo))
        assertEquals(WallpaperChoice(Wallpaper.Halo, PhotoEffect.Mono, 0.4f), current.withImage(Wallpaper.Halo))
    }

    /** The shipped set, in the picker's order, with each image's own effect: the same list as the iPhone's. */
    @Test
    fun theShippedSetAndItsDefaults() {
        assertEquals(
            listOf(
                Wallpaper.Halo to PhotoEffect.Mono,
                Wallpaper.Crescent to PhotoEffect.Scanlines,
                Wallpaper.Figure to PhotoEffect.Halftone,
                Wallpaper.Hands to PhotoEffect.Original,
                Wallpaper.Ruins to PhotoEffect.Dither,
                Wallpaper.Eyes to PhotoEffect.Scanlines,
                Wallpaper.Torii to PhotoEffect.Mono,
            ),
            Wallpaper.shipped.map { it to it.defaultEffect },
        )
        assertEquals(listOf(Wallpaper.Halo, Wallpaper.Crescent, Wallpaper.Figure, Wallpaper.Ruins), Wallpaper.shipped.filter { it.treated })
        assertTrue(Wallpaper.shipped.all { it.foot != null })
    }

    @Test
    fun everyShippedImageTakesEveryEffect() {
        for (image in Wallpaper.shipped) {
            val drawn = PhotoEffect.entries.map { e -> pixels(Wallpapers.bitmap(context, WallpaperChoice(image, e), true)!!).toList() }
            assertEquals("$image: every effect draws its own picture", drawn.size, drawn.toSet().size)
        }
    }

    @Test
    fun theDefaultEffectWithNoFadeIsTheShippedFileExactly() {
        // Eyes and Torii ship as they were: their own effect is drawn on them.
        for (image in Wallpaper.shipped - Wallpaper.Eyes - Wallpaper.Torii) {
            for (dark in listOf(true, false)) {
                val choice = WallpaperChoice.of(image)
                assertTrue("$image", choice.plain)
                val shown = Wallpapers.bitmap(context, choice, dark)!!
                val asset = context.assets.open("wallpapers/${image.name.lowercase()}.jpg").use { BitmapFactory.decodeStream(it) }
                assertTrue("$image as picked is the shipped file", pixels(asset).contentEquals(pixels(shown)))
            }
        }
        // Another effect, or Original, on a treated image is drawn from its untreated original.
        val original = Wallpapers.bitmap(context, WallpaperChoice(Wallpaper.Halo), true)!!
        val file = context.assets.open("wallpapers/halo-original.jpg").use { BitmapFactory.decodeStream(it) }
        assertTrue(pixels(file).contentEquals(pixels(original)))
        assertFalse(WallpaperChoice(Wallpaper.Halo, PhotoEffect.Dither).plain)
    }

    @Test
    fun aTreatedImageKeptBeforeEveryImageTookEffectsLooksAsItDid() {
        // Looks 2: Halo with no effect (drawn as it ships) and a fade.
        prefs.edit().putInt("wallpaperLooks", 2)
            .putString("wallpaperDarkImage", "Halo").putString("wallpaperDarkEffect", "Original").putFloat("wallpaperDarkFade", 0.3f)
            .putString("wallpaperLightImage", "Hands").putString("wallpaperLightEffect", "Dither").putFloat("wallpaperLightFade", 0f)
            .commit()
        val read = Appearance.load(context)
        assertEquals(WallpaperChoice(Wallpaper.Halo, PhotoEffect.Mono, 0.3f), read.wallpaperDark)
        assertEquals(WallpaperChoice(Wallpaper.Hands, PhotoEffect.Dither), read.wallpaperLight)
        assertEquals(3, prefs.getInt("wallpaperLooks", 0))
        assertEquals("Mono", prefs.getString("wallpaperDarkEffect", null))
        // The same pixels as before: the shipped file faded, nothing drawn on it.
        val made = Wallpapers.bitmap(context, read.wallpaperDark, true)!!
        val asset = context.assets.open("wallpapers/halo.jpg").use { BitmapFactory.decodeStream(it) }
        assertTrue(pixels(PhotoEffects.apply(asset, PhotoEffect.Original, 0.3f, true, Wallpaper.Halo.foot)).contentEquals(pixels(made)))
        // Halo with Original, picked since, stays: the untreated original.
        Appearance.set(context, read.withWallpaper(true, WallpaperChoice(Wallpaper.Halo)))
        assertEquals(WallpaperChoice(Wallpaper.Halo), Appearance.load(context).wallpaperDark)
    }

    @Test
    fun eyesAndToriiShipAsTheyWere() {
        // The assets are the boards' sources (faded at the foot), so an effect is never drawn on an effect.
        val eyes = Wallpapers.base(context, WallpaperChoice(Wallpaper.Eyes), true)!!
        val px = pixels(eyes)
        // No scanlines: neighbouring rows near the top are alike.
        var diff = 0L
        for (x in 0 until eyes.width) diff += kotlin.math.abs(((px[200 * eyes.width + x] shr 8) and 0xFF) - ((px[201 * eyes.width + x] shr 8) and 0xFF))
        assertTrue("Eyes has no scanlines: ${diff / eyes.width}", diff / eyes.width < 6)
        assertEquals(null, runCatching { context.assets.open("wallpapers/torii-dither.jpg") }.getOrNull())
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
        val base = WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Dither, 0.5f)
        val keys = listOf(
            Wallpapers.key(base, true),
            Wallpapers.key(base, false),
            Wallpapers.key(base.copy(image = Wallpaper.Torii), true),
            Wallpapers.key(base.copy(effect = PhotoEffect.Halftone), true),
            Wallpapers.key(base.copy(fade = 0.6f), true),
            Wallpapers.key(base.copy(image = Wallpaper.Photo), true),
        )
        assertEquals(keys.size, keys.toSet().size)
        // The fade is kept in hundredths: the same step is the same picture.
        assertEquals(Wallpapers.key(base.copy(fade = 0.501f), true), Wallpapers.key(base.copy(fade = 0.499f), true))
        assertEquals("Eyes-Dither-50-dark-r4", Wallpapers.key(base, true))
    }

    @Test
    fun aShippedImageAsItIsIsTheAssetUnchanged() {
        for (dark in listOf(true, false)) {
            val choice = WallpaperChoice.of(Wallpaper.Halo)
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
        val choice = WallpaperChoice(Wallpaper.Eyes, PhotoEffect.Dither, 0.4f)
        assertEquals(null, Wallpapers.ready(context, choice, true))
        val made = Wallpapers.bitmap(context, choice, true)!!
        assertTrue(Wallpapers.madeFile(context, choice, true).isFile)
        assertSame(made, Wallpapers.ready(context, choice, true))
        assertSame(made, Wallpapers.bitmap(context, choice, true))
        assertFalse(pixels(made).contentEquals(pixels(Wallpapers.bitmap(context, WallpaperChoice(Wallpaper.Eyes), true)!!)))
        // Read back from its file once memory lets it go, at once (the page opens with it).
        Wallpapers.forget()
        assertTrue(pixels(made).contentEquals(pixels(Wallpapers.ready(context, choice, true)!!)))
        // The other theme is its own picture.
        Wallpapers.bitmap(context, choice, false)
        assertTrue(Wallpapers.madeFile(context, choice, false).isFile)
        // The frost follows the made picture, not the asset.
        val frosted = Wallpapers.frosted(choice, true, made)
        assertNotEquals(pixels(frosted).toList(), pixels(Wallpapers.frosted(WallpaperChoice(Wallpaper.Eyes), true, Wallpapers.bitmap(context, WallpaperChoice(Wallpaper.Eyes), true)!!)).toList())
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
