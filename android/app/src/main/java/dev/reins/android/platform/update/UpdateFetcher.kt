package dev.rewarden.android.platform.update

import java.io.FilterInputStream
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL

/** Where releases come from. Blocking; called on a background thread. */
interface UpdateFetcher {
    /** The text of `latest.json`. */
    fun manifest(): String

    /** Opens the APK called [file] (already validated by [Release.parse]). The caller closes the stream. */
    fun apk(file: String): InputStream
}

/**
 * Fetches over HTTPS only: the manifest at [manifestUrl] and APKs from `files/` next to it. Redirects are followed by
 * [HttpURLConnection] only within HTTPS, and the final address is checked again. Integrity does not rest on the
 * transport alone: every APK is checked against the manifest's size and SHA-256 before it can be installed.
 */
class HttpUpdateFetcher(private val manifestUrl: String) : UpdateFetcher {
    override fun manifest(): String = open(URL(manifestUrl), MANIFEST_READ_TIMEOUT).use { stream ->
        val bytes = stream.readNBytes(MAX_MANIFEST + 1)
        if (bytes.size > MAX_MANIFEST) throw UpdateException.BadManifest("larger than $MAX_MANIFEST bytes")
        bytes.decodeToString()
    }

    override fun apk(file: String): InputStream = open(URL(URL(manifestUrl), "files/$file"), APK_READ_TIMEOUT)

    private fun open(url: URL, readTimeout: Int): InputStream {
        if (url.protocol != "https") throw UpdateException.Network(IOException("refusing ${url.protocol} for updates"))
        val connection = url.openConnection() as HttpURLConnection
        connection.connectTimeout = CONNECT_TIMEOUT
        connection.readTimeout = readTimeout
        connection.useCaches = false
        connection.setRequestProperty("Accept-Encoding", "identity")
        try {
            val status = connection.responseCode
            if (connection.url.protocol != "https") throw IOException("redirected away from https")
            if (status != HttpURLConnection.HTTP_OK) throw UpdateException.Http(status)
            return object : FilterInputStream(connection.inputStream) {
                override fun close() {
                    try {
                        super.close()
                    } finally {
                        connection.disconnect()
                    }
                }
            }
        } catch (e: Exception) {
            connection.disconnect()
            throw e
        }
    }

    private companion object {
        const val CONNECT_TIMEOUT = 10_000
        const val MANIFEST_READ_TIMEOUT = 10_000
        const val APK_READ_TIMEOUT = 30_000
        const val MAX_MANIFEST = 16 * 1024
    }
}
