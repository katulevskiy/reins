package dev.reins.android.core

import dev.reins.core.AccountKeys
import dev.reins.core.BudgetView
import dev.reins.core.PaymentsOverview
import dev.reins.core.PurchaseChoice
import dev.reins.core.SpendLimitInput
import dev.reins.core.SpendLimitView
import dev.reins.core.SpendingView
import dev.reins.core.AccountView
import dev.reins.core.StartingPolicy
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.DownloadProgress
import dev.reins.core.ModelRuntime
import dev.reins.core.ModelStatus
import dev.reins.core.Preset
import dev.reins.core.ProfileView
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict
import dev.reins.core.ActivityEntry
import dev.reins.core.ApprovalChoice
import dev.reins.core.ApprovalKind
import dev.reins.core.StandingGrant
import dev.reins.core.ApprovalView
import dev.reins.core.BlobView
import dev.reins.core.ConnectionView
import dev.reins.core.EmailContent
import dev.reins.core.GmailStatus
import dev.reins.core.GrantView
import dev.reins.core.JoinProgress
import dev.reins.core.JoinStart
import dev.reins.core.JoinView
import dev.reins.core.LoginProgress
import dev.reins.core.McpAddStep
import dev.reins.core.McpServerView
import dev.reins.core.ServiceView
import dev.reins.core.PairingView
import dev.reins.core.PendingItem
import dev.reins.core.ReinsCoreInterface
import dev.reins.core.ServerInfo
import dev.reins.core.SessionInfo
import dev.reins.core.SsoOutcome
import dev.reins.core.SsoStart
import dev.reins.core.VaultItemDetail
import dev.reins.core.VaultItemInput
import dev.reins.core.VaultItemSummary
import dev.reins.core.VaultPasskeyOptions
import dev.reins.core.VaultPasskeyView
import dev.reins.core.VaultSshKey
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
    create: () -> ReinsCoreInterface,
) : ReinsCoreInterface {
    private val delegate by lazy(create)

    private suspend inline fun <T> io(crossinline block: suspend ReinsCoreInterface.() -> T): T =
        withContext(dispatcher) { delegate.block() }

    override suspend fun activity(limit: UInt): List<ActivityEntry> = io { activity(limit) }

    override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: UByte?, label: String?) =
        io { answerPairing(pairingId, approve, chosenCode, label) }

    override suspend fun approvalView(requestId: String): ApprovalView = io { approvalView(requestId) }

    override suspend fun approve(requestId: String, choice: ApprovalChoice) = io { approve(requestId, choice) }

    override suspend fun approveQuick(requestId: String) = io { approveQuick(requestId) }

    override suspend fun startingPolicy(): StartingPolicy? = io { startingPolicy() }

    override suspend fun setStartingPolicy(policy: StartingPolicy) = io { setStartingPolicy(policy) }

    override suspend fun connections(): List<ConnectionView> = io { connections() }

    override suspend fun vaultItems(query: String): List<VaultItemSummary> = io { vaultItems(query) }

    override suspend fun vaultItem(id: String): VaultItemDetail = io { vaultItem(id) }

    override suspend fun vaultReveal(id: String, key: String): String = io { vaultReveal(id, key) }

    override suspend fun vaultCreate(input: VaultItemInput): String = io { vaultCreate(input) }

    override suspend fun vaultUpdate(id: String, input: VaultItemInput) = io { vaultUpdate(id, input) }

    override suspend fun vaultDelete(id: String) = io { vaultDelete(id) }

    override suspend fun vaultGenerateSshKey(name: String): VaultSshKey = io { vaultGenerateSshKey(name) }

    override suspend fun phoneKeyFingerprint(): String = io { phoneKeyFingerprint() }

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

    override suspend fun handlePushDeferringAutopilot(kind: String, id: String) = io { handlePushDeferringAutopilot(kind, id) }

    override suspend fun login(serverUrl: String, email: String, password: String, totp: String?): SessionInfo =
        io { login(serverUrl, email, password, totp) }

    override suspend fun createAccount(serverUrl: String, email: String, password: String): SessionInfo =
        io { createAccount(serverUrl, email, password) }

    override suspend fun logout() = io { logout() }
    override suspend fun logoutWithBrowser(): String? = io { logoutWithBrowser() }
    override suspend fun deleteAccount(confirmEmail: String) = io { deleteAccount(confirmEmail) }

    override suspend fun ssoBegin(serverUrl: String): SsoStart = io { ssoBegin(serverUrl) }
    override suspend fun serverInfo(serverUrl: String): ServerInfo = io { serverInfo(serverUrl) }

    override suspend fun ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String): SsoOutcome =
        io { ssoFinish(serverUrl, callbackUrl, state, verifier) }

    override suspend fun resetAccount(serverUrl: String, callbackUrl: String, state: String, verifier: String): SsoOutcome =
        io { resetAccount(serverUrl, callbackUrl, state, verifier) }

    override suspend fun accountKeys(): AccountKeys = io { accountKeys() }

    override suspend fun unlockAccount(codeOrPassword: String) = io { unlockAccount(codeOrPassword) }

    override suspend fun accountRecoveryCode(): String = io { accountRecoveryCode() }

    override suspend fun vaultPasskeyOptions(): VaultPasskeyOptions = io { vaultPasskeyOptions() }

    override suspend fun vaultPasskeys(): List<VaultPasskeyView> = io { vaultPasskeys() }

    override suspend fun addVaultPasskey(credentialId: ByteArray, prfOutput: ByteArray, name: String): List<VaultPasskeyView> =
        io { addVaultPasskey(credentialId, prfOutput, name) }

    override suspend fun removeVaultPasskey(credentialId: ByteArray): List<VaultPasskeyView> = io { removeVaultPasskey(credentialId) }

    override suspend fun unlockWithVaultPasskey(credentialId: ByteArray, prfOutput: ByteArray) =
        io { unlockWithVaultPasskey(credentialId, prfOutput) }

    override suspend fun joinBegin(deviceName: String): JoinStart = io { joinBegin(deviceName) }

    override suspend fun joinPoll(): JoinProgress = io { joinPoll() }

    override suspend fun joinCancel() = io { joinCancel() }

    override suspend fun joinView(id: String): JoinView = io { joinView(id) }

    override suspend fun answerJoin(id: String, approve: Boolean) = io { answerJoin(id, approve) }

    override suspend fun pairingView(pairingId: String): PairingView = io { pairingView(pairingId) }

    override suspend fun pairingByCode(userCode: String): PairingView = io { pairingByCode(userCode) }

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

    // ---- Payments ---------------------------------------------------------------------------------------------

    override suspend fun approvePurchase(requestId: String, choice: PurchaseChoice) = io { approvePurchase(requestId, choice) }

    override suspend fun paymentsOverview(): PaymentsOverview = io { paymentsOverview() }

    override suspend fun paymentsSetMethod(methodId: String, enabled: Boolean) = io { paymentsSetMethod(methodId, enabled) }

    override suspend fun paymentsSetNickname(methodId: String, nickname: String?) = io { paymentsSetNickname(methodId, nickname) }

    override suspend fun paymentsSetDefaults(methodId: String?, addressId: String?) = io { paymentsSetDefaults(methodId, addressId) }

    override suspend fun paymentsConnectProvider(kind: String, apiKey: String, sandbox: Boolean, singleUse: Boolean) =
        io { paymentsConnectProvider(kind, apiKey, sandbox, singleUse) }

    override suspend fun paymentsDisconnectProvider() = io { paymentsDisconnectProvider() }

    override suspend fun paymentsSetCardOptions(tolerancePct: UInt, singleUse: Boolean) = io { paymentsSetCardOptions(tolerancePct, singleUse) }

    override suspend fun paymentsSetBudget(budget: BudgetView) = io { paymentsSetBudget(budget) }

    override suspend fun paymentsAddLimit(limit: SpendLimitInput): SpendLimitView = io { paymentsAddLimit(limit) }

    override suspend fun paymentsRemoveLimit(limitId: String) = io { paymentsRemoveLimit(limitId) }

    override suspend fun paymentsSpending(since: Long): SpendingView = io { paymentsSpending(since) }

    override suspend fun paymentsAcknowledgeCharge(purchaseId: String) = io { paymentsAcknowledgeCharge(purchaseId) }

    override suspend fun paymentsCloseCard(purchaseId: String) = io { paymentsCloseCard(purchaseId) }

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
