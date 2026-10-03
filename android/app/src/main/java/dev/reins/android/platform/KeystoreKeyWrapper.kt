package dev.reins.android.platform

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import dev.reins.core.ForeignException
import dev.reins.core.KeyWrapper
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Wraps the core's data-encryption key with a non-exportable AES-256-GCM key in the Android Keystore
 * (StrongBox when the device has it). Output layout: 12-byte IV followed by ciphertext and tag.
 * A key that was lost or invalidated makes [unwrap] fail; the core then starts over with a fresh key.
 */
class KeystoreKeyWrapper(private val alias: String = DEFAULT_ALIAS) : KeyWrapper {
    private val store: KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

    @Synchronized
    override fun wrap(plaintext: ByteArray): ByteArray = guarded("wrap") {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        cipher.iv + cipher.doFinal(plaintext)
    }

    @Synchronized
    override fun unwrap(wrapped: ByteArray): ByteArray = guarded("unwrap") {
        require(wrapped.size > IV_BYTES + TAG_BYTES) { "wrapped key is too short" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        val existing = store.getKey(alias, null) as? SecretKey ?: error("the Keystore key is gone")
        cipher.init(Cipher.DECRYPT_MODE, existing, GCMParameterSpec(TAG_BYTES * 8, wrapped, 0, IV_BYTES))
        cipher.doFinal(wrapped, IV_BYTES, wrapped.size - IV_BYTES)
    }

    private fun key(): SecretKey = (store.getKey(alias, null) as? SecretKey) ?: generate()

    private fun generate(): SecretKey {
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
            .setIsStrongBoxBacked(strongBox)
            .build()

        fun make(strongBox: Boolean): SecretKey {
            val generator = javax.crypto.KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
            generator.init(spec(strongBox))
            return generator.generateKey()
        }
        return try {
            make(strongBox = true)
        } catch (_: StrongBoxUnavailableException) {
            make(strongBox = false)
        }
    }

    private inline fun <T> guarded(what: String, block: () -> T): T = try {
        block()
    } catch (e: Exception) {
        // Never include key material or plaintext; only the failure class.
        throw ForeignException.Failed("Keystore $what failed: ${e.javaClass.simpleName}")
    }

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
        const val DEFAULT_ALIAS = "reins_dek_wrap_v1"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BYTES = 16
    }
}
