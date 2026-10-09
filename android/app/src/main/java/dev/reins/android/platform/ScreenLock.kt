package dev.reins.android.platform

import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Approving anything needs the phone's screen lock or a strong biometric ([BiometricAuthenticator]). Without one every
 * approval fails, so the onboarding and the Activity tab say so before the first request does.
 */
object ScreenLock {
    /** Whether approving can work on this phone. Tests replace it: Robolectric has no screen lock. */
    @Volatile
    var check: (Context) -> Boolean = { context ->
        BiometricManager.from(context).canAuthenticate(BIOMETRIC_STRONG or DEVICE_CREDENTIAL) == BiometricManager.BIOMETRIC_SUCCESS
    }

    fun isSet(context: Context): Boolean = check(context)

    /** Android's screen lock settings: the enrolment screen where there is one, else Security. */
    fun openSettings(context: Context) {
        val enroll = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Intent(Settings.ACTION_BIOMETRIC_ENROLL)
                .putExtra(Settings.EXTRA_BIOMETRIC_AUTHENTICATORS_ALLOWED, BIOMETRIC_STRONG or DEVICE_CREDENTIAL)
        } else {
            Intent(Settings.ACTION_SECURITY_SETTINGS)
        }
        runCatching { context.startActivity(enroll.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }.onFailure {
            runCatching { context.startActivity(Intent(Settings.ACTION_SECURITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }
    }
}

/** The screen lock as the UI sees it: re-read every time the app comes back to the front (from Settings, say). */
@Stable
class ScreenLockState internal constructor(private val context: Context) {
    var set by mutableStateOf(true)
        internal set

    fun openSettings() = ScreenLock.openSettings(context)
}

@Composable
fun rememberScreenLockState(): ScreenLockState {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val state = remember { ScreenLockState(context) }
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            state.set = withContext(Dispatchers.Default) { ScreenLock.isSet(context) }
        }
    }
    return state
}
