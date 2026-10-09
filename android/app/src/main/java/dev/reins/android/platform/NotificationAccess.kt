package dev.reins.android.platform

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.edit
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Whether approval requests can reach the user as notifications. Without them an AI waits for a phone that never
 * rings, so the onboarding asks for them and the Activity tab keeps saying so until they are on.
 */
object NotificationAccess {
    /** Android 13 and later ask for the permission at runtime; before that only the app-level switch matters. */
    fun enabled(context: Context): Boolean {
        val granted = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        return granted && NotificationManagerCompat.from(context).areNotificationsEnabled()
    }

    /** This app's notification settings, for when Android no longer shows the permission dialog. */
    fun openSettings(context: Context) {
        val intent = Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
            .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(intent) }.onFailure {
            context.startActivity(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.fromParts("package", context.packageName, null))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }
    }

    /** Whether the dialog was shown once already (Android stops showing it after two refusals). */
    internal fun asked(context: Context): Boolean = prefs(context).getBoolean(KEY_ASKED, false)

    internal fun markAsked(context: Context) = prefs(context).edit { putBoolean(KEY_ASKED, true) }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences("permissions", Context.MODE_PRIVATE)

    private const val KEY_ASKED = "notifications.asked"
}

/** Notification access as the UI sees it: re-read every time the app comes back to the front. */
@Stable
class NotificationState internal constructor(private val onRequest: () -> Unit) {
    var enabled by mutableStateOf(true)
        internal set

    /** Shows Android's dialog, or this app's notification settings once Android has stopped showing it. */
    fun request() = onRequest()
}

@Composable
fun rememberNotificationState(): NotificationState {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val holder = remember { arrayOfNulls<NotificationState>(1) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        holder[0]?.enabled = granted && NotificationAccess.enabled(context)
        // Refused again: the dialog will not come back, so the next tap goes to Settings.
        if (!granted) NotificationAccess.markAsked(context)
    }
    val state = remember {
        NotificationState {
            val activity = context.findActivity()
            val canAsk = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED &&
                (!NotificationAccess.asked(context) || activity?.shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS) == true)
            if (canAsk) {
                NotificationAccess.markAsked(context)
                launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
            } else {
                NotificationAccess.openSettings(context)
            }
        }.also { holder[0] = it }
    }
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            state.enabled = withContext(Dispatchers.Default) { NotificationAccess.enabled(context) }
        }
    }
    return state
}

private tailrec fun Context.findActivity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.findActivity()
    else -> null
}
