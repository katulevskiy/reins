package dev.rewarden.android.design

import dev.rewarden.android.design.blobatar.Blobatar
import java.security.MessageDigest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The port must draw exactly what blobatar draws. These are blobatar's own golden fixtures
 * (`packages/blobatar/test/golden/gen2.txt`): every default seed of its 1000-name corpus plus its backdrop
 * variants, checked by the same SHA-256 tripwire the library uses, and a few complete markups so a failure reads.
 */
class BlobatarGoldenTest {
    private fun lines(name: String): List<List<String>> =
        javaClass.classLoader!!.getResourceAsStream(name)!!.bufferedReader(Charsets.UTF_8).readLines()
            .filter { it.isNotEmpty() && !it.startsWith("#") }
            .map { it.split('\t') }

    private fun backdrop(label: String) = when (label) {
        "", "bg:none" -> Blobatar.Backdrop.NONE
        "bg:square", "square" -> Blobatar.Backdrop.SQUARE
        "bg:circle" -> Blobatar.Backdrop.CIRCLE
        "bg:squircle", "squircle" -> Blobatar.Backdrop.SQUIRCLE
        else -> error("unknown case $label")
    }

    private fun sha16(text: String) =
        MessageDigest.getInstance("SHA-256").digest(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }.take(16)

    @Test
    fun `the port renders the same markup as the library for its whole golden corpus`() {
        val rows = lines("blobatar-gen2-hashes.tsv")
        assertTrue("fixture looks truncated: ${rows.size}", rows.size >= 1000)
        val wrong = rows.filter { (seed, label, hash) -> sha16(Blobatar.svg(seed, backdrop(label))) != hash }
        assertEquals("names whose blobatar differs from the library's", emptyList<String>(), wrong.map { "${it[0]}|${it[1]}" }.take(10))
    }

    @Test
    fun `complete markups match byte for byte`() {
        for ((name, background, markup) in lines("blobatar-gen2-markup.tsv")) {
            assertEquals("$name/$background", markup, Blobatar.svg(name, backdrop(background)))
        }
    }

    @Test
    fun `names are normalised like the library does`() {
        assertEquals(Blobatar.svg("alain@x.com"), Blobatar.svg("  Alain@X.com "))
        assertEquals(Blobatar.svg("café"), Blobatar.svg("café"))
    }
}
