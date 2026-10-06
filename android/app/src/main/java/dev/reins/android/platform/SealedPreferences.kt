package dev.reins.android.platform

import android.content.Context
import android.util.Base64

/** Small native caches are authenticated ciphertext under a separate Android Keystore key. */
class SealedPreferences(context: Context, name: String) {
    private val prefs = context.getSharedPreferences(name, Context.MODE_PRIVATE)
    private val keys by lazy { keyFactory("reins_native_cache_v1_$name") }
    init {
        // Plaintext caches from older versions have no reliable account binding.
        if (!prefs.getBoolean("sealed.v1", false)) prefs.edit().clear().putBoolean("sealed.v1", true).commit()
    }
    internal companion object {
        var keyFactory: (String) -> dev.reins.core.KeyWrapper = { KeystoreKeyWrapper(it) }
    }
    fun getString(key: String, fallback: String?): String? {
        val sealed = prefs.getString(key, null) ?: return fallback
        return runCatching {
            val plain = keys.unwrap(Base64.decode(sealed, Base64.NO_WRAP))
            try { plain.toString(Charsets.UTF_8).takeIf { it.startsWith(key + "\u0000") }?.substring(key.length + 1) }
            finally { plain.fill(0) }
        }.getOrNull() ?: fallback
    }
    fun getBoolean(key: String, fallback: Boolean) = getString(key, null)?.toBooleanStrictOrNull() ?: fallback
    fun getLong(key: String, fallback: Long) = getString(key, null)?.toLongOrNull() ?: fallback
    fun getStringSet(key: String, fallback: Set<String>): Set<String> = getString(key, null)?.split('\u0000')?.filter { it.isNotEmpty() }?.toSet() ?: fallback
    fun edit() = Editor()
    inner class Editor {
        private val editor = prefs.edit()
        fun putString(key: String, value: String): Editor = apply {
            val plain = (key + "\u0000" + value).toByteArray(Charsets.UTF_8)
            try { editor.putString(key, Base64.encodeToString(keys.wrap(plain), Base64.NO_WRAP)) } finally { plain.fill(0) }
        }
        fun remove(key: String): Editor = apply { editor.remove(key) }
        fun putBoolean(key: String, value: Boolean) = putString(key, value.toString())
        fun putLong(key: String, value: Long) = putString(key, value.toString())
        fun putStringSet(key: String, value: Set<String>) = putString(key, value.joinToString("\u0000"))
        fun clear(): Editor = apply { editor.clear().putBoolean("sealed.v1", true) }
        fun commit() = editor.commit()
        fun apply() { editor.apply() }
    }
}
