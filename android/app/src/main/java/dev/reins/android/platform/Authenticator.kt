package dev.reins.android.platform

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

sealed interface AuthResult {
    data object Success : AuthResult

    /** The user backed out or failed the check. */
    data object Cancelled : AuthResult

    /** No screen lock / biometric is set up, so approving is not possible. */
    data object Unavailable : AuthResult
}

/** Approving anything requires this to return [AuthResult.Success]; every other outcome fails closed. */
fun interface Authenticator {
    suspend fun authenticate(title: String, subtitle: String): AuthResult
}

private const val AUTHENTICATORS = BIOMETRIC_STRONG or DEVICE_CREDENTIAL

class BiometricAuthenticator(private val activity: FragmentActivity) : Authenticator {
    override suspend fun authenticate(title: String, subtitle: String): AuthResult {
        if (BiometricManager.from(activity).canAuthenticate(AUTHENTICATORS) != BiometricManager.BIOMETRIC_SUCCESS) {
            return AuthResult.Unavailable
        }
        return suspendCancellableCoroutine { continuation ->
            val prompt = BiometricPrompt(
                activity,
                ContextCompat.getMainExecutor(activity),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        if (continuation.isActive) continuation.resume(AuthResult.Success)
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        if (continuation.isActive) continuation.resume(AuthResult.Cancelled)
                    }
                    // onAuthenticationFailed (one bad attempt) keeps the prompt open; nothing to do.
                },
            )
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(AUTHENTICATORS)
                .setConfirmationRequired(false)
                .build()
            continuation.invokeOnCancellation { prompt.cancelAuthentication() }
            prompt.authenticate(info)
        }
    }
}

/** Lets instrumented tests replace the biometric prompt. Production never touches this. */
object AuthenticatorProvider {
    @Volatile
    var factory: (FragmentActivity) -> Authenticator = ::BiometricAuthenticator
}
