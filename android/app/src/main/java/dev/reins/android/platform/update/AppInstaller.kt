package dev.reins.android.platform.update

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import dev.reins.android.ReinsApp
import java.io.File

/** Installs a downloaded, verified APK over the running app. */
interface AppInstaller {
    /** Whether the user allows Reins to install apps ("install unknown apps"). */
    fun canInstall(): Boolean

    /** Opens Android's "install unknown apps" setting for Reins. */
    fun openPermissionSettings(context: Context)

    /** Hands [file] to the system installer; the outcome arrives at [InstallResultReceiver]. */
    fun install(file: File)
}

/**
 * [AppInstaller] on the [PackageInstaller] session API. Android checks that the APK is signed with the same key as the
 * installed app and has a higher versionCode; it asks the user to confirm unless it may update silently.
 */
class PlatformInstaller(context: Context) : AppInstaller {
    private val context = context.applicationContext

    override fun canInstall(): Boolean = context.packageManager.canRequestPackageInstalls()

    override fun openPermissionSettings(context: Context) {
        val intent = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }

    override fun install(file: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(file.length())
            setInstallReason(PackageManager.INSTALL_REASON_USER)
            // Android still asks unless this app may update itself silently.
            setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        }
        val id = installer.createSession(params)
        try {
            installer.openSession(id).use { session ->
                session.openWrite("base.apk", 0, file.length()).use { out ->
                    file.inputStream().use { it.copyTo(out, 256 * 1024) }
                    session.fsync(out)
                }
                session.commit(statusReceiver().intentSender)
            }
        } catch (e: Exception) {
            installer.abandonSession(id)
            throw e
        }
    }

    /** Explicit, so it may be mutable: the installer fills in the status extras. */
    private fun statusReceiver(): PendingIntent = PendingIntent.getBroadcast(
        context,
        REQUEST_CODE,
        Intent(context, InstallResultReceiver::class.java).setAction(InstallResultReceiver.ACTION),
        PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private companion object {
        const val REQUEST_CODE = 0x5550
    }
}

/**
 * Install outcomes from [PackageInstaller]. Not exported: only the system (through this app's pending intent) reaches
 * it. When Android needs the user to confirm, its confirmation screen is started here; the app is in front then,
 * because installing only starts from a tap in the app.
 */
class InstallResultReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            val prompt = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
            } else {
                @Suppress("DEPRECATION") intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
            }
            prompt?.let { confirm ->
                context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        } else if (status != PackageInstaller.STATUS_SUCCESS) {
            Log.w("ReinsUpdate", "install failed: $status $message")
        }
        (context.applicationContext as ReinsApp).container.updates?.onInstallResult(status, message)
    }

    companion object {
        const val ACTION = "dev.reins.android.INSTALL_STATUS"
    }
}
