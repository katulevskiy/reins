package dev.reins.android.autopilot

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.PowerManager
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import dev.reins.android.ReinsApp
import dev.reins.core.CoreException
import dev.reins.core.DownloadProgress
import dev.reins.core.ModelUpdateOutcome
import dev.reins.core.ReinsCoreInterface
import java.util.concurrent.TimeUnit
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.first

/**
 * When a newer model may be fetched. Automatic updates wait for Wi-Fi (any unmetered network) and stay off Battery
 * Saver and Data Saver; "Download on Wi-Fi only" does not loosen that. The user's "Check now" only needs the network
 * the first download would use.
 */
object ModelUpdateGate {
    data class Conditions(
        val online: Boolean,
        val unmetered: Boolean,
        val powerSave: Boolean,
        val dataSaver: Boolean,
    )

    /** An automatic update may download now. */
    fun allows(c: Conditions): Boolean = c.online && c.unmetered && !c.powerSave && !c.dataSaver

    /** "Check now" may download now ([wifiOnly]: the "Download on Wi-Fi only" switch). */
    fun checkNowAllows(c: Conditions, wifiOnly: Boolean): Boolean = c.online && (c.unmetered || !wifiOnly)
}

/** The phone's network and power state as the gate reads it. Tests set [override]. */
object NetworkConditions {
    @Volatile var override: (() -> ModelUpdateGate.Conditions)? = null

    fun read(context: Context): ModelUpdateGate.Conditions {
        override?.let { return it() }
        val connectivity = context.getSystemService(ConnectivityManager::class.java)
        val caps = connectivity?.activeNetwork?.let { connectivity.getNetworkCapabilities(it) }
        return ModelUpdateGate.Conditions(
            online = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true,
            unmetered = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) == true,
            powerSave = context.getSystemService(PowerManager::class.java)?.isPowerSaveMode == true,
            dataSaver = connectivity?.restrictBackgroundStatus == ConnectivityManager.RESTRICT_BACKGROUND_STATUS_ENABLED,
        )
    }
}

/** What a background run asks of WorkManager. */
enum class UpdateRun {
    /** Not now (the network, the power, the core): WorkManager tries again later. */
    Retry,

    /** Nothing left to do, or nothing worth retrying (the model in use stays either way). */
    Done,
}

/** Progress nobody watches: the core's model status shows how far an update is. */
internal object NoProgress : DownloadProgress {
    override fun progress(downloaded: ULong, total: ULong) = Unit
}

/**
 * One background run of the model update, without WorkManager so it can be tested. [manual] is a "Check now" that
 * waited for Wi-Fi: it runs even with "Update automatically" off and retries a model that failed before.
 * [downloadRunning]: the first download's own job is running. [refresh] re-reads Autopilot after the core answered.
 */
suspend fun runModelUpdate(
    core: ReinsCoreInterface,
    conditions: ModelUpdateGate.Conditions,
    downloadRunning: Boolean,
    manual: Boolean = false,
    refresh: suspend () -> Unit = {},
): UpdateRun {
    try {
        // Opens the account in a process started for this run; signed out, there is no model to update.
        if (core.session() == null) return UpdateRun.Done
        if (!manual && !core.modelUpdatePending()) return UpdateRun.Done
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
        return UpdateRun.Done
    }
    val allowed = if (manual) ModelUpdateGate.checkNowAllows(conditions, wifiOnly = true) else ModelUpdateGate.allows(conditions)
    if (!allowed || downloadRunning) return UpdateRun.Retry
    return try {
        core.updateModel(automatic = !manual, progress = NoProgress)
        refresh()
        UpdateRun.Done
    } catch (e: CancellationException) {
        throw e
    } catch (e: CoreException) {
        refresh()
        // A dropped connection or a busy host: what arrived is kept and the next run goes on from it. A model that
        // failed its check is remembered by the core, which leaves it alone until "Check now".
        if (e is CoreException.Network || e is CoreException.Server) UpdateRun.Retry else UpdateRun.Done
    } catch (_: Exception) {
        UpdateRun.Done
    }
}

/** Starting the background update runs. An interface so screens and tests need no WorkManager. */
interface ModelUpdates {
    /** At launch: one run as soon as Wi-Fi allows, and one a day after that. Leaves existing ones alone. */
    fun schedule()

    /** "Check now" without Wi-Fi: a run that checks as soon as Wi-Fi is there, queued after any waiting one. */
    fun checkOnWifi()
}

class WorkModelUpdates(private val context: Context) : ModelUpdates {
    override fun schedule() {
        val work = WorkManager.getInstance(context)
        work.enqueueUniqueWork(NOW, ExistingWorkPolicy.KEEP, oneTime(manual = false))
        val daily = PeriodicWorkRequestBuilder<ModelUpdateWorker>(24, TimeUnit.HOURS)
            .setConstraints(constraints())
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, BACKOFF_MINUTES, TimeUnit.MINUTES)
            .build()
        work.enqueueUniquePeriodicWork(DAILY, ExistingPeriodicWorkPolicy.KEEP, daily)
    }

    override fun checkOnWifi() {
        WorkManager.getInstance(context).enqueueUniqueWork(NOW, ExistingWorkPolicy.APPEND_OR_REPLACE, oneTime(manual = true))
    }

    private fun oneTime(manual: Boolean) = OneTimeWorkRequestBuilder<ModelUpdateWorker>()
        .setConstraints(constraints())
        .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, BACKOFF_MINUTES, TimeUnit.MINUTES)
        .setInputData(workDataOf(ModelUpdateWorker.MANUAL to manual))
        .build()

    private fun constraints() = Constraints.Builder()
        .setRequiredNetworkType(NetworkType.UNMETERED)
        .setRequiresBatteryNotLow(true)
        .setRequiresStorageNotLow(true)
        .build()

    companion object {
        const val NOW = "autopilot-model-update"
        const val DAILY = "autopilot-model-update-daily"
        private const val BACKOFF_MINUTES = 10L
    }
}

/**
 * Updates the model in the background, quietly: no notification, no sound, no prompt. On any failure the model in use
 * stays; the core picks an interrupted download up where it stopped.
 */
class ModelUpdateWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val container = (applicationContext as ReinsApp).container
        val running = try {
            WorkManager.getInstance(applicationContext).getWorkInfosForUniqueWorkFlow(WorkModelDownloads.NAME).first()
                .any { it.state == WorkInfo.State.RUNNING }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            false
        }
        val run = runModelUpdate(
            core = container.core,
            conditions = NetworkConditions.read(applicationContext),
            downloadRunning = running,
            manual = inputData.getBoolean(MANUAL, false),
            refresh = { container.refreshAutopilot() },
        )
        return if (run == UpdateRun.Retry) Result.retry() else Result.success()
    }

    companion object {
        const val MANUAL = "manual"
    }
}

/** What "Check now" says under its button. */
object CheckNow {
    const val WAITING = "Waiting for Wi-Fi"

    fun outcome(o: ModelUpdateOutcome): String? = when (o) {
        ModelUpdateOutcome.UP_TO_DATE -> "Up to date"
        ModelUpdateOutcome.UPDATED -> "Updated"
        ModelUpdateOutcome.UNAVAILABLE -> "Not available yet"
        ModelUpdateOutcome.NOT_INSTALLED -> "Not downloaded"
        ModelUpdateOutcome.OFF, ModelUpdateOutcome.REJECTED -> null
    }
}
