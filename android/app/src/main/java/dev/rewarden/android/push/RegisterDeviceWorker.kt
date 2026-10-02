package dev.rewarden.android.push

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.rewarden.android.RewardenApp
import dev.rewarden.core.CoreException
import kotlin.coroutines.cancellation.CancellationException

/** Re-registers the device after Firebase issued a new token, so pushes keep arriving. */
class RegisterDeviceWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val container = (applicationContext as RewardenApp).container
        return try {
            if (container.core.session() == null) return Result.success()
            container.registerDevice(force = false)
            Result.success()
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            when {
                e is CoreException.Network -> Result.retry()
                e is CoreException.Server && e.status.toInt() >= 500 -> Result.retry()
                else -> Result.failure()
            }
        }
    }

    companion object {
        fun enqueue(context: Context) {
            val request = OneTimeWorkRequestBuilder<RegisterDeviceWorker>()
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(context)
                .enqueueUniqueWork("register-device", ExistingWorkPolicy.REPLACE, request)
        }
    }
}
