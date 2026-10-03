package dev.rewarden.android.platform.update

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.rewarden.android.RewardenApp
import java.util.concurrent.TimeUnit

/** Looks for a new release every few hours while online, downloads it if allowed and announces it once. */
class UpdateWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        // Failures are quiet: the next run tries again.
        (applicationContext as RewardenApp).container.updates?.backgroundCheck()
        return Result.success()
    }

    companion object {
        private const val NAME = "app-update"

        /** Keeps one periodic check; calling it again leaves the existing schedule alone. */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<UpdateWorker>(6, TimeUnit.HOURS)
                .setConstraints(
                    Constraints.Builder()
                        .setRequiredNetworkType(NetworkType.CONNECTED)
                        .setRequiresStorageNotLow(true)
                        .build(),
                )
                .build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }
    }
}
