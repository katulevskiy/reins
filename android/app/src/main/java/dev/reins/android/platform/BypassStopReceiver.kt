package dev.reins.android.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.reins.android.ReinsApp
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.launch

/**
 * "Stop" on the bypass notification (every bypass ends now) or on the Overnight one (every Overnight ends now). Not
 * exported; only the notifications' intents reach it.
 */
class BypassStopReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action
        if (action != ACTION_STOP && action != ACTION_STOP_OVERNIGHT) return
        val container = (context.applicationContext as ReinsApp).container
        val pending = goAsync()
        container.appScope.launch {
            try {
                if (action == ACTION_STOP) container.stopBypasses() else container.stopOvernights()
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_STOP = "dev.reins.android.STOP_BYPASS"
        const val ACTION_STOP_OVERNIGHT = "dev.reins.android.STOP_OVERNIGHT"
    }
}

/**
 * Re-reads Autopilot when a bypass or an Overnight is due to end, so the notifications and the header follow. The core
 * ends them at their time whatever happens here; this only keeps the phone's display in step.
 */
class BypassEndWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        (applicationContext as ReinsApp).container.refreshAutopilot()
        return Result.success()
    }

    companion object {
        private const val NAME = "autopilot-bypass-end"

        /** Runs just after [until] (unix seconds); null cancels the run. */
        fun schedule(context: Context, until: Long?, nowMillis: Long = System.currentTimeMillis()) {
            val work = WorkManager.getInstance(context)
            // A bypass without an end ([AutopilotText.UNTIL_OFF]) has nothing to wait for.
            if (until == null || dev.reins.android.autopilot.AutopilotText.isUntilOff(until)) {
                work.cancelUniqueWork(NAME)
                return
            }
            val delay = (until * 1000 - nowMillis).coerceAtLeast(0) + 1_500
            val request = OneTimeWorkRequestBuilder<BypassEndWorker>().setInitialDelay(delay, TimeUnit.MILLISECONDS).build()
            work.enqueueUniqueWork(NAME, ExistingWorkPolicy.REPLACE, request)
        }
    }
}
