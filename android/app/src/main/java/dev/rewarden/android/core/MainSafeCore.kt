package dev.rewarden.android.core

import dev.rewarden.core.AccountView
import dev.rewarden.core.AutopilotMode
import dev.rewarden.core.AutopilotSettings
import dev.rewarden.core.DownloadProgress
import dev.rewarden.core.ModelRuntime
import dev.rewarden.core.ModelStatus
import dev.rewarden.core.Preset
import dev.rewarden.core.ProfileView
import dev.rewarden.core.SuggestionView
import dev.rewarden.core.Verdict
import dev.rewarden.core.ActivityEntry
import dev.rewarden.core.ApprovalChoice
import dev.rewarden.core.ApprovalKind
import dev.rewarden.core.StandingGrant
import dev.rewarden.core.ApprovalView
import dev.rewarden.core.BlobView
import dev.rewarden.core.ConnectionView
import dev.rewarden.core.EmailContent
import dev.rewarden.core.GmailStatus
import dev.rewarden.core.GrantView
import dev.rewarden.core.LoginProgress
import dev.rewarden.core.McpAddStep
import dev.rewarden.core.McpServerView
import dev.rewarden.core.ServiceView
import dev.rewarden.core.PairingView
import dev.rewarden.core.PendingItem
import dev.rewarden.core.RewardenCoreInterface
import dev.rewarden.core.SessionInfo
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The only core the UI ever sees. UniFFI polls a Rust future on the calling thread, so the synchronous prefix of an
 * `async fn` (Argon2 in `login`, SQLite in `pending`) would run on the main thread if called from `viewModelScope`.
 * Every call therefore hops to [dispatcher] first, and the delegate itself (which opens the database and the
 * Keystore) is only constructed there too.
 */
class MainSafeCore(
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
    create: () -> RewardenCoreInterface,
) : RewardenCoreInterface {
    private val delegate by lazy(create)

    private suspend inline fun <T> io(crossinline block: suspend RewardenCoreInterface.() -> T): T =
        withContext(dispatcher) { delegate.block() }

    override suspend fun activity(limit: UInt): List<ActivityEntry> = io { activity(limit) }

    override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: UByte?, label: String?) =
        io { answerPairing(pairingId, approve, chosenCode, label) }

    override suspend fun approvalView(requestId: String): ApprovalView = io { approvalView(requestId) }

    override suspend fun approve(requestId: String, choice: ApprovalChoice) = io { approve(requestId, choice) }

    override suspend fun connections(): List<ConnectionView> = io { connections() }

    override suspend fun createGrant(connectionId: String, account: String, kind: ApprovalKind, standing: StandingGrant) =
        io { createGrant(connectionId, account, kind, standing) }

    override suspend fun resumeGrant(grantId: String, durationSecs: ULong) = io { resumeGrant(grantId, durationSecs) }

    override suspend fun resumeGrantEdited(grantId: String, standing: StandingGrant) = io { resumeGrantEdited(grantId, standing) }

    override suspend fun fetchEmail(account: String?, messageId: String): EmailContent = io { fetchEmail(account, messageId) }

    override suspend fun deleteGrant(grantId: String) = io { deleteGrant(grantId) }

    override suspend fun accounts(): List<AccountView> = io { accounts() }

    override suspend fun addAccount(hint: String): AccountView = io { addAccount(hint) }

    override suspend fun removeAccount(account: String) = io { removeAccount(account) }

    override suspend fun accountStatus(account: String): GmailStatus = io { accountStatus(account) }

    override suspend fun services(): List<ServiceView> = io { services() }

    override suspend fun addServiceAccount(service: String, hint: String): AccountView = io { addServiceAccount(service, hint) }

    override suspend fun addTokenAccount(service: String, token: String): AccountView = io { addTokenAccount(service, token) }

    override suspend fun loginBegin(service: String, phone: String) = io { loginBegin(service, phone) }

    override suspend fun loginCode(service: String, code: String): LoginProgress = io { loginCode(service, code) }

    override suspend fun loginPassword(service: String, password: String): AccountView = io { loginPassword(service, password) }

    override suspend fun removeServiceAccount(service: String, account: String) = io { removeServiceAccount(service, account) }

    override suspend fun serviceAccountStatus(service: String, account: String): GmailStatus =
        io { serviceAccountStatus(service, account) }

    override suspend fun setConnectionIcon(connectionId: String, icon: String?) = io { setConnectionIcon(connectionId, icon) }

    override suspend fun deny(requestId: String) = io { deny(requestId) }

    override suspend fun gmailStatus(): GmailStatus = io { gmailStatus() }

    override suspend fun grants(): List<GrantView> = io { grants() }

    override suspend fun handlePush(kind: String, id: String) = io { handlePush(kind, id) }

    override suspend fun login(serverUrl: String, email: String, password: String, totp: String?): SessionInfo =
        io { login(serverUrl, email, password, totp) }

    override suspend fun logout() = io { logout() }

    override suspend fun pairingView(pairingId: String): PairingView = io { pairingView(pairingId) }

    override suspend fun pending(): List<PendingItem> = io { pending() }

    override suspend fun registerDevice(fcmToken: String?) = io { registerDevice(fcmToken) }

    override suspend fun revokeConnection(connectionId: String) = io { revokeConnection(connectionId) }

    override suspend fun revokeGrant(grantId: String) = io { revokeGrant(grantId) }

    override suspend fun session(): SessionInfo? = io { session() }

    override suspend fun sync(waitSecs: UInt): List<PendingItem> = io { sync(waitSecs) }

    override suspend fun blobView(id: String): BlobView = io { blobView(id) }

    override suspend fun answerBlob(id: String, approve: Boolean) = io { answerBlob(id, approve) }

    override suspend fun mcpServers(): List<McpServerView> = io { mcpServers() }

    override suspend fun mcpAdd(url: String, name: String?): McpAddStep = io { mcpAdd(url, name) }

    override suspend fun mcpAddWithToken(url: String, token: String, name: String?): McpServerView =
        io { mcpAddWithToken(url, token, name) }

    override suspend fun mcpFinishSignIn(serverId: String, redirectUrl: String): McpServerView =
        io { mcpFinishSignIn(serverId, redirectUrl) }

    override suspend fun mcpRefresh(id: String): McpAddStep = io { mcpRefresh(id) }

    override suspend fun mcpRemove(id: String) = io { mcpRemove(id) }

    override suspend fun mcpSetHeavy(id: String, tool: String, heavy: Boolean) = io { mcpSetHeavy(id, tool, heavy) }

    // ---- Autopilot ----------------------------------------------------------------------------------------------

    /** Not suspending: the runtime is handed over as the core is built (see `AppContainer.createRealCore`). */
    override fun setModelRuntime(runtime: ModelRuntime) = delegate.setModelRuntime(runtime)

    override suspend fun autopilotSettings(): AutopilotSettings = io { autopilotSettings() }

    override suspend fun setAutopilotMode(connectionId: String?, mode: AutopilotMode?, minutes: UInt?) =
        io { setAutopilotMode(connectionId, mode, minutes) }

    override suspend fun setAutopilotWifiOnly(wifiOnly: Boolean) = io { setAutopilotWifiOnly(wifiOnly) }

    override suspend fun autopilotProfiles(): List<ProfileView> = io { autopilotProfiles() }

    override suspend fun createProfile(name: String, icon: String?): ProfileView = io { createProfile(name, icon) }

    override suspend fun renameProfile(profileId: String, name: String, icon: String?) = io { renameProfile(profileId, name, icon) }

    override suspend fun deleteProfile(profileId: String) = io { deleteProfile(profileId) }

    override suspend fun resetProfile(profileId: String) = io { resetProfile(profileId) }

    override suspend fun setDefaultProfile(profileId: String) = io { setDefaultProfile(profileId) }

    override suspend fun assignProfile(connectionId: String, profileId: String?) = io { assignProfile(connectionId, profileId) }

    override suspend fun setClassLock(profileId: String, classKey: String, locked: Boolean?) = io { setClassLock(profileId, classKey, locked) }

    override suspend fun setPreset(profileId: String, preset: Preset) = io { setPreset(profileId, preset) }

    override suspend fun autopilotSuggestion(requestId: String): SuggestionView? = io { autopilotSuggestion(requestId) }

    override suspend fun correctDecision(activityId: Long, shouldHave: Verdict) = io { correctDecision(activityId, shouldHave) }

    override suspend fun modelStatus(): ModelStatus = io { modelStatus() }

    override suspend fun downloadModel(progress: DownloadProgress): ModelStatus = io { downloadModel(progress) }

    override suspend fun deleteModel() = io { deleteModel() }

    override suspend fun autopilotEvaluate(profileId: String?, situation: String): SuggestionView = io { autopilotEvaluate(profileId, situation) }
}
