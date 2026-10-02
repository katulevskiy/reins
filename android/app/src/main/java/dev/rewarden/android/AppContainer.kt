package dev.rewarden.android

import android.content.Context
import dev.rewarden.android.autopilot.AutopilotText
import dev.rewarden.android.autopilot.ModelDownloads
import dev.rewarden.android.autopilot.OnnxModelRuntime
import dev.rewarden.android.autopilot.WorkModelDownloads
import dev.rewarden.android.core.CoreProvider
import dev.rewarden.android.core.MainSafeCore
import dev.rewarden.android.feedback.AndroidFeedback
import dev.rewarden.android.feedback.FeedbackStore
import dev.rewarden.android.platform.AppNotifier
import dev.rewarden.android.platform.BypassEndWorker
import dev.rewarden.android.platform.GrantReminders
import dev.rewarden.android.platform.FirebaseSupport
import dev.rewarden.android.platform.GoogleAuthorizer
import dev.rewarden.android.platform.KeystoreKeyWrapper
import dev.rewarden.android.platform.McpSignIn
import dev.rewarden.android.platform.PhoneBridge
import dev.rewarden.android.platform.Foreground
import dev.rewarden.android.platform.update.HttpUpdateFetcher
import dev.rewarden.android.platform.update.PlatformInstaller
import dev.rewarden.android.platform.update.PrefsUpdateStore
import dev.rewarden.android.platform.update.UpdateController
import dev.rewarden.android.platform.update.UpdateNotifier
import dev.rewarden.android.platform.update.UpdateProvider
import dev.rewarden.android.platform.update.Updater
import dev.rewarden.android.state.AppState
import dev.rewarden.android.state.DeviceStatusStore
import dev.rewarden.android.state.OnboardingStore
import dev.rewarden.android.state.SessionState
import dev.rewarden.core.AutopilotSettings
import dev.rewarden.core.CoreException
import dev.rewarden.core.RewardenCore
import dev.rewarden.core.RewardenCoreInterface
import dev.rewarden.core.SessionInfo
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
    val core: RewardenCoreInterface = CoreProvider.factory.create(this) ?: MainSafeCore { createRealCore() }

    /** The MCP server sign-in that waits for its browser page to come back. */
    val mcpSignIn = McpSignIn(context, { core }, state)

    private fun createRealCore(): RewardenCoreInterface {
        val dataDir = java.io.File(context.noBackupFilesDir, "core").apply { mkdirs() }
        return RewardenCore(
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
            withContext(Dispatchers.IO) {
                state.setSeenActivityId(deviceStatus.seenActivityId())
                state.setApprovalDevice(deviceStatus.isApprovalDevice() && !deviceStatus.isReplaced())
                // Before the session flips, so the main screen does not flash up ahead of the setup.
                state.setSetupPending(onboarding.isPending(info))
            }
        }
        state.setSession(if (info == null) SessionState.SignedOut else SessionState.SignedIn(info))
        if (info != null) {
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

    /** A sign-in or a new account from the onboarding screens: the setup after it is to be shown (once per account). */
    suspend fun beginOnboarding(info: SessionInfo) {
        withContext(Dispatchers.IO) { onboarding.begin(info) }
    }

    /** The setup after signing in was finished or skipped: the main screen from now on. */
    suspend fun finishOnboarding() {
        val info = (state.session.value as? SessionState.SignedIn)?.info
        if (info != null) withContext(Dispatchers.IO) { onboarding.finish(info) }
        state.setSetupPending(false)
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
     * explicit "register this phone"; background token refreshes never take the role back from a replacing phone.
     */
    suspend fun registerDevice(force: Boolean) {
        val replaced = withContext(Dispatchers.IO) { deviceStatus.isReplaced() }
        if (replaced && !force) return
        val token = FirebaseSupport.token(context)
        core.registerDevice(token)
        withContext(Dispatchers.IO) {
            deviceStatus.setReplaced(false)
            deviceStatus.setApprovalDevice(true)
        }
        state.setDeviceReplaced(false)
        state.setApprovalDevice(true)
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
