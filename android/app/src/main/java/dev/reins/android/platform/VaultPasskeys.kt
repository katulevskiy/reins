package dev.reins.android.platform

import android.app.Activity
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialNoCreateOptionException
import androidx.credentials.exceptions.CreateCredentialProviderConfigurationException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialProviderConfigurationException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.NoCredentialException
import androidx.credentials.exceptions.domerrors.DomError
import androidx.credentials.exceptions.domerrors.InvalidStateError
import androidx.credentials.exceptions.domerrors.NotAllowedError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException
import dev.reins.android.ui.common.PASSKEY_UNSUPPORTED
import dev.reins.core.ReinsCoreInterface
import dev.reins.core.VaultPasskeyOptions
import dev.reins.core.VaultPasskeyView
import java.util.Base64
import org.json.JSONArray
import org.json.JSONObject

/** What the platform's passkey UI gave back. */
sealed interface PasskeyResult {
    /**
     * The passkey ([credentialId]) and its PRF output. [prf] is null only from [PasskeyPrompt.create], when the provider
     * gives it only as a passkey is used ([addVaultPasskey] then asks for it).
     */
    class Passkey(val credentialId: ByteArray, val prf: ByteArray?) : PasskeyResult

    /** The provider has no PRF, or no passkey for the account: it cannot open the vault. */
    data object Unsupported : PasskeyResult

    /** The user closed the prompt. */
    data object Cancelled : PasskeyResult

    /** Anything else, in words to show. */
    data class Failed(val message: String) : PasskeyResult
}

/** The platform's passkey UI, for the passkey that opens the vault. Every outcome is a [PasskeyResult]. */
interface PasskeyPrompt {
    /** Makes a passkey for the account with the WebAuthn `prf` extension; the passkeys it has already are excluded. */
    suspend fun create(options: VaultPasskeyOptions): PasskeyResult

    /** Uses one of the passkeys [allowed] (credential ids), evaluating its PRF on the options' salt. */
    suspend fun get(options: VaultPasskeyOptions, allowed: List<ByteArray>): PasskeyResult
}

/** A passkey could not be made to open the vault; [message] says why, in words to show. */
class PasskeyException(override val message: String) : Exception(message)

/**
 * "Add a passkey": makes one with [prompt] and has the core keep the vault's copy for it, named [name] (the phone it
 * was made on). A provider that gives the PRF output only as a passkey is used is asked once more, for the one just
 * made. Null when the user closed a prompt; [PasskeyException] when the provider cannot open the vault, and nothing is
 * added then. Only where the vault is open (the core refuses otherwise).
 */
suspend fun addVaultPasskey(core: ReinsCoreInterface, prompt: PasskeyPrompt, name: String): List<VaultPasskeyView>? {
    val options = core.vaultPasskeyOptions()
    val made = prompt.create(options).passkey() ?: return null
    val prf = made.prf ?: prompt.get(options, listOf(made.credentialId)).passkey()?.let {
        it.prf ?: throw PasskeyException(PASSKEY_UNSUPPORTED)
    } ?: return null
    return core.addVaultPasskey(made.credentialId, prf, name)
}

/** The passkey, or null when the user cancelled; a provider that cannot give one throws. */
private fun PasskeyResult.passkey(): PasskeyResult.Passkey? = when (this) {
    is PasskeyResult.Passkey -> this
    PasskeyResult.Cancelled -> null
    PasskeyResult.Unsupported -> throw PasskeyException(PASSKEY_UNSUPPORTED)
    is PasskeyResult.Failed -> throw PasskeyException(message)
}

/**
 * The WebAuthn JSON Credential Manager takes and gives (byte strings in base64url without padding). Separate from the
 * prompt so it can be checked without one.
 */
internal object PasskeyJson {
    fun creation(options: VaultPasskeyOptions): String = JSONObject()
        .put("rp", JSONObject().put("id", options.rpId).put("name", "Reins"))
        .put(
            "user",
            JSONObject().put("id", b64(options.userHandle)).put("name", options.userName).put("displayName", options.userName),
        )
        .put("challenge", b64(options.challenge))
        // ES256, then RS256 for the providers that only have that.
        .put("pubKeyCredParams", JSONArray().put(algorithm(-7)).put(algorithm(-257)))
        .put("excludeCredentials", descriptors(options.credentialIds))
        .put("authenticatorSelection", JSONObject().put("residentKey", "required").put("userVerification", "required"))
        .put("extensions", prf(options))
        .toString()

    fun assertion(options: VaultPasskeyOptions, allowed: List<ByteArray>): String = JSONObject()
        .put("challenge", b64(options.challenge))
        .put("rpId", options.rpId)
        .put("allowCredentials", descriptors(allowed))
        .put("userVerification", "required")
        .put("extensions", prf(options))
        .toString()

    /**
     * A registration ([created]) or an assertion response: the credential id and `clientExtensionResults.prf`. No `prf`
     * there, or `enabled: false`, is a provider without PRF. A new passkey with `enabled: true` but no result yet gives
     * a [PasskeyResult.Passkey] without its PRF output.
     */
    fun parse(json: String, created: Boolean): PasskeyResult {
        val unreadable = PasskeyResult.Failed("The password manager gave an answer Reins cannot read. Try again.")
        val response = runCatching { JSONObject(json) }.getOrNull() ?: return unreadable
        val id = response.optString("rawId").ifEmpty { response.optString("id") }
        val credentialId = unb64(id)?.takeIf { it.isNotEmpty() } ?: return unreadable
        val prf = response.optJSONObject("clientExtensionResults")?.optJSONObject("prf") ?: return PasskeyResult.Unsupported
        val first = prf.optJSONObject("results")?.optString("first")?.let(::unb64)?.takeIf { it.isNotEmpty() }
        return when {
            first != null -> PasskeyResult.Passkey(credentialId, first)
            created && prf.optBoolean("enabled", false) -> PasskeyResult.Passkey(credentialId, null)
            else -> PasskeyResult.Unsupported
        }
    }

    private fun prf(options: VaultPasskeyOptions) =
        JSONObject().put("prf", JSONObject().put("eval", JSONObject().put("first", b64(options.prfSalt))))

    private fun algorithm(alg: Int) = JSONObject().put("type", "public-key").put("alg", alg)

    private fun descriptors(ids: List<ByteArray>) =
        JSONArray().apply { ids.forEach { put(JSONObject().put("type", "public-key").put("id", b64(it))) } }

    fun b64(bytes: ByteArray): String = Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)

    fun unb64(text: String): ByteArray? = runCatching { Base64.getUrlDecoder().decode(text) }.getOrNull()
}

/**
 * The passkey UI of Credential Manager (Google Password Manager or the provider the user picked). It needs [activity]
 * for its sheet, like the biometric prompt.
 */
class CredentialManagerPasskeys(private val activity: Activity) : PasskeyPrompt {
    private val manager by lazy { CredentialManager.create(activity) }

    override suspend fun create(options: VaultPasskeyOptions): PasskeyResult = try {
        val response = manager.createCredential(activity, CreatePublicKeyCredentialRequest(PasskeyJson.creation(options)))
        (response as? CreatePublicKeyCredentialResponse)?.let { PasskeyJson.parse(it.registrationResponseJson, created = true) }
            ?: PasskeyResult.Unsupported
    } catch (e: CreateCredentialCancellationException) {
        PasskeyResult.Cancelled
    } catch (e: CreatePublicKeyCredentialDomException) {
        dom(e.domError, made = true)
    } catch (e: CreateCredentialProviderConfigurationException) {
        PasskeyResult.Unsupported
    } catch (e: CreateCredentialUnsupportedException) {
        PasskeyResult.Unsupported
    } catch (e: CreateCredentialNoCreateOptionException) {
        PasskeyResult.Unsupported
    } catch (e: CreateCredentialException) {
        PasskeyResult.Failed("The passkey could not be made. Try again.")
    }

    override suspend fun get(options: VaultPasskeyOptions, allowed: List<ByteArray>): PasskeyResult = try {
        val option = GetPublicKeyCredentialOption(PasskeyJson.assertion(options, allowed))
        val credential = manager.getCredential(activity, GetCredentialRequest(listOf(option))).credential
        (credential as? PublicKeyCredential)?.let { PasskeyJson.parse(it.authenticationResponseJson, created = false) }
            ?: PasskeyResult.Unsupported
    } catch (e: GetCredentialCancellationException) {
        PasskeyResult.Cancelled
    } catch (e: GetPublicKeyCredentialDomException) {
        dom(e.domError, made = false)
    } catch (e: NoCredentialException) {
        PasskeyResult.Unsupported
    } catch (e: GetCredentialProviderConfigurationException) {
        PasskeyResult.Unsupported
    } catch (e: GetCredentialUnsupportedException) {
        PasskeyResult.Unsupported
    } catch (e: GetCredentialException) {
        PasskeyResult.Failed("The passkey could not be used. Try again.")
    }

    /** WebAuthn's own errors: NotAllowedError is how a provider says the user dismissed it (or let it time out). */
    private fun dom(error: DomError, made: Boolean): PasskeyResult = when {
        error is NotAllowedError -> PasskeyResult.Cancelled
        made && error is InvalidStateError -> PasskeyResult.Failed("This password manager has a passkey for your vault already.")
        made -> PasskeyResult.Failed("The passkey could not be made. Try again.")
        else -> PasskeyResult.Failed("The passkey could not be used. Try again.")
    }
}

/** Lets tests replace the platform's passkey UI. Production never touches this. */
object PasskeyPromptProvider {
    @Volatile
    var factory: (Activity) -> PasskeyPrompt = ::CredentialManagerPasskeys
}
