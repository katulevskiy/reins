package dev.reins.android.profiles

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import dev.reins.android.BuildConfig
import dev.reins.android.platform.ScreenLock
import dev.reins.core.AttestationInput
import dev.reins.core.AttestationKindInput
import dev.reins.core.EnrollInput
import dev.reins.core.PostureInput
import java.io.File
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/** A work profile's org device key: made in secure hardware with an attestation of [challenge]. */
data class DeviceKey(
    /** SubjectPublicKeyInfo DER, base64. */
    val publicKey: String,
    /** StrongBox or the TEE holds it. */
    val hardware: Boolean,
    /** The key attestation certificate chain (DER, base64), leaf first; empty when the platform gave none. */
    val chain: List<String>,
)

interface DeviceKeys {
    /** Makes (replacing) the key [alias], attesting [challenge]. */
    fun create(alias: String, challenge: ByteArray): DeviceKey

    /** ECDSA P-256 SHA-256 (DER) of [data] with [alias]. */
    fun sign(alias: String, data: ByteArray): ByteArray
}

/** Android Keystore: StrongBox when the phone has it, else the TEE; key attestation with the challenge. */
class KeystoreDeviceKeys : DeviceKeys {
    private fun keyStore() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    override fun create(alias: String, challenge: ByteArray): DeviceKey {
        val store = keyStore()
        if (store.containsAlias(alias)) store.deleteEntry(alias)
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setAttestationChallenge(challenge)
            .setIsStrongBoxBacked(strongBox)
            .build()
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        val pair = try {
            generator.initialize(spec(true))
            generator.generateKeyPair()
        } catch (_: StrongBoxUnavailableException) {
            generator.initialize(spec(false))
            generator.generateKeyPair()
        }
        val info = KeyFactory.getInstance(pair.private.algorithm, "AndroidKeyStore").getKeySpec(pair.private, KeyInfo::class.java)
        val chain = keyStore().getCertificateChain(alias).orEmpty().map { Base64.encodeToString(it.encoded, Base64.NO_WRAP) }
        return DeviceKey(
            publicKey = Base64.encodeToString(pair.public.encoded, Base64.NO_WRAP),
            hardware = info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX ||
                info.securityLevel == KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT,
            chain = chain,
        )
    }

    override fun sign(alias: String, data: ByteArray): ByteArray {
        val key = keyStore().getKey(alias, null) as PrivateKey
        return Signature.getInstance("SHA256withECDSA").run {
            initSign(key)
            update(data)
            sign()
        }
    }
}

/** What the phone says about itself (06 §6); recorded by the org, gating comes later. */
object Posture {
    /** Signs of root: a test-keys build, or an `su` binary in the usual places. `null`: could not tell. */
    fun rooted(tags: String?, exists: (String) -> Boolean): Boolean? = runCatching {
        tags.orEmpty().contains("test-keys") || SU_PATHS.any(exists)
    }.getOrNull()

    private val SU_PATHS = listOf(
        "/system/bin/su", "/system/xbin/su", "/sbin/su", "/system/su", "/vendor/bin/su", "/su/bin/su", "/data/local/xbin/su",
    )

    fun collect(context: Context): PostureInput = PostureInput(
        screenLock = ScreenLock.check(context),
        osName = "Android",
        osVersion = Build.VERSION.RELEASE.orEmpty().ifEmpty { Build.VERSION.SDK_INT.toString() },
        rooted = rooted(Build.TAGS) { File(it).exists() },
        appVersion = BuildConfig.VERSION_NAME,
    )
}

/**
 * Signs with a work profile's enrolled org key (O2): an org review's answer or the member's own policy. The key was
 * made without user authentication, so no prompt; the phone's screen lock gates the app. ECDSA P-256 DER, base64.
 */
fun orgSign(keys: DeviceKeys, profileId: String, message: ByteArray): String =
    Base64.encodeToString(keys.sign(WorkProfiles.orgKeyAlias(profileId), message), Base64.NO_WRAP)

/**
 * The bytes an org review's answer signs: the core's (`org_review_message`), and a web passkey's confirmation
 * (`org_passkey_confirm_message`, over the listed key's hash and registration time); tests, without the native core,
 * replace them.
 */
object OrgSigning {
    val CORE: (String, dev.reins.core.OrgReviewView, Boolean, Long) -> ByteArray =
        { org, review, approve, at -> dev.reins.core.orgReviewMessage(org, review, approve, at) }

    @Volatile var message: (String, dev.reins.core.OrgReviewView, Boolean, Long) -> ByteArray = CORE

    val CORE_PASSKEY: (String, dev.reins.core.OrgPasskeyView, Long) -> ByteArray =
        { org, key, at -> dev.reins.core.orgPasskeyConfirmMessage(org, key.credentialId, key.spkiHash, key.createdAt, at) }

    @Volatile var passkeyMessage: (String, dev.reins.core.OrgPasskeyView, Long) -> ByteArray = CORE_PASSKEY
}

/**
 * The enrollment for [challenge]: a fresh org device key for [profileId], its signature over the challenge, the
 * key attestation chain, and the posture. The attestation's challenge is the enrollment message: the chain's leaf
 * certifies this very key (the binding O2's verifier checks with `reins_proto::org::attestation_binding`'s inputs).
 */
fun enrollment(keys: DeviceKeys, profileId: String, challenge: String, posture: PostureInput): EnrollInput {
    val alias = WorkProfiles.orgKeyAlias(profileId)
    val key = keys.create(alias, WorkProfiles.enrollmentMessage(challenge))
    val signature = keys.sign(alias, WorkProfiles.enrollmentMessage(challenge))
    return EnrollInput(
        deviceKey = key.publicKey,
        signature = Base64.encodeToString(signature, Base64.NO_WRAP),
        hardware = key.hardware,
        attestations = listOf(
            if (key.chain.isEmpty()) AttestationInput(AttestationKindInput.NONE, "")
            else AttestationInput(AttestationKindInput.ANDROID_KEY_ATTESTATION, key.chain.joinToString(",")),
        ),
        posture = posture,
    )
}
