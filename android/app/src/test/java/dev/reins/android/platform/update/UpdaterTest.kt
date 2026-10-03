package dev.reins.android.platform.update

import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.io.IOException
import java.nio.file.Files
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class UpdaterTest {
    private val server = FakeUpdateServer()
    private val store = MemoryUpdateStore()
    private val dir: File = Files.createTempDirectory("updates").toFile()
    private var now = 1_790_000_000_000L

    private fun updater(current: Long = 10) = Updater({ server }, dir, store, current) { now }

    private fun files() = dir.listFiles().orEmpty().map { it.name }.sorted()

    @Test
    fun `a newer release is reported and the check is remembered`() {
        val r = server.publish(11)
        assertEquals(r, updater().latest())
        assertEquals(now, store.lastCheckAt)
    }

    @Test
    fun `the same or an older release is not an update`() {
        server.publish(10)
        assertNull(updater().latest())
        server.publish(9)
        assertNull(updater().latest())
    }

    @Test
    fun `a bad manifest fails the check`() {
        server.manifest = "{ nope"
        assertThrows(UpdateException.BadManifest::class.java) { updater().latest() }
    }

    @Test
    fun `network failures are reported as such`() {
        server.failure = IOException("timeout")
        assertThrows(UpdateException.Network::class.java) { updater().latest() }
    }

    @Test
    fun `a download is verified, reports progress and is remembered`() {
        val bytes = FakeUpdateServer.apkBytes(11, 300_000)
        val r = server.publish(11, bytes)
        val seen = mutableListOf<Long>()
        val file = updater().download(r) { seen += it }
        assertArrayEquals(bytes, file.readBytes())
        assertEquals(listOf(r.file), files())
        assertEquals(300_000L, seen.last())
        assertTrue(seen.zipWithNext().all { (a, b) -> b >= a })
        assertEquals(r, updater().downloaded())
    }

    @Test
    fun `a download that is already there is not fetched again`() {
        val r = server.publish(11)
        updater().download(r)
        updater().download(r)
        assertEquals(1, server.apkCalls)
    }

    @Test
    fun `a digest mismatch rejects and deletes the download`() {
        val r = server.publish(11)
        server.files[r.file] = FakeUpdateServer.apkBytes(99, r.size.toInt())
        assertThrows(UpdateException.Corrupt::class.java) { updater().download(r) }
        assertEquals(emptyList<String>(), files())
        assertNull(updater().downloaded())
    }

    @Test
    fun `a short or long download is rejected and deleted`() {
        val r = server.publish(11)
        server.files[r.file] = FakeUpdateServer.apkBytes(11, r.size.toInt() - 1)
        assertThrows(UpdateException.Corrupt::class.java) { updater().download(r) }
        assertEquals(emptyList<String>(), files())
        server.files[r.file] = FakeUpdateServer.apkBytes(11, r.size.toInt() + 10)
        assertThrows(UpdateException.Corrupt::class.java) { updater().download(r) }
        assertEquals(emptyList<String>(), files())
    }

    @Test
    fun `an interrupted download leaves nothing behind`() {
        val r = server.publish(11)
        server.failure = IOException("reset")
        assertThrows(UpdateException.Network::class.java) { updater().download(r) }
        assertEquals(emptyList<String>(), files())
    }

    @Test
    fun `a file changed after the download no longer verifies and is deleted`() {
        val r = server.publish(11)
        val file = updater().download(r)
        assertNotNull(updater().verified(r))
        file.writeBytes(FakeUpdateServer.apkBytes(5, r.size.toInt()))
        assertNull(updater().verified(r))
        assertFalse(file.exists())
        assertNull(updater().downloaded())
    }

    @Test
    fun `stale downloads are deleted when a newer release appears or the app is up to date`() {
        val old = server.publish(11)
        updater().download(old)
        File(dir, "leftover.apk.part").writeText("x")
        server.publish(12)
        updater().latest()
        assertEquals(emptyList<String>(), files())
        assertNull(updater().downloaded())

        val r12 = server.publish(12)
        updater().download(r12)
        // The phone now runs 12: the file is of no use any more.
        assertNull(updater(current = 12).downloaded())
        assertEquals(emptyList<String>(), files())
    }

    @Test
    fun `the current release is kept while it is still the latest`() {
        val r = server.publish(11)
        updater().download(r)
        updater().latest()
        assertEquals(listOf(r.file), files())
        assertEquals(r, updater().downloaded())
    }

    @Test
    fun `automatic checks run at most every three hours`() {
        server.publish(11)
        assertTrue(updater().dueForCheck())
        updater().latest()
        now += 3 * 3600_000L - 1
        assertFalse(updater().dueForCheck())
        now += 1
        assertTrue(updater().dueForCheck())
    }

    @Test
    fun `later snoozes that version for a day`() {
        val r = server.publish(11)
        val u = updater()
        assertFalse(u.isSnoozed(r))
        u.snooze(r)
        assertTrue(u.isSnoozed(r))
        now += 24 * 3600_000L - 1
        assertTrue(u.isSnoozed(r))
        now += 1
        assertFalse(u.isSnoozed(r))
        u.snooze(r)
        assertFalse(u.isSnoozed(server.publish(12)))
        u.unsnooze()
        assertFalse(u.isSnoozed(r))
    }
}
