package dev.reins.android

import dev.reins.android.platform.SealedPreferences
import dev.reins.core.KeyWrapper
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/** Robolectric has no hardware Keystore. Native-cache tests still use authenticated AES-GCM. */
object TestNativeKeys {
    fun install() {
        SealedPreferences.keyFactory = { alias -> object : KeyWrapper {
            private val key = SecretKeySpec(MessageDigest.getInstance("SHA-256").digest(alias.toByteArray()), "AES")
            override fun wrap(plaintext: ByteArray): ByteArray {
                val c = Cipher.getInstance("AES/GCM/NoPadding")
                c.init(Cipher.ENCRYPT_MODE, key)
                return c.iv + c.doFinal(plaintext)
            }
            override fun unwrap(wrapped: ByteArray): ByteArray {
                val c = Cipher.getInstance("AES/GCM/NoPadding")
                c.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, wrapped, 0, 12))
                return c.doFinal(wrapped, 12, wrapped.size - 12)
            }
        } }
    }
}
