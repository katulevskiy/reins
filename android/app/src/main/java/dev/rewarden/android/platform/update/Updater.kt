package dev.rewarden.android.platform.update

import java.io.File
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest

/**
 * The update logic without Android: find the latest release, download it into [dir] (app-private storage) and verify
 * it, keep [dir] free of anything stale, and remember snoozes and checks in [store].
 *
 * Blocking and not thread-safe: [UpdateController] calls it on one background coroutine at a time.
 */
class Updater(
    private val fetcher: () -> UpdateFetcher,
    private val dir: File,
    private val store: UpdateStore,
    /** The running app's versionCode. */
    private val installed: Long,
    private val now: () -> Long = System::currentTimeMillis,
) {
    /** The latest release if it is newer than the running app, else null. Deletes downloads that are no longer of use. */
    fun latest(): Release? {
        val release = Release.parse(io { fetcher().manifest() })
        store.lastCheckAt = now()
        val newer = release.takeIf { it.isNewerThan(installed) }
        keepOnly(newer)
        return newer
    }

    /** Whether an automatic check is due (the last one is at least [CHECK_INTERVAL_MS] old). */
    fun dueForCheck(): Boolean = now() - store.lastCheckAt !in 0 until CHECK_INTERVAL_MS

    /**
     * Downloads [release] (unless it is already here) and verifies its size and SHA-256 while it streams. Returns the
     * verified file; on any failure nothing is left behind. [onProgress] gets the bytes received so far.
     */
    fun download(release: Release, onProgress: (Long) -> Unit = {}): File {
        val target = File(dir, release.file)
        if (downloaded() == release && target.length() == release.size) return target
        keepOnly(null)
        dir.mkdirs()
        val part = File(dir, release.file + ".part")
        try {
            io { fetcher().apk(release.file) }.use { input -> copyVerified(input, part, release, onProgress) }
            if (!part.renameTo(target)) throw UpdateException.Storage(IOException("could not move the download into place"))
        } catch (e: Exception) {
            part.delete()
            target.delete()
            throw e
        }
        store.downloaded = release.toJson()
        return target
    }

    /** A newer release downloaded earlier (its file is present with the right size), or null. */
    fun downloaded(): Release? {
        val release = store.downloaded?.let { runCatching { Release.parse(it) }.getOrNull() }
        if (release == null || !release.isNewerThan(installed) || File(dir, release.file).length() != release.size) {
            keepOnly(null)
            return null
        }
        return release
    }

    /** Hashes the downloaded file once more right before it is installed; a file that no longer matches is deleted. */
    fun verified(release: Release): File? {
        if (downloaded() != release) return null
        val file = File(dir, release.file)
        val ok = try {
            file.inputStream().use { sha256(it) } == release.sha256
        } catch (e: IOException) {
            false
        }
        if (!ok) keepOnly(null)
        return file.takeIf { ok }
    }

    /** Deletes every download (the installer found the file unusable). */
    fun discard() = keepOnly(null)

    var autoDownload: Boolean
        get() = store.autoDownload
        set(value) {
            store.autoDownload = value
        }

    /** "Later": the prompt stays away from this version for [SNOOZE_MS]. */
    fun snooze(release: Release) {
        store.snoozedVersion = release.versionCode
        store.snoozedUntil = now() + SNOOZE_MS
    }

    fun unsnooze() {
        store.snoozedVersion = 0
        store.snoozedUntil = 0
    }

    fun isSnoozed(release: Release): Boolean = store.snoozedVersion == release.versionCode && now() < store.snoozedUntil

    /** Whether a notification for [release] is still due; each version is announced at most once. */
    fun shouldNotify(release: Release): Boolean = release.versionCode > store.notifiedVersion

    fun markNotified(release: Release) {
        store.notifiedVersion = maxOf(store.notifiedVersion, release.versionCode)
    }

    /** Deletes everything in [dir] except the verified download of [keep]. */
    private fun keepOnly(keep: Release?) {
        val stored = store.downloaded?.let { runCatching { Release.parse(it) }.getOrNull() }
        val keepName = keep?.takeIf { it == stored }?.file
        dir.listFiles()?.filter { it.name != keepName }?.forEach { it.deleteRecursively() }
        if (keepName == null && store.downloaded != null) store.downloaded = null
    }

    private fun copyVerified(input: InputStream, part: File, release: Release, onProgress: (Long) -> Unit) {
        val digest = MessageDigest.getInstance("SHA-256")
        var total = 0L
        try {
            part.outputStream().use { out ->
                val buffer = ByteArray(64 * 1024)
                while (true) {
                    val n = io { input.read(buffer) }
                    if (n < 0) break
                    total += n
                    if (total > release.size) throw UpdateException.Corrupt("longer than ${release.size} bytes")
                    digest.update(buffer, 0, n)
                    out.write(buffer, 0, n)
                    onProgress(total)
                }
            }
        } catch (e: IOException) {
            throw UpdateException.Storage(e)
        }
        if (total != release.size) throw UpdateException.Corrupt("$total of ${release.size} bytes")
        if (digest.digest().toHex() != release.sha256) throw UpdateException.Corrupt("SHA-256 mismatch")
    }

    /** Network failures become [UpdateException.Network]; update errors pass through. */
    private inline fun <T> io(block: () -> T): T = try {
        block()
    } catch (e: UpdateException) {
        throw e
    } catch (e: IOException) {
        throw UpdateException.Network(e)
    }

    companion object {
        const val CHECK_INTERVAL_MS = 3 * 3600_000L
        const val SNOOZE_MS = 24 * 3600_000L

        fun sha256(input: InputStream): String {
            val digest = MessageDigest.getInstance("SHA-256")
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                digest.update(buffer, 0, n)
            }
            return digest.digest().toHex()
        }

        private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
    }
}
