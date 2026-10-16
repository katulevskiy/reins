package dev.reins.android.design

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.design.blobatar.Blobatar
import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * A connection's blobatar keeps indexes into the shared palette and shape list, so the app's copies must be the
 * shared file's (`design/signal/tokens/blobatar-palette.json`, which the core and iOS check too); and the overrides
 * change only what they override.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class BlobatarPaletteTest {
    private fun fixture(): JSONObject {
        var dir: File? = File(System.getProperty("user.dir")!!).absoluteFile
        while (dir != null) {
            val file = File(dir, "design/signal/tokens/blobatar-palette.json")
            if (file.isFile) return JSONObject(file.readText())
            dir = dir.parentFile
        }
        error("blobatar-palette.json not found above ${System.getProperty("user.dir")}")
    }

    @Test
    fun `the palette and the shapes are the shared ones`() {
        val json = fixture()
        val colours = json.getJSONArray("colours")
        assertEquals(colours.length(), Blobatar.COLOURS.size)
        for (i in 0 until colours.length()) {
            val c = colours.getJSONObject(i)
            val mine = Blobatar.COLOURS[i]
            assertEquals("colour $i", c.getString("name"), mine.name.lowercase())
            assertEquals("colour $i l", c.getDouble("l"), mine.l, 0.0)
            assertEquals("colour $i c", c.getDouble("c"), mine.c, 0.0)
            assertEquals("colour $i h", c.getDouble("h"), mine.h, 0.0)
        }
        val shapes = json.getJSONArray("shapes")
        assertEquals((0 until shapes.length()).map { shapes.getString(it) }, Blobatar.SHAPES)
    }

    @Test
    fun `a shape override draws that shape and keeps the seed's other traits`() {
        // Overriding with the shape the seed picks anyway changes nothing.
        for (seed in listOf("c1", "M2A", "notes-bot", "honeypot")) {
            val natural = Blobatar.SHAPES.indices.single { i -> Blobatar.svg(seed, shape = i) == Blobatar.svg(seed) }
            assertTrue(natural in Blobatar.SHAPES.indices)
        }
        // Every shape is a different figure.
        val all = Blobatar.SHAPES.indices.map { Blobatar.svg("c1", shape = it) }
        assertEquals(all.size, all.toSet().size)
        // An index out of range is clamped, never a crash.
        assertEquals(Blobatar.svg("c1", shape = Blobatar.SHAPES.lastIndex), Blobatar.svg("c1", shape = 99))
    }

    @Test
    fun `a colour override paints the head in that colour, with eyes that read on it`() {
        for (colour in Blobatar.COLOURS) {
            val figure = Blobatar.figure("c1", colour = colour)
            val head = figure.marks.first().fill
            assertEquals(colour.name, Blobatar.headHex(colour), head)
            assertNotEquals(colour.name, head, figure.marks.last().fill)
        }
        // The same seed and shape, two colours: the same outline.
        val red = Blobatar.figure("c1", shape = 3, colour = Blobatar.COLOURS[0])
        val blue = Blobatar.figure("c1", shape = 3, colour = Blobatar.COLOURS[7])
        assertEquals(red.marks.map { (it as? Blobatar.Mark.Path)?.d }, blue.marks.map { (it as? Blobatar.Mark.Path)?.d })
        assertNotEquals(red.marks.first().fill, blue.marks.first().fill)
    }

    @Test
    fun `the twelve head colours stay apart`() {
        val heads = Blobatar.COLOURS.map(Blobatar::headHex)
        assertEquals(heads.size, heads.toSet().size)
    }
}
