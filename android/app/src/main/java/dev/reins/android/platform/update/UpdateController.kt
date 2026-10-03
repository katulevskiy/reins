package dev.reins.android.platform.update

import android.content.Context
import android.content.pm.PackageInstaller
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Where the update stands, as Settings and the prompt show it. */
sealed interface UpdateStatus {
    val release: Release? get() = null

    data object Idle : UpdateStatus
    data object Checking : UpdateStatus
    data object UpToDate : UpdateStatus

    /** Newer than this app, not downloaded (automatic downloads are off). */
    data class Available(override val release: Release) : UpdateStatus
    data class Downloading(override val release: Release, val percent: Int) : UpdateStatus

    /** Downloaded and verified: can be installed. */
    data class Ready(override val release: Release) : UpdateStatus
    data class Installing(override val release: Release) : UpdateStatus

    /** [message] is for the user; [release] is set when the failure concerns a known release (download, install). */
    data class Failed(val message: String, override val release: Release? = null) : UpdateStatus
}

data class UpdateState(
    val status: UpdateStatus = UpdateStatus.Idle,
    val autoDownload: Boolean = true,
    /** The user said "Later" to this release less than a day ago. */
    val snoozed: Boolean = false,
    /** The user asked to install: the prompt follows the download and the install through. */
    val installRequested: Boolean = false,
    /** Android needs "install unknown apps" for Reins first; the app explains before opening that setting. */
    val askPermission: Boolean = false,
) {
    /** Whether the in-app prompt is up (the screen still decides where, and never over an approval sheet). */
    val showPrompt: Boolean
        get() = when (status) {
            is UpdateStatus.Available, is UpdateStatus.Ready -> !snoozed || installRequested
            is UpdateStatus.Downloading, is UpdateStatus.Installing -> installRequested
            is UpdateStatus.Failed -> installRequested && status.release != null
            else -> false
        }
}

/** Announces a release outside the app (a notification). [ready]: downloaded, else only available. */
fun interface ReleaseNotifier {
    fun announce(release: Release, ready: Boolean)
}

/**
 * The app's update flow: checks (by hand, on foreground at most every three hours, and from [UpdateWorker]), the
 * download, the prompt, and installation through [AppInstaller]. One instance per process, in the app container;
 * everything that touches the disk or the network runs on [io], one operation at a time.
 */
class UpdateController(
    private val updater: Updater,
    private val installer: () -> AppInstaller,
    private val notifier: ReleaseNotifier,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    /** Whether the app is in front; announcements are skipped then because the prompt shows instead. */
    private val inForeground: () -> Boolean = { false },
) {
    private val _state = MutableStateFlow(UpdateState())
    val state: StateFlow<UpdateState> = _state.asStateFlow()

    private val lock = Mutex()
    private var job: Job? = null

    /** Set while the user is in Android's "install unknown apps" screen. */
    @Volatile private var awaitingPermission = false

    /** "Install" was tapped before the download finished: install as soon as it is ready. */
    @Volatile private var installWhenReady = false

    private val status get() = _state.value.status
    private val busy get() = job?.isActive == true

    /** Reads the saved settings and offers a download from an earlier run. Does not check. */
    fun start() {
        launch { restore() }
    }

    private suspend fun restore() = lock.withLock { restoreLocked() }

    private fun restoreLocked() {
        val ready = updater.downloaded()
        _state.update {
            it.copy(
                autoDownload = updater.autoDownload,
                status = if (ready != null && it.status == UpdateStatus.Idle) UpdateStatus.Ready(ready) else it.status,
                snoozed = ready != null && updater.isSnoozed(ready),
            )
        }
    }

    /** "Check for updates" in Settings. */
    fun checkNow() {
        if (busy) return
        job = launch { check(quiet = false, download = updater.autoDownload) }
    }

    /** The app came to the front: finish an install that waited for the permission, else check if one is due. */
    fun onForeground() {
        if (awaitingPermission) {
            awaitingPermission = false
            if (installer().canInstall()) {
                install()
            } else {
                status.release?.let { release ->
                    setStatus(UpdateStatus.Failed("Reins can't install updates until you allow it to install apps.", release))
                }
            }
            return
        }
        // Android's install confirmation pauses and resumes the app; that is no reason to check.
        if (busy || status is UpdateStatus.Installing) return
        job = launch { if (updater.dueForCheck()) check(quiet = true, download = updater.autoDownload) }
    }

    /** The periodic background check: downloads if allowed, then announces each new version once. */
    suspend fun backgroundCheck() {
        val release = withContext(io) { lock.withLock { runCheck(quiet = true, download = updater.autoDownload) } } ?: return
        val ready = status is UpdateStatus.Ready
        withContext(io) {
            if (!updater.shouldNotify(release) || updater.isSnoozed(release) || inForeground()) return@withContext
            notifier.announce(release, ready)
            updater.markNotified(release)
        }
    }

    /** "Try again" after a failure. */
    fun retry() {
        val failed = status as? UpdateStatus.Failed ?: return
        val release = failed.release
        when {
            release == null -> checkNow()
            _state.value.installRequested -> install()
            else -> download(release)
        }
    }

    fun setAutoDownload(on: Boolean) {
        _state.update { it.copy(autoDownload = on) }
        launch {
            updater.autoDownload = on
        }
        val available = status as? UpdateStatus.Available
        if (on && available != null) download(available.release)
    }

    /** "Later": the prompt stays away from this release for a day. */
    fun later() {
        val release = status.release ?: return
        _state.update { it.copy(snoozed = true, installRequested = false) }
        launch {
            lock.withLock {
                updater.snooze(release)
                _state.update { it.copy(snoozed = true) }
            }
        }
    }

    /** The user tapped the update notification: show the prompt even if it was snoozed. */
    fun openFromNotification() {
        _state.update { it.copy(snoozed = false) }
        launch {
            lock.withLock {
                updater.unsnooze()
                _state.update { it.copy(snoozed = false) }
                if (status == UpdateStatus.Idle) restoreLocked()
            }
        }
    }

    /** "Install": downloads first if needed, asks for the install permission if needed, then starts the installer. */
    fun install() {
        _state.update { it.copy(installRequested = true) }
        when (val s = status) {
            is UpdateStatus.Ready -> startInstall(s.release)
            is UpdateStatus.Available -> {
                installWhenReady = true
                download(s.release)
            }
            is UpdateStatus.Failed -> s.release?.let {
                installWhenReady = true
                download(it)
            }
            UpdateStatus.Checking, is UpdateStatus.Downloading -> installWhenReady = true
            else -> Unit
        }
    }

    /** The user agreed to the explanation: open Android's setting for this app and continue when they come back. */
    fun openPermissionSettings(context: Context) {
        _state.update { it.copy(askPermission = false) }
        awaitingPermission = true
        installer().openPermissionSettings(context)
    }

    fun dismissPermission() {
        _state.update { it.copy(askPermission = false, installRequested = false) }
    }

    /** What the package installer reported (see [InstallResultReceiver]). */
    fun onInstallResult(status: Int, message: String?) {
        val release = this.status.release ?: return
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION, PackageInstaller.STATUS_SUCCESS -> Unit
            PackageInstaller.STATUS_FAILURE_ABORTED -> {
                _state.update { it.copy(installRequested = false) }
                setStatus(UpdateStatus.Ready(release))
            }
            else -> {
                if (status == PackageInstaller.STATUS_FAILURE_INVALID) launch { lock.withLock { updater.discard() } }
                setStatus(UpdateStatus.Failed(installFailureMessage(status, message)!!, release))
            }
        }
    }

    private fun download(release: Release) {
        if (busy) return
        job = launch { fetch(release) }
    }

    private fun startInstall(release: Release) {
        if (!installer().canInstall()) {
            _state.update { it.copy(askPermission = true) }
            return
        }
        setStatus(UpdateStatus.Installing(release))
        job = launch {
            lock.withLock {
                val file = updater.verified(release)
                if (file == null) {
                    setStatus(UpdateStatus.Failed(UpdateException.Corrupt("changed on disk").message!!, release))
                    return@withLock
                }
                try {
                    installer().install(file)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    setStatus(UpdateStatus.Failed("Couldn't start installing the update. Try again.", release))
                }
            }
        }
    }

    /** One check; [quiet] checks (automatic ones) leave the status as it was when they fail. */
    private suspend fun check(quiet: Boolean, download: Boolean) {
        lock.withLock { runCheck(quiet, download) }
    }

    /** Returns the newer release if there is one and it is usable (ready, or available without automatic downloads). */
    private fun runCheck(quiet: Boolean, download: Boolean): Release? {
        val before = status
        if (!quiet) setStatus(UpdateStatus.Checking)
        val release = try {
            updater.latest()
        } catch (e: UpdateException) {
            // An automatic check that fails says nothing; the next one may work.
            setStatus(if (quiet) before else UpdateStatus.Failed(e.message!!))
            return null
        }
        if (release == null) {
            setStatus(UpdateStatus.UpToDate)
            return null
        }
        _state.update { it.copy(snoozed = updater.isSnoozed(release)) }
        if (updater.downloaded() == release) {
            ready(release)
            return release
        }
        if (!download) {
            setStatus(UpdateStatus.Available(release))
            return release
        }
        return if (runDownload(release, quiet)) release else null
    }

    private suspend fun fetch(release: Release) {
        lock.withLock { runDownload(release, quiet = false) }
    }

    private fun runDownload(release: Release, quiet: Boolean): Boolean {
        setStatus(UpdateStatus.Downloading(release, 0))
        try {
            updater.download(release) { bytes ->
                val percent = (bytes * 100 / release.size).toInt()
                if ((status as? UpdateStatus.Downloading)?.percent != percent) setStatus(UpdateStatus.Downloading(release, percent))
            }
        } catch (e: UpdateException) {
            installWhenReady = false
            setStatus(if (quiet && !_state.value.installRequested) UpdateStatus.Available(release) else UpdateStatus.Failed(e.message!!, release))
            return false
        }
        ready(release)
        return true
    }

    private fun ready(release: Release) {
        setStatus(UpdateStatus.Ready(release))
        if (installWhenReady) {
            installWhenReady = false
            scope.launch { startInstall(release) }
        }
    }

    private fun setStatus(status: UpdateStatus) {
        _state.update { it.copy(status = status) }
    }

    private fun launch(block: suspend () -> Unit): Job = scope.launch {
        try {
            withContext(io) { block() }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            setStatus(UpdateStatus.Failed(e.message ?: "The update failed. Try again.", status.release))
        }
    }
}

/** What to tell the user when the package installer gives up; null when there is nothing to say (they cancelled). */
fun installFailureMessage(status: Int, message: String?): String? = when (status) {
    PackageInstaller.STATUS_SUCCESS, PackageInstaller.STATUS_PENDING_USER_ACTION, PackageInstaller.STATUS_FAILURE_ABORTED -> null
    PackageInstaller.STATUS_FAILURE_CONFLICT ->
        "Android refused the update: it is signed with a different key than the Reins installed on this phone."
    PackageInstaller.STATUS_FAILURE_INCOMPATIBLE -> "This update can't be installed on this phone."
    PackageInstaller.STATUS_FAILURE_STORAGE -> "Not enough free space to install the update."
    PackageInstaller.STATUS_FAILURE_INVALID -> "The downloaded file is not a valid app and was deleted."
    PackageInstaller.STATUS_FAILURE_BLOCKED -> "Installing was blocked on this phone (by a device policy or another app)."
    else -> "The update could not be installed" + (message?.takeIf { it.isNotBlank() }?.let { ": $it" } ?: ".")
}
