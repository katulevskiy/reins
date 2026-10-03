package dev.reins.android.push

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.reins.android.ReinsApp
import dev.reins.android.ui.common.userMessage
import dev.reins.core.CoreException
import kotlin.coroutines.cancellation.CancellationException

/** Re-registers the device after Firebase issued a new token, so pushes keep arriving. */
class RegisterDeviceWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val container = (applicationContext as ReinsApp).container
        return try {
            if (container.core.session() == null) return Result.success()
            container.registerDevice(force = false)
            Result.success()
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            when {
                // Final: another phone approves for the account. Retrying cannot help; the app asks the user to take
                // over (approve this phone from the other one, or the recovery code).
                e is CoreException.OtherApprovalDevice -> {
                    container.state.setRegistrationError(e.userMessage())
                    Result.failure()
                }
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
