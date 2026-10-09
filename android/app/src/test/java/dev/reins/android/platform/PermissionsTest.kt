package dev.reins.android.platform

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * What every build must ask for. Robolectric enforces none of these, so a missing one only shows on a phone: without
 * VIBRATE, `Vibrator.vibrate` throws and every haptic but `performHapticFeedback` is silently dropped.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class PermissionsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Test
    fun `the manifest declares what haptics, notifications and the model download use`() {
        @Suppress("DEPRECATION")
        val info = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
        val requested = info.requestedPermissions.orEmpty().toSet()
        val needed = listOf(
            Manifest.permission.VIBRATE,
            Manifest.permission.POST_NOTIFICATIONS,
            Manifest.permission.INTERNET,
            Manifest.permission.ACCESS_NETWORK_STATE,
            Manifest.permission.RECEIVE_BOOT_COMPLETED,
            Manifest.permission.FOREGROUND_SERVICE,
            Manifest.permission.FOREGROUND_SERVICE_DATA_SYNC,
        )
        assertEquals(emptyList<String>(), needed.filterNot { it in requested })
    }
}
