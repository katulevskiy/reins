package dev.reins.android.platform

import android.content.Context
import android.util.Base64

/** Small native caches are authenticated ciphertext under a separate Android Keystore key. */
class SealedPreferences(context: Context, private val name: String) {
    private val prefs = context.getSharedPreferences(name, Context.MODE_PRIVATE)
    private val keys by lazy { keyFactory("reins_native_cache_v1_$name") }
    init {
        // Plaintext caches from older versions have no reliable account binding.
        if (!prefs.getBoolean("sealed.v1", false)) prefs.edit().clear().putBoolean("sealed.v1", true).commit()
    }
    internal companion object {
        var keyFactory: (String) -> dev.reins.core.KeyWrapper = { KeystoreKeyWrapper(it) }

        /**
         * What each stored ciphertext opened to, by `name`, key and that exact ciphertext. A Keystore operation is a
         * binder call (tens of ms on StrongBox), so a value is opened once per write instead of on every read; a
         * ciphertext changed on disk misses the cache and is opened (and authenticated) again.
         */
        private val opened = java.util.concurrent.ConcurrentHashMap<String, Pair<String, String>>()
    }
    private val prefix = name + "\u0000"
    fun getString(key: String, fallback: String?): String? {
        val sealed = prefs.getString(key, null) ?: return fallback
        opened[prefix + key]?.let { (ciphertext, value) -> if (ciphertext == sealed) return value }
        val value = runCatching {
            val plain = keys.unwrap(Base64.decode(sealed, Base64.NO_WRAP))
            try { plain.toString(Charsets.UTF_8).takeIf { it.startsWith(key + "\u0000") }?.substring(key.length + 1) }
            finally { plain.fill(0) }
        }.getOrNull() ?: return fallback
        opened[prefix + key] = sealed to value
        return value
    }
    fun getBoolean(key: String, fallback: Boolean) = getString(key, null)?.toBooleanStrictOrNull() ?: fallback
    fun getLong(key: String, fallback: Long) = getString(key, null)?.toLongOrNull() ?: fallback
    fun getStringSet(key: String, fallback: Set<String>): Set<String> = getString(key, null)?.split('\u0000')?.filter { it.isNotEmpty() }?.toSet() ?: fallback
    fun edit() = Editor()
    inner class Editor {
        private val editor = prefs.edit()
        private var cleared = false
        private val written = HashMap<String, Pair<String, String>?>()
        fun putString(key: String, value: String): Editor = apply {
            // Already stored as is: nothing to seal or write.
            if (!cleared && key !in written && getString(key, null) == value) return@apply
            val plain = (key + "\u0000" + value).toByteArray(Charsets.UTF_8)
            try {
                val sealed = Base64.encodeToString(keys.wrap(plain), Base64.NO_WRAP)
                editor.putString(key, sealed)
                written[key] = sealed to value
            } finally { plain.fill(0) }
        }
        fun remove(key: String): Editor = apply { editor.remove(key); written[key] = null }
        fun putBoolean(key: String, value: Boolean) = putString(key, value.toString())
        fun putLong(key: String, value: Long) = putString(key, value.toString())
        fun putStringSet(key: String, value: Set<String>) = putString(key, value.joinToString("\u0000"))
        fun clear(): Editor = apply { editor.clear().putBoolean("sealed.v1", true); cleared = true; written.clear() }
        fun commit() = editor.commit().also { remember() }
        fun apply() { editor.apply(); remember() }
        private fun remember() {
            if (cleared) opened.keys.removeAll { it.startsWith(prefix) }
            for ((key, entry) in written) if (entry == null) opened.remove(prefix + key) else opened[prefix + key] = entry
        }
    }
}
