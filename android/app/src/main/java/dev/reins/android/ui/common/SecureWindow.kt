package dev.rewarden.android.ui.common

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.view.WindowManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext

/**
 * FLAG_SECURE (no screenshots, hidden in recents) while a sensitive screen is shown. Reference-counted per activity,
 * so moving from one secure screen to another never drops the flag in between.
 */
object SecureFlag {
    private val counts = java.util.WeakHashMap<Activity, Int>()

    fun acquire(activity: Activity) {
        val count = (counts[activity] ?: 0) + 1
        counts[activity] = count
        if (count == 1) activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    fun release(activity: Activity) {
        val count = (counts[activity] ?: 0) - 1
        if (count <= 0) {
            counts.remove(activity)
            activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        } else {
            counts[activity] = count
        }
    }
}

@Composable
fun SecureWindow() {
    val activity = LocalContext.current.findActivity() ?: return
    if (!dev.rewarden.android.BuildConfig.SECURE_SCREENS) return
    DisposableEffect(activity) {
        SecureFlag.acquire(activity)
        onDispose { SecureFlag.release(activity) }
    }
}

fun Context.findActivity(): Activity? {
    var context = this
    while (context is ContextWrapper) {
        if (context is Activity) return context
        context = context.baseContext
    }
    return null
}
