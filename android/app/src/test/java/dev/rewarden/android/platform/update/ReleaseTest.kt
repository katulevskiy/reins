package dev.rewarden.android.platform.update

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.platform.update.FakeUpdateServer.Companion.manifestJson
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** org.json is part of Android, so this runs under Robolectric. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class ReleaseTest {
    private val sha = "a".repeat(64)

    private fun bad(json: String) {
        assertThrows(UpdateException.BadManifest::class.java) { Release.parse(json) }
    }

    @Test
    fun `the manifest the release script writes is read in full`() {
        val r = Release.parse(
            """{"versionCode": 29834567, "versionName": "0.1.0", "build": "0.1.0-202610010115-a9643bcb",
             "file": "rewarden-0.1.0-202610010115-a9643bcb.apk", "sha256": "$sha", "size": 34236184,
             "published_at": 1790745300}""",
        )
        assertEquals(29834567L, r.versionCode)
        assertEquals("0.1.0", r.versionName)
        assertEquals("0.1.0-202610010115-a9643bcb", r.build)
        assertEquals("rewarden-0.1.0-202610010115-a9643bcb.apk", r.file)
        assertEquals(sha, r.sha256)
        assertEquals(34236184L, r.size)
        assertEquals(1790745300L, r.publishedAt)
    }

    @Test
    fun `a stored release reads back the same`() {
        val r = Release.parse(manifestJson(42, "0.2.0", "rewarden-0.2.0.apk", sha, 1000))
        assertEquals(r, Release.parse(r.toJson()))
    }

    @Test
    fun `upper case digests are accepted and kept in lower case`() {
        assertEquals(sha, Release.parse(manifestJson(2, "0.1.0", "r.apk", sha.uppercase(), 10)).sha256)
    }

    @Test
    fun `malformed or incomplete manifests are refused`() {
        bad("")
        bad("not json")
        bad("[]")
        bad("""{"versionCode": 2, "versionName": "0.1.0"""")
        bad("""{"versionName": "0.1.0", "build": "b", "file": "r.apk", "sha256": "$sha", "size": 10}""")
        bad(manifestJson(2, "0.1.0", "r.apk", sha, 10).replace("\"versionCode\": 2", "\"versionCode\": \"2\""))
        bad(manifestJson(2, "0.1.0", "r.apk", sha, 10).replace("\"versionCode\": 2", "\"versionCode\": 2.5"))
        bad(manifestJson(0, "0.1.0", "r.apk", sha, 10))
        bad(manifestJson(-5, "0.1.0", "r.apk", sha, 10))
        bad(manifestJson(2, "", "r.apk", sha, 10))
        bad(manifestJson(2, "0.1.0 <b>", "r.apk", sha, 10))
        bad(manifestJson(2, "0.1.0", "r.apk", sha, 10, build = "x".repeat(200)))
    }

    @Test
    fun `file names that are not a plain apk name are refused`() {
        for (name in listOf("../evil.apk", "files/r.apk", ".hidden.apk", ".apk", "r.zip", "r.apk.zip", "r apk", "r .apk", "%2e%2e.apk", "r\\\\x.apk", "r\\u002fx.apk", "")) {
            bad(manifestJson(2, "0.1.0", name, sha, 10))
        }
        Release.parse(manifestJson(2, "0.1.0", "rewarden-0.1.0_b.1.apk", sha, 10))
    }

    @Test
    fun `digests must be 64 hex characters`() {
        bad(manifestJson(2, "0.1.0", "r.apk", "a".repeat(63), 10))
        bad(manifestJson(2, "0.1.0", "r.apk", "a".repeat(65), 10))
        bad(manifestJson(2, "0.1.0", "r.apk", "g".repeat(64), 10))
    }

    @Test
    fun `sizes must be between one byte and 200 MiB`() {
        bad(manifestJson(2, "0.1.0", "r.apk", sha, 0))
        bad(manifestJson(2, "0.1.0", "r.apk", sha, -1))
        bad(manifestJson(2, "0.1.0", "r.apk", sha, 200L * 1024 * 1024 + 1))
        Release.parse(manifestJson(2, "0.1.0", "r.apk", sha, 200L * 1024 * 1024))
        Release.parse(manifestJson(2, "0.1.0", "r.apk", sha, 1))
    }

    @Test
    fun `a release is newer only when its version code is higher`() {
        val r = Release.parse(manifestJson(100, "0.1.0", "r.apk", sha, 10))
        assertTrue(r.isNewerThan(99))
        assertFalse(r.isNewerThan(100))
        assertFalse(r.isNewerThan(101))
    }
}
