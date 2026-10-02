package dev.rewarden.android.push

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.Constraints
import dev.rewarden.android.RewardenApp

/** Fetches what the push announced and lets the core process it (parks it, notifies the user). */
class PushWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val payload = PushPayload.parse(
            mapOf("t" to (inputData.getString(KEY_KIND) ?: ""), "id" to (inputData.getString(KEY_ID) ?: "")),
        ) ?: return Result.failure()
        val container = (applicationContext as RewardenApp).container
        val handler = PushHandler(container.core) {
            container.markReplaced()
            container.notifier.deviceReplaced()
        }
        val outcome = handler.handle(payload)
        container.refreshPending()
        return when (outcome) {
            PushOutcome.DONE -> Result.success()
            PushOutcome.RETRY -> if (runAttemptCount < MAX_ATTEMPTS) Result.retry() else Result.failure()
        }
    }

    companion object {
        private const val KEY_KIND = "kind"
        private const val KEY_ID = "id"
        private const val MAX_ATTEMPTS = 4

        fun enqueue(context: Context, payload: PushPayload, highPriority: Boolean) {
            val request = OneTimeWorkRequestBuilder<PushWorker>()
                .setInputData(Data.Builder().putString(KEY_KIND, payload.kind).putString(KEY_ID, payload.id).build())
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .apply {
                    if (highPriority) setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                }
                .build()
            WorkManager.getInstance(context)
                .enqueueUniqueWork("push-${payload.kind}-${payload.id}", ExistingWorkPolicy.KEEP, request)
        }
    }
}
