package dev.rewarden.android.platform.update

import org.json.JSONException
import org.json.JSONObject

/**
 * One published release, as `releases/android/latest.json` describes it (written by `scripts/release-android.sh`).
 * Only a manifest that passes every check below becomes a [Release], so the rest of the updater can trust its fields:
 * [file] is a plain file name (no path), [sha256] is 64 lower-case hex digits and [size] is 1 byte to 200 MiB.
 */
data class Release(
    /** Release time in minutes since 1970, so every release is newer than the one before. */
    val versionCode: Long,
    val versionName: String,
    val build: String,
    val file: String,
    val sha256: String,
    val size: Long,
    val publishedAt: Long,
) {
    fun isNewerThan(installedVersionCode: Long): Boolean = versionCode > installedVersionCode

    fun toJson(): String = JSONObject()
        .put("versionCode", versionCode)
        .put("versionName", versionName)
        .put("build", build)
        .put("file", file)
        .put("sha256", sha256)
        .put("size", size)
        .put("published_at", publishedAt)
        .toString()

    companion object {
        const val MAX_SIZE = 200L * 1024 * 1024
        private val FILE = Regex("[A-Za-z0-9._-]+\\.apk")
        private val SHA256 = Regex("[0-9a-fA-F]{64}")
        private val VERSION_NAME = Regex("[A-Za-z0-9.+-]{1,32}")
        private val BUILD = Regex("[A-Za-z0-9._+-]{1,100}")

        /** Reads and validates a manifest; anything unexpected is [UpdateException.BadManifest]. */
        fun parse(json: String): Release {
            val o = try {
                JSONObject(json)
            } catch (e: JSONException) {
                throw UpdateException.BadManifest("not a JSON object")
            }
            val versionCode = o.integer("versionCode")
            if (versionCode <= 0) throw UpdateException.BadManifest("versionCode $versionCode")
            val versionName = o.text("versionName")
            if (!VERSION_NAME.matches(versionName)) throw UpdateException.BadManifest("versionName")
            val build = o.text("build")
            if (!BUILD.matches(build)) throw UpdateException.BadManifest("build")
            val file = o.text("file")
            if (!FILE.matches(file) || file.startsWith(".")) throw UpdateException.BadManifest("file name")
            val sha256 = o.text("sha256")
            if (!SHA256.matches(sha256)) throw UpdateException.BadManifest("sha256")
            val size = o.integer("size")
            if (size !in 1..MAX_SIZE) throw UpdateException.BadManifest("size $size")
            val publishedAt = if (o.has("published_at")) o.integer("published_at") else 0L
            return Release(versionCode, versionName, build, file, sha256.lowercase(), size, publishedAt)
        }

        /** A whole JSON number; strings and fractions are refused rather than coerced. */
        private fun JSONObject.integer(key: String): Long = when (val v = opt(key)) {
            is Int -> v.toLong()
            is Long -> v
            else -> throw UpdateException.BadManifest("$key is not a whole number")
        }

        private fun JSONObject.text(key: String): String =
            opt(key) as? String ?: throw UpdateException.BadManifest("$key is missing")
    }
}
