package dev.reins.android

import android.content.Context
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.ModelDownloads
import dev.reins.android.autopilot.OnnxModelRuntime
import dev.reins.android.autopilot.WorkModelDownloads
import dev.reins.android.core.CoreProvider
import dev.reins.android.core.MainSafeCore
import dev.reins.android.feedback.AndroidFeedback
import dev.reins.android.feedback.FeedbackStore
import dev.reins.android.platform.AppNotifier
import dev.reins.android.platform.BypassEndWorker
import dev.reins.android.platform.GrantReminders
import dev.reins.android.platform.FirebaseSupport
import dev.reins.android.platform.GoogleAuthorizer
import dev.reins.android.platform.KeystoreKeyWrapper
import dev.reins.android.platform.McpSignIn
import dev.reins.android.platform.PhoneBridge
import dev.reins.android.platform.SsoSignIn
import dev.reins.android.platform.Foreground
import dev.reins.android.platform.update.HttpUpdateFetcher
import dev.reins.android.platform.update.PlatformInstaller
import dev.reins.android.platform.update.PrefsUpdateStore
import dev.reins.android.platform.update.UpdateController
import dev.reins.android.platform.update.UpdateNotifier
import dev.reins.android.platform.update.UpdateProvider
import dev.reins.android.platform.update.Updater
import dev.reins.android.state.AppState
import dev.reins.android.state.DeviceStatusStore
import dev.reins.android.state.OnboardingStore
import dev.reins.android.state.RecoveryRecord
import dev.reins.android.state.SessionState
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AutopilotSettings
import dev.reins.core.CoreException
import dev.reins.core.ReinsCore
import dev.reins.core.ReinsCoreInterface
import dev.reins.core.SessionInfo
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Process singletons. No DI framework: the app has one of everything. */
class AppContainer(private val context: Context) {
    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val state = AppState()
    val google = GoogleAuthorizer(context)

    /** The Sounds & haptics switches; the notification channels follow them too. */
    val feedbackStore: FeedbackStore by lazy { FeedbackStore(context) }

    /** Sounds and haptics. Compose reaches it through `LocalFeedback`, view models through here. */
    val feedback: AndroidFeedback by lazy { AndroidFeedback(context, feedbackStore) }

    val notifier = AppNotifier(
        context,
        settings = { feedbackStore.current },
        feedback = { feedback },
        onAutopilot = { appScope.launch { refreshAutopilot() } },
    ) { appScope.launch { refreshPending() } }

    /** Runs Autopilot's model on this phone; handed to the core as it is built, so background pushes can use it. */
    val modelRuntime: OnnxModelRuntime by lazy { OnnxModelRuntime() }

    /** The model download job. */
    val modelDownloads: ModelDownloads by lazy { WorkModelDownloads(context) }
    val deviceStatus = DeviceStatusStore(context)

    /** Whether the setup after a fresh sign-in is still to be shown, per account. */
    val onboarding = OnboardingStore(context)
    val recoveryRecord = RecoveryRecord(context)
    val phone = PhoneBridge(context)

    /**
     * In-app updates from [BuildConfig.UPDATE_URL]; downloads live in the cache, out of backups. Null in the `play` build
     * ([BuildConfig.SELF_UPDATE] is false): Google Play installs its updates, and Play's policy forbids an app updating
     * itself.
     */
    val updates: UpdateController? = if (!BuildConfig.SELF_UPDATE) null else run {
        val http = HttpUpdateFetcher(BuildConfig.UPDATE_URL)
        val platformInstaller = PlatformInstaller(context)
        UpdateController(
            Updater(
                fetcher = { UpdateProvider.fetcher ?: http },
                dir = java.io.File(context.cacheDir, "updates"),
                store = PrefsUpdateStore(context),
                installed = BuildConfig.VERSION_CODE.toLong(),
            ),
            installer = { UpdateProvider.installer ?: platformInstaller },
            notifier = UpdateNotifier(context),
            scope = appScope,
            inForeground = { Foreground.focused },
        ).also { it.start() }
    }

    /** Every call runs on `Dispatchers.IO`; the Rust core is built there too. */
    val core: ReinsCoreInterface = CoreProvider.factory.create(this) ?: MainSafeCore { createRealCore() }

    /** The MCP server sign-in that waits for its browser page to come back. */
    val mcpSignIn = McpSignIn(context, { core }, state)

    /** The sign-in through the server's SSO ("Continue") that waits for its browser page to come back. */
    val ssoSignIn = SsoSignIn(context)

    private fun createRealCore(): ReinsCoreInterface {
        val dataDir = java.io.File(context.noBackupFilesDir, "core").apply { mkdirs() }
        return ReinsCore(
            dataDir.absolutePath,
            KeystoreKeyWrapper(),
            google,
            notifier,
            phone,
            BuildConfig.TELEGRAM_API_ID,
            BuildConfig.TELEGRAM_API_HASH,
        ).also { it.setModelRuntime(modelRuntime) }
    }

    suspend fun refreshSession() {
        val info = try {
            core.session()
        } catch (e: CoreException) {
            null
        }
        if (info != null) {
            // Compute the gate before publishing SignedIn, including after a process restart. Password accounts
            // have no account secret and keep their self-hosted onboarding.
            var recoveryFailure: String? = null
            val code = try {
                core.accountRecoveryCode()
            } catch (e: CoreException.Invalid) {
                null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // An older install may need its first token refresh to learn the account id. Retry visibly rather
                // than crashing startup or silently bypassing the mandatory recovery step while offline.
                recoveryFailure = e.userMessage()
                null
            }
            val needsRecording = code != null && withContext(Dispatchers.IO) { !recoveryRecord.confirmed(info.serverUrl, code) }
            state.setRecoveryToRecord(if (needsRecording) code else null)
            state.setRecoveryLoadError(recoveryFailure)
            withContext(Dispatchers.IO) {
                state.setSeenActivityId(deviceStatus.seenActivityId())
                state.setApprovalDevice(deviceStatus.isApprovalDevice() && !deviceStatus.isReplaced())
                // Before the session flips, so the main screen does not flash up ahead of the setup or the Unlock screen.
                state.setSetupPending(onboarding.isPending(info))
                state.setKeysLocked(deviceStatus.keysLocked())
                state.setApprovalTakeover(deviceStatus.needsTakeover())
            }
        }
        state.setSession(if (info == null) SessionState.SignedOut else SessionState.SignedIn(info))
        if (info != null && !state.keysLocked.value) {
            refreshPending()
            refreshConnections()
        }
    }

    /** Re-reads everything held on the phone: what waits, what happened, which permissions exist. */
    suspend fun refreshPending() {
        try {
            state.setPending(core.pending())
            state.setActivity(core.activity(ACTIVITY_LIMIT))
            val grants = core.grants()
            state.setGrants(grants)
            state.setAccounts(core.accounts())
            state.setServices(core.services())
            state.setMcpServers(core.mcpServers())
            val info = core.session()
            if (info != null && state.session.value is SessionState.SignedIn) {
                state.setSession(SessionState.SignedIn(info))
            }
            refreshAutopilot()
            withContext(Dispatchers.IO) { GrantReminders.sync(context, grants) }
        } catch (e: CoreException) {
            if (e is CoreException.NotLoggedIn) state.setSession(SessionState.SignedOut)
        }
        withContext(Dispatchers.IO) { state.setDeviceReplaced(deviceStatus.isReplaced()) }
    }

    /**
     * Re-reads Autopilot (modes, bypasses, model) and keeps what shows it in step: the header pill, the ongoing bypass
     * notification and the job that refreshes all this when a bypass ends.
     */
    suspend fun refreshAutopilot(): AutopilotSettings? {
        val settings = try {
            core.autopilotSettings()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            return state.autopilot.value
        }
        state.setAutopilot(settings)
        val labels = state.connections.value.associate { it.id to it.label }
        val notice = AutopilotText.bypassNotice(settings, { labels[it] ?: "an AI" }, System.currentTimeMillis() / 1000)
        withContext(Dispatchers.IO) {
            notifier.showBypass(notice)
            BypassEndWorker.schedule(context, notice?.until)
        }
        return settings
    }

    /** Ends every bypass now (the bypass notification's Stop): each goes back to the mode it interrupted. */
    suspend fun stopBypasses() {
        val settings = refreshAutopilot() ?: return
        val now = System.currentTimeMillis() / 1000
        try {
            if ((settings.bypassUntil ?: 0) > now) core.setAutopilotMode(null, settings.baseMode, null)
            settings.connections.filter { (it.bypassUntil ?: 0) > now }.forEach { core.setAutopilotMode(it.connectionId, it.baseMode, null) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Whatever could not be stopped still ends at its time; the refresh below shows what is left.
        }
        refreshAutopilot()
    }

    /**
     * A sign-in, a new account, or "Continue" with the account's keys open on this phone: the setup after it is to be
     * shown (once per account), and this phone becomes the approval device. A failed registration is kept in
     * [AppState.registrationError] (Settings offers it again); the sign-in itself stands.
     */
    suspend fun finishSignIn(info: SessionInfo) {
        setKeysLocked(false)
        beginOnboarding(info)
        state.setRegistrationError(null)
        refreshSession()
        try {
            registerDevice(force = true)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            state.setRegistrationError(e.userMessage())
        }
    }

    /**
     * "Continue" signed in to an account whose keys this phone cannot open yet ([locked]), or they were just opened.
     * Kept on disk, so a relaunch comes back to the Unlock screen instead of registering this phone.
     */
    suspend fun setKeysLocked(locked: Boolean) {
        withContext(Dispatchers.IO) { deviceStatus.setKeysLocked(locked) }
        state.setKeysLocked(locked)
    }

    /** A sign-in or a new account from the onboarding screens: the setup after it is to be shown (once per account). */
    suspend fun beginOnboarding(info: SessionInfo) {
        withContext(Dispatchers.IO) { onboarding.begin(info) }
    }

    /** The setup after signing in was finished or skipped: the main screen from now on. */
    suspend fun finishOnboarding() {
        if (state.recoveryToRecord.value != null) return
        val info = (state.session.value as? SessionState.SignedIn)?.info
        if (info != null) withContext(Dispatchers.IO) { onboarding.finish(info) }
        state.setSetupPending(false)
    }

    /** Acknowledgement alone lives on disk; never copy the recovery code into preferences or saved UI state. */
    suspend fun confirmRecoveryRecord() {
        val info = (state.session.value as? SessionState.SignedIn)?.info ?: return
        val code = state.recoveryToRecord.value ?: return
        val saved = withContext(Dispatchers.IO) { recoveryRecord.confirm(info.serverUrl, code) }
        if (saved && (state.session.value as? SessionState.SignedIn)?.info == info && state.recoveryToRecord.value == code) {
            state.setRecoveryToRecord(null)
        }
    }

    /** The AI connections (a network call; failures keep the last list). */
    suspend fun refreshConnections() {
        try {
            state.setConnections(core.connections())
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // Offline or signed out: icons fall back to names, nothing else depends on this.
        }
    }

    /** The user looked at the activity list up to [id]. */
    suspend fun markActivitySeen(id: Long) {
        if (id <= state.seenActivityId.value) return
        state.setSeenActivityId(id)
        withContext(Dispatchers.IO) { deviceStatus.setSeenActivityId(id) }
    }

    /**
     * Makes this phone the approval device (with an FCM token when Firebase is configured). [force] is for the user's
     * explicit "register this phone"; background token refreshes never take the role back from a replacing phone. A
     * phone whose account keys are still locked is never registered: the phone that has them must approve it first.
     * When another phone approves for the account and this one brings no proof, the server refuses
     * ([CoreException.OtherApprovalDevice], rethrown): the Unlock screen then offers the two ways to take over.
     */
    suspend fun registerDevice(force: Boolean) {
        val (replaced, locked) = withContext(Dispatchers.IO) { deviceStatus.isReplaced() to deviceStatus.keysLocked() }
        if (locked || (replaced && !force)) return
        val token = FirebaseSupport.token(context)
        try {
            core.registerDevice(token)
        } catch (e: CoreException.OtherApprovalDevice) {
            setApprovalTakeover(true)
            throw e
        }
        withContext(Dispatchers.IO) {
            deviceStatus.setReplaced(false)
            deviceStatus.setApprovalDevice(true)
            deviceStatus.setNeedsTakeover(false)
        }
        state.setDeviceReplaced(false)
        state.setApprovalDevice(true)
        state.setApprovalTakeover(false)
    }

    /**
     * Whether the Unlock screen offers taking the approval role over from the other phone ([needed]), or the user put
     * it off ("Not now"; Settings offers it again). Kept on disk, so a relaunch comes back to it.
     */
    suspend fun setApprovalTakeover(needed: Boolean) {
        withContext(Dispatchers.IO) { deviceStatus.setNeedsTakeover(needed) }
        state.setApprovalTakeover(needed)
    }

    /** The server says another phone is the approval device now. */
    suspend fun markReplaced() {
        withContext(Dispatchers.IO) { deviceStatus.setReplaced(true) }
        state.setDeviceReplaced(true)
        state.setApprovalDevice(false)
    }

    /** Forgets what belongs to the signed-in account. */
    suspend fun forgetAccount() {
        withContext(Dispatchers.IO) { deviceStatus.clear() }
        state.setApprovalDevice(false)
        state.setDeviceReplaced(false)
        state.setKeysLocked(false)
        state.setApprovalTakeover(false)
    }

    private companion object {
        const val ACTIVITY_LIMIT = 300u
    }

    /** Runs [block] and lets cancellation through; every other failure is reported to [onError]. */
    inline fun <T> guard(onError: (Throwable) -> Unit, block: () -> T): T? = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: Throwable) {
        onError(e)
        null
    }
}
