package dev.rewarden.android.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.rewarden.android.RewardenApp
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.launch

/** "Stop" on the bypass notification: every bypass ends now. Not exported; only the notification's intent reaches it. */
class BypassStopReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_STOP) return
        val container = (context.applicationContext as RewardenApp).container
        val pending = goAsync()
        container.appScope.launch {
            try {
                container.stopBypasses()
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_STOP = "dev.rewarden.android.STOP_BYPASS"
    }
}

/**
 * Re-reads Autopilot when a bypass is due to end, so the notification and the header follow. The core ends the bypass
 * at its time whatever happens here; this only keeps the phone's display in step.
 */
class BypassEndWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        (applicationContext as RewardenApp).container.refreshAutopilot()
        return Result.success()
    }

    companion object {
        private const val NAME = "autopilot-bypass-end"

        /** Runs just after [until] (unix seconds); null cancels the run. */
        fun schedule(context: Context, until: Long?, nowMillis: Long = System.currentTimeMillis()) {
            val work = WorkManager.getInstance(context)
            if (until == null) {
                work.cancelUniqueWork(NAME)
                return
            }
            val delay = (until * 1000 - nowMillis).coerceAtLeast(0) + 1_500
            val request = OneTimeWorkRequestBuilder<BypassEndWorker>().setInitialDelay(delay, TimeUnit.MILLISECONDS).build()
            work.enqueueUniqueWork(NAME, ExistingWorkPolicy.REPLACE, request)
        }
    }
}
