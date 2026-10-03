package dev.rewarden.android.platform.update

import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest

/** An update server in memory: one manifest and the APKs it may point at. */
class FakeUpdateServer : UpdateFetcher {
    @Volatile var manifest: String = manifestJson(1, "0.1.0", "rewarden-0.1.0-1.apk", sha(byteArrayOf(1)), 1)
    val files = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()
    @Volatile var failure: Exception? = null
    @Volatile var manifestCalls = 0
    @Volatile var apkCalls = 0

    /** When set, downloads stop halfway until it opens. */
    @Volatile var gate: java.util.concurrent.CountDownLatch? = null

    /** Publishes [bytes] as release [versionCode] and returns what the manifest says about it. */
    fun publish(versionCode: Long, bytes: ByteArray = apkBytes(versionCode), versionName: String = "0.2.$versionCode"): Release {
        val file = "rewarden-$versionName-$versionCode.apk"
        files[file] = bytes
        manifest = manifestJson(versionCode, versionName, file, sha(bytes), bytes.size.toLong())
        return Release.parse(manifest)
    }

    override fun manifest(): String {
        manifestCalls++
        failure?.let { throw it }
        return manifest
    }

    override fun apk(file: String): InputStream {
        apkCalls++
        failure?.let { throw it }
        val bytes = files[file] ?: throw IOException("404 $file")
        val gate = gate ?: return ByteArrayInputStream(bytes)
        val half = bytes.size / 2
        return java.io.SequenceInputStream(
            ByteArrayInputStream(bytes, 0, half),
            object : InputStream() {
                private val rest by lazy {
                    gate.await(20, java.util.concurrent.TimeUnit.SECONDS)
                    ByteArrayInputStream(bytes, half, bytes.size - half)
                }

                override fun read(): Int = rest.read()

                override fun read(b: ByteArray, off: Int, len: Int): Int = rest.read(b, off, len)
            },
        )
    }

    companion object {
        fun apkBytes(seed: Long, size: Int = 200_000): ByteArray = ByteArray(size) { ((it * 31 + seed) % 251).toByte() }

        fun sha(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

        fun manifestJson(versionCode: Long, versionName: String, file: String, sha256: String, size: Long, build: String = "$versionName-202610010115-a9643bcb") =
            """{"versionCode": $versionCode, "versionName": "$versionName", "build": "$build",
               "file": "$file", "sha256": "$sha256", "size": $size, "published_at": 1790745300}"""
    }
}

class MemoryUpdateStore : UpdateStore {
    override var autoDownload = true
    override var lastCheckAt = 0L
    override var snoozedVersion = 0L
    override var snoozedUntil = 0L
    override var notifiedVersion = 0L
    override var downloaded: String? = null
}

/** Records what the updater asked the phone to install. */
class FakeInstaller : AppInstaller {
    @Volatile var allowed = true
    val installed = mutableListOf<File>()
    var permissionScreens = 0

    override fun canInstall(): Boolean = allowed

    override fun openPermissionSettings(context: android.content.Context) {
        permissionScreens++
    }

    override fun install(file: File) {
        installed += file
    }
}
