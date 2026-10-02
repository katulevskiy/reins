package dev.rewarden.android.autopilot

import android.content.Context
import android.content.pm.ServiceInfo
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.rewarden.android.RewardenApp
import dev.rewarden.android.platform.AppNotifier
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.CoreException
import dev.rewarden.core.DownloadProgress
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/** Where the model download stands as far as the job goes (the bytes come from `modelStatus()`). */
enum class DownloadJob {
    /** No job: nothing asked for, or it finished. */
    Idle,

    /** Asked for, waiting for Wi-Fi (or any network). */
    Waiting,

    /** Downloading now. */
    Running,
}

/** Starting, cancelling and watching the download job. An interface so screens and tests need no WorkManager. */
interface ModelDownloads {
    fun start(wifiOnly: Boolean)

    /** Cancels a job that has not started yet (a running download finishes in the core either way). */
    fun cancel()

    val job: Flow<DownloadJob>
}

/** The WorkManager job: Wi-Fi only unless the user allowed mobile data, shown as a progress notification. */
class WorkModelDownloads(private val context: Context) : ModelDownloads {
    override fun start(wifiOnly: Boolean) {
        val network = if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED
        val request = OneTimeWorkRequestBuilder<ModelDownloadWorker>()
            .setConstraints(Constraints.Builder().setRequiredNetworkType(network).setRequiresStorageNotLow(true).build())
            .addTag(NAME)
            .build()
        WorkManager.getInstance(context).enqueueUniqueWork(NAME, ExistingWorkPolicy.KEEP, request)
    }

    override fun cancel() {
        WorkManager.getInstance(context).cancelUniqueWork(NAME)
    }

    override val job: Flow<DownloadJob>
        get() = WorkManager.getInstance(context).getWorkInfosForUniqueWorkFlow(NAME).map { infos ->
            when {
                infos.any { it.state == WorkInfo.State.RUNNING } -> DownloadJob.Running
                infos.any { it.state == WorkInfo.State.ENQUEUED || it.state == WorkInfo.State.BLOCKED } -> DownloadJob.Waiting
                else -> DownloadJob.Idle
            }
        }

    companion object {
        const val NAME = "autopilot-model"
    }
}

/**
 * Downloads the model through the core, which checks every file against the hashes built into it and keeps nothing
 * that does not match. Runs in the foreground (a few hundred megabytes) with a progress notification.
 */
class ModelDownloadWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val container = (applicationContext as RewardenApp).container
        val notifier = container.notifier
        showProgress(notifier, 0, 0)
        val progress = object : DownloadProgress {
            private var lastShown = 0L

            override fun progress(downloaded: ULong, total: ULong) {
                val now = System.currentTimeMillis()
                // The notification is redrawn a few times a second at most.
                if (now - lastShown < 700 && downloaded != total) return
                lastShown = now
                setForegroundAsync(foregroundInfo(notifier, downloaded.toLong(), total.toLong()))
            }
        }
        return try {
            container.core.downloadModel(progress)
            container.refreshAutopilot()
            notifier.downloadFinished(null)
            Result.success()
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            container.refreshAutopilot()
            // A dropped connection is worth another go; a file that fails its check is not.
            if (e is CoreException.Network && runAttemptCount < MAX_ATTEMPTS) {
                Result.retry()
            } else {
                notifier.downloadFinished(AutopilotText.modelError(container.core.modelStatus().error ?: e.userMessage()))
                Result.failure()
            }
        }
    }

    /** Some phones refuse a foreground start from the background; the download then runs as ordinary work. */
    private suspend fun showProgress(notifier: AppNotifier, downloaded: Long, total: Long) {
        try {
            setForeground(foregroundInfo(notifier, downloaded, total))
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
        }
    }

    override suspend fun getForegroundInfo(): ForegroundInfo =
        foregroundInfo((applicationContext as RewardenApp).container.notifier, 0, 0)

    private fun foregroundInfo(notifier: AppNotifier, downloaded: Long, total: Long) =
        ForegroundInfo(AppNotifier.DOWNLOAD_ID, notifier.downloadNotification(downloaded, total), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)

    private companion object {
        const val MAX_ATTEMPTS = 3
    }
}
