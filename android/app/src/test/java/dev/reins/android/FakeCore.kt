package dev.reins.android

import dev.reins.core.AccountKeys
import dev.reins.core.AccountView
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot
import dev.reins.core.DownloadProgress
import dev.reins.core.ModelRuntime
import dev.reins.core.ModelState
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
import dev.reins.core.CoreException
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
import dev.reins.core.SessionInfo
import dev.reins.core.SsoOutcome
import dev.reins.core.SsoStart
import dev.reins.core.VaultPasskeyOptions
import dev.reins.core.VaultPasskeyView
import java.util.concurrent.CopyOnWriteArrayList

/** In-memory core for UI tests: holds state, records the calls that matter. */
class FakeCore : ReinsCoreInterface {
    @Volatile var session: SessionInfo? = null
    @Volatile var loginError: CoreException? = null
    @Volatile var pending: List<PendingItem> = emptyList()
    @Volatile var approval: ApprovalView? = null
    @Volatile var pairing: PairingView? = null
    @Volatile var grants: List<GrantView> = emptyList()
    @Volatile var connections: List<ConnectionView> = emptyList()
    @Volatile var activity: List<ActivityEntry> = emptyList()
    @Volatile var gmail: GmailStatus = GmailStatus.NeedsConsent

    val approvals = CopyOnWriteArrayList<Pair<String, ApprovalChoice>>()
    val denials = CopyOnWriteArrayList<String>()
    val pairingAnswers = CopyOnWriteArrayList<List<Any?>>()
    val revokedGrants = CopyOnWriteArrayList<String>()
    val logins = CopyOnWriteArrayList<List<String?>>()
    val registrations = CopyOnWriteArrayList<String?>()

    /** Thrown by `createAccount` (an email already registered, a weak password, sign-ups closed). */
    @Volatile var createAccountError: CoreException? = null
    val createdAccounts = CopyOnWriteArrayList<List<String>>()

    /** Thrown by `pairingByCode` (an unknown or expired code: `NotFound`). */
    @Volatile var pairingByCodeError: CoreException? = null
    /** What `pairingByCode` parks and returns; null: [TestData.pairingView] of the desktop app. */
    @Volatile var pairingByCodeResult: PairingView? = null
    val pairingCodes = CopyOnWriteArrayList<String>()

    fun resetOnboarding() {
        createAccountError = null
        createdAccounts.clear()
        pairingByCodeError = null
        pairingByCodeResult = null
        pairingCodes.clear()
    }

    override suspend fun activity(limit: UInt) = activity

    override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: UByte?, label: String?) {
        pairingAnswers += listOf(pairingId, approve, chosenCode, label)
        pending = pending.filterNot { it.id == pairingId }
    }

    override suspend fun approvalView(requestId: String) = approval ?: throw CoreException.NotFound()

    override suspend fun approve(requestId: String, choice: ApprovalChoice) {
        approvals += requestId to choice
        pending = pending.filterNot { it.id == requestId }
    }

    /** Requests approved without being opened (a notification's Approve, "Approve all"). */
    val quickApprovals = CopyOnWriteArrayList<String>()

    override suspend fun approveQuick(requestId: String) {
        val item = pending.firstOrNull { it.id == requestId } ?: throw CoreException.NotFound()
        if (!item.quick) throw CoreException.Invalid("Open this request to decide.")
        quickApprovals += requestId
        pending = pending.filterNot { it.id == requestId }
    }

    override suspend fun connections() = connections

    val createdGrants = CopyOnWriteArrayList<Triple<String, ApprovalKind, StandingGrant>>()
    /** The account each created grant was made for. */
    val createdGrantAccounts = CopyOnWriteArrayList<String>()
    val resumed = CopyOnWriteArrayList<Pair<String, ULong>>()
    @Volatile var accounts: List<AccountView> = emptyList()
    val addedAccounts = CopyOnWriteArrayList<String>()
    val removedAccounts = CopyOnWriteArrayList<String>()
    @Volatile var accountStatuses: Map<String, GmailStatus> = emptyMap()

    val resumedEdited = CopyOnWriteArrayList<Pair<String, StandingGrant>>()

    override suspend fun resumeGrantEdited(grantId: String, standing: StandingGrant) {
        resumedEdited += grantId to standing
        grants = grants.map {
            if (it.id == grantId) it.copy(active = true, state = "active", uses = 0u, maxUses = standing.maxUses, expiresAt = 1_700_000_000L + (standing.durationSecs?.toLong() ?: 0L)) else it
        }
    }

    /** What opening an email returns, by message id; anything else is "no longer in Gmail". */
    val emails = java.util.concurrent.ConcurrentHashMap<String, EmailContent>()
    val openedEmails = CopyOnWriteArrayList<Pair<String?, String>>()

    override suspend fun fetchEmail(account: String?, messageId: String): EmailContent {
        openedEmails += account to messageId
        return emails[messageId] ?: throw CoreException.Gmail("that email is no longer in Gmail")
    }

    val deletedGrants = CopyOnWriteArrayList<String>()

    override suspend fun deleteGrant(grantId: String) {
        deletedGrants += grantId
        grants = grants.filterNot { it.id == grantId }
    }

    override suspend fun accounts() = accounts

    override suspend fun addAccount(hint: String): AccountView {
        addedAccounts += hint
        return AccountView("gmail", hint.lowercase(), 1_700_000_000L).also { accounts = accounts + it }
    }

    override suspend fun removeAccount(account: String) {
        removedAccounts += account
        accounts = accounts.filterNot { it.account == account }
        grants = grants.filterNot { it.account == account }
    }

    /** The catalogue the Integrations screen shows; tests replace it. */
    @Volatile var catalogue: List<ServiceView> = defaultServices()
    val serviceAdded = CopyOnWriteArrayList<Pair<String, String>>()
    val tokensAdded = CopyOnWriteArrayList<Pair<String, String>>()
    val serviceRemoved = CopyOnWriteArrayList<Pair<String, String>>()
    val loginCalls = CopyOnWriteArrayList<List<String>>()
    /** Codes the fake Telegram accepts; a code of "22222" leads on to a password prompt. */
    @Volatile var telegramPhone: String = ""
    @Volatile var serviceFailure: CoreException? = null
    @Volatile var serviceStatuses: Map<String, GmailStatus> = emptyMap()

    private fun withAccount(service: String, account: String): AccountView {
        val view = AccountView(service, account, 1_700_000_000L)
        accounts = accounts + view
        return view
    }

    override suspend fun services() = catalogue.map { s -> s.copy(accounts = accounts.filter { it.service == s.service }) }

    override suspend fun addServiceAccount(service: String, hint: String): AccountView {
        serviceFailure?.let { throw it }
        serviceAdded += service to hint
        return withAccount(service, hint.ifEmpty { "this phone" })
    }

    override suspend fun addTokenAccount(service: String, token: String): AccountView {
        serviceFailure?.let { throw it }
        tokensAdded += service to token
        return withAccount(service, if (service == "vault") "me@example.com" else "octo-cat")
    }

    override suspend fun loginBegin(service: String, phone: String) {
        serviceFailure?.let { throw it }
        loginCalls += listOf("begin", phone)
        telegramPhone = phone
    }

    override suspend fun loginCode(service: String, code: String): LoginProgress {
        loginCalls += listOf("code", code)
        if (code == "00000") throw CoreException.Invalid("That code is wrong or has expired. Ask for a new one.")
        if (code == "22222") return LoginProgress.NeedsPassword("pet")
        withAccount(service, telegramPhone)
        return LoginProgress.Done(telegramPhone)
    }

    override suspend fun loginPassword(service: String, password: String): AccountView {
        loginCalls += listOf("password", password)
        return withAccount(service, telegramPhone)
    }

    override suspend fun removeServiceAccount(service: String, account: String) {
        serviceRemoved += service to account
        accounts = accounts.filterNot { it.service == service && it.account == account }
    }

    override suspend fun serviceAccountStatus(service: String, account: String) =
        serviceStatuses["$service/$account"] ?: GmailStatus.Ready

    override suspend fun accountStatus(account: String) = accountStatuses[account] ?: GmailStatus.Ready

    override suspend fun resumeGrant(grantId: String, durationSecs: ULong) {
        resumed += grantId to durationSecs
        grants = grants.map {
            if (it.id == grantId) it.copy(active = true, state = "active", uses = 0u, expiresAt = 1_700_000_000L + durationSecs.toLong()) else it
        }
    }
    /** Last choice per connection; "auto" when cleared. */
    val icons = java.util.concurrent.ConcurrentHashMap<String, String>()

    override suspend fun createGrant(connectionId: String, account: String, kind: ApprovalKind, standing: StandingGrant) {
        createdGrants += Triple(connectionId, kind, standing)
        createdGrantAccounts += account
    }

    override suspend fun setConnectionIcon(connectionId: String, icon: String?) {
        icons[connectionId] = icon ?: "auto"
        connections = connections.map { if (it.id == connectionId) it.copy(icon = icon) else it }
    }

    override suspend fun deny(requestId: String) {
        denials += requestId
        pending = pending.filterNot { it.id == requestId }
    }

    override suspend fun gmailStatus() = gmail

    override suspend fun grants() = grants

    override suspend fun handlePush(kind: String, id: String) = Unit

    override suspend fun handlePushDeferringAutopilot(kind: String, id: String) = Unit

    override suspend fun login(serverUrl: String, email: String, password: String, totp: String?): SessionInfo {
        logins += listOf(serverUrl, email, password, totp)
        loginError?.let { throw it }
        return SessionInfo(serverUrl, email).also { session = it }
    }

    override suspend fun createAccount(serverUrl: String, email: String, password: String): SessionInfo {
        createdAccounts += listOf(serverUrl, email, password)
        createAccountError?.let { throw it }
        return SessionInfo(serverUrl, email).also { session = it }
    }

    override suspend fun logout() {
        session = null
    }
    @Volatile var browserLogoutUrl: String? = null
    override suspend fun logoutWithBrowser(): String? {
        logout()
        return browserLogoutUrl
    }

    /** The emails `deleteAccount` deleted the account with; [deleteAccountError] is thrown instead when set. */
    val deletedAccounts = java.util.concurrent.CopyOnWriteArrayList<String>()
    @Volatile var deleteAccountError: Exception? = null
    override suspend fun deleteAccount(confirmEmail: String) {
        deleteAccountError?.let { throw it }
        deletedAccounts += confirmEmail
        session = null
    }

    // ---- passwordless sign-in and "Add another phone" ------------------------------------------------------------

    /** What "Continue" finds: a new account (keys made silently), one this phone opens, or one locked on another phone. */
    @Volatile var ssoKeys: AccountKeys = AccountKeys.CREATED
    @Volatile var ssoEmail: String = "me@example.com"
    /** Thrown by `ssoBegin` / `ssoFinish`. */
    @Volatile var ssoBeginError: CoreException? = null
    @Volatile var ssoFinishError: CoreException? = null
    val ssoBegins = CopyOnWriteArrayList<String>()
    /** Server, callback, state, verifier of each `ssoFinish`. */
    val ssoFinishes = CopyOnWriteArrayList<List<String>>()
    /** Thrown by `resetAccount`; server, callback, state, verifier of each. */
    @Volatile var resetError: CoreException? = null
    val resets = CopyOnWriteArrayList<List<String>>()

    /** The signed-in account's keys on this phone, and its recovery code (null: an account made with a master password). */
    @Volatile var keys: AccountKeys = AccountKeys.UNLOCKED
    @Volatile var recoveryCode: String? = null
    val unlockAttempts = CopyOnWriteArrayList<String>()
    val recoveryCodeReads = java.util.concurrent.atomic.AtomicInteger()
    val syncStarts = java.util.concurrent.atomic.AtomicInteger()

    /** `joinPoll` answers `Waiting` this many times, then [joinAnswer]. */
    @Volatile var joinWaits = 2
    @Volatile var joinAnswer: JoinProgress = JoinProgress.JOINED
    @Volatile var joinBeginError: CoreException? = null
    val joinBegins = CopyOnWriteArrayList<String>()
    val joinPolls = java.util.concurrent.atomic.AtomicInteger()
    val joinCancels = java.util.concurrent.atomic.AtomicInteger()

    /** Phones that asked this one (the approval device) for the keys, by request id. */
    val joins = java.util.concurrent.ConcurrentHashMap<String, JoinView>()
    val joinAnswers = CopyOnWriteArrayList<Pair<String, Boolean>>()

    fun resetSso() {
        browserLogoutUrl = null
        ssoKeys = AccountKeys.CREATED
        ssoEmail = "me@example.com"
        ssoBeginError = null
        ssoFinishError = null
        ssoBegins.clear()
        ssoFinishes.clear()
        resetError = null
        resets.clear()
        keys = AccountKeys.UNLOCKED
        recoveryCode = null
        unlockAttempts.clear()
        recoveryCodeReads.set(0)
        syncStarts.set(0)
        joinWaits = 2
        joinAnswer = JoinProgress.JOINED
        joinBeginError = null
        joinBegins.clear()
        joinPolls.set(0)
        joinCancels.set(0)
        joins.clear()
        joinAnswers.clear()
        otherApprovalDevice = false
        takeoverProof = false
        refusedRegistrations.set(0)
        resetVaultPasskeys()
    }

    override suspend fun ssoBegin(serverUrl: String): SsoStart {
        ssoBegins += serverUrl
        ssoBeginError?.let { throw it }
        return SsoStart("$serverUrl/identity/connect/authorize?state=$SSO_STATE", "com.reins2fa.app", SSO_STATE, SSO_VERIFIER)
    }

    /** Like the core: the callback must answer the sign-in that sent `state`. */
    override suspend fun ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String): SsoOutcome {
        ssoFinishes += listOf(serverUrl, callbackUrl, state, verifier)
        ssoFinishError?.let { throw it }
        if ("state=$state" !in callbackUrl) throw CoreException.Invalid("The sign-in did not come back as expected. Try again.")
        val info = SessionInfo(serverUrl, ssoEmail)
        session = info
        keys = ssoKeys
        recoveryCode = if (ssoKeys == AccountKeys.LOCKED) null else RECOVERY_CODE
        return SsoOutcome(info, ssoKeys)
    }

    /** Like the core: the signed-in account's vault starts over with new keys and a new recovery code. */
    override suspend fun resetAccount(serverUrl: String, callbackUrl: String, state: String, verifier: String): SsoOutcome {
        resets += listOf(serverUrl, callbackUrl, state, verifier)
        resetError?.let { throw it }
        if ("state=$state" !in callbackUrl) throw CoreException.Invalid("The sign-in did not come back as expected. Try again.")
        val info = session ?: throw CoreException.NotLoggedIn()
        keys = AccountKeys.CREATED
        recoveryCode = RESET_RECOVERY_CODE
        return SsoOutcome(info, AccountKeys.CREATED)
    }

    override suspend fun accountKeys(): AccountKeys = keys

    override suspend fun unlockAccount(codeOrPassword: String) {
        unlockAttempts += codeOrPassword
        val normalized = codeOrPassword.filter { it.isLetterOrDigit() }.uppercase()
        if (normalized != RECOVERY_CODE.filter { it.isLetterOrDigit() } && codeOrPassword != "correct horse battery staple") {
            throw CoreException.Invalid("That is neither the recovery code nor the master password.")
        }
        keys = AccountKeys.UNLOCKED
        recoveryCode = RECOVERY_CODE
        takeoverProof = true
    }

    override suspend fun accountRecoveryCode(): String {
        recoveryCodeReads.incrementAndGet()
        return recoveryCode ?: throw CoreException.Invalid("This account has no recovery code: it was made with a master password.")
    }

    // ---- passkeys that open the vault -----------------------------------------------------------------------------

    /**
     * The account's vault passkeys. By default it has one already ([existingPasskey]), so the sign-ins of the other
     * tests go straight to the recovery code; the passkey tests start from none.
     */
    @Volatile var vaultPasskeys: List<VaultPasskeyView> = listOf(existingPasskey())
    /** Thrown by `vaultPasskeys` and `vaultPasskeyOptions` (offline: `Network`). */
    @Volatile var vaultPasskeysError: CoreException? = null
    /** Name, credential id and PRF output of each `addVaultPasskey`. */
    val passkeyAdds = CopyOnWriteArrayList<Triple<String, ByteArray, ByteArray>>()
    val passkeyRemovals = CopyOnWriteArrayList<ByteArray>()
    val passkeyUnlocks = CopyOnWriteArrayList<ByteArray>()

    fun resetVaultPasskeys() {
        vaultPasskeys = listOf(existingPasskey())
        vaultPasskeysError = null
        passkeyAdds.clear()
        passkeyRemovals.clear()
        passkeyUnlocks.clear()
    }

    override suspend fun vaultPasskeyOptions(): VaultPasskeyOptions {
        vaultPasskeysError?.let { throw it }
        val email = session?.email ?: throw CoreException.NotLoggedIn()
        return VaultPasskeyOptions(
            "127.0.0.1", "user-1".toByteArray(), email, ByteArray(32) { 7 }, ByteArray(32) { 1 }, vaultPasskeys.map { it.credentialId },
        )
    }

    override suspend fun vaultPasskeys(): List<VaultPasskeyView> {
        vaultPasskeysError?.let { throw it }
        return vaultPasskeys
    }

    /** Like the core: only where the vault is open (this phone keeps the account secret). */
    override suspend fun addVaultPasskey(credentialId: ByteArray, prfOutput: ByteArray, name: String): List<VaultPasskeyView> {
        if (recoveryCode == null) throw CoreException.Invalid("Open the vault on this phone first: a passkey can only be added where the vault is open.")
        if (prfOutput.size != 32) throw CoreException.Invalid("This passkey did not give the secret Reins needs. Try another passkey.")
        passkeyAdds += Triple(name, credentialId, prfOutput)
        vaultPasskeys = vaultPasskeys + VaultPasskeyView(credentialId, name, 1_700_000_050)
        return vaultPasskeys
    }

    override suspend fun removeVaultPasskey(credentialId: ByteArray): List<VaultPasskeyView> {
        passkeyRemovals += credentialId
        vaultPasskeys = vaultPasskeys.filterNot { it.credentialId.contentEquals(credentialId) }
        return vaultPasskeys
    }

    /** Opens the vault like the recovery code when the passkey is one of the account's and gives [PASSKEY_PRF]. */
    override suspend fun unlockWithVaultPasskey(credentialId: ByteArray, prfOutput: ByteArray) {
        passkeyUnlocks += credentialId
        if (vaultPasskeys.none { it.credentialId.contentEquals(credentialId) }) {
            throw CoreException.Invalid("This passkey was not added to open this account's vault.")
        }
        if (!prfOutput.contentEquals(PASSKEY_PRF)) throw CoreException.Invalid("This passkey does not open this account's vault.")
        keys = AccountKeys.UNLOCKED
        recoveryCode = RECOVERY_CODE
        takeoverProof = true
    }

    override suspend fun joinBegin(deviceName: String): JoinStart {
        joinBegins += deviceName
        joinBeginError?.let { throw it }
        joinPolls.set(0)
        return JoinStart("join-1", "482 193", 1_700_000_700)
    }

    override suspend fun joinPoll(): JoinProgress {
        if (joinPolls.incrementAndGet() <= joinWaits) return JoinProgress.WAITING
        if (joinAnswer == JoinProgress.JOINED) {
            keys = AccountKeys.UNLOCKED
            recoveryCode = RECOVERY_CODE
            takeoverProof = true
        }
        return joinAnswer
    }

    override suspend fun joinCancel() {
        joinCancels.incrementAndGet()
    }

    override suspend fun joinView(id: String): JoinView = joins[id] ?: throw CoreException.NotFound()

    override suspend fun answerJoin(id: String, approve: Boolean) {
        joinAnswers += id to approve
        joins.remove(id)
        pending = pending.filterNot { it.id == id }
    }

    override suspend fun pairingView(pairingId: String) = pairing ?: throw CoreException.NotFound()

    /** Parks the pairing the code stands for, like a pushed one: it is then pending and has a view. */
    override suspend fun pairingByCode(userCode: String): PairingView {
        pairingCodes += userCode
        pairingByCodeError?.let { throw it }
        val view = pairingByCodeResult ?: TestData.pairingView("pair-code", "Reins desktop app on laptop", "laptop", keyFingerprint = "4821 9930")
        pairing = view
        pending = pending.filterNot { it.id == view.id } + TestData.pairingItem(view.id)
        return view
    }

    override suspend fun pending() = pending

    /**
     * Another phone approves for the account: `registerDevice` is refused ([CoreException.OtherApprovalDevice]) until
     * this phone brings a proof, from `unlockAccount` or a join the other phone approved.
     */
    @Volatile var otherApprovalDevice = false
    @Volatile var takeoverProof = false
    val refusedRegistrations = java.util.concurrent.atomic.AtomicInteger()

    override suspend fun registerDevice(fcmToken: String?) {
        if (otherApprovalDevice && !takeoverProof) {
            refusedRegistrations.incrementAndGet()
            throw CoreException.OtherApprovalDevice()
        }
        registrations += fcmToken
    }

    val revokedConnections = CopyOnWriteArrayList<String>()

    override suspend fun revokeConnection(connectionId: String) {
        revokedConnections += connectionId
        connections = connections.filterNot { it.id == connectionId }
    }

    override suspend fun revokeGrant(grantId: String) {
        revokedGrants += grantId
        grants = grants.filterNot { it.id == grantId }
    }

    override suspend fun session() = session

    /** The foreground poll: returns quickly with what is pending. */
    override suspend fun sync(waitSecs: UInt): List<PendingItem> {
        syncStarts.incrementAndGet()
        kotlinx.coroutines.delay(250)
        return pending
    }

    // ---- uploads ------------------------------------------------------------------------------------------------

    /** Uploads waiting for a decision, by blob id. */
    val blobs = java.util.concurrent.ConcurrentHashMap<String, BlobView>()
    val blobAnswers = CopyOnWriteArrayList<Pair<String, Boolean>>()

    override suspend fun blobView(id: String): BlobView = blobs[id] ?: throw CoreException.NotFound()

    override suspend fun answerBlob(id: String, approve: Boolean) {
        blobAnswers += id to approve
        blobs.remove(id)
        pending = pending.filterNot { it.id == id }
    }

    // ---- MCP servers --------------------------------------------------------------------------------------------

    @Volatile var mcp: List<McpServerView> = emptyList()
    /** What the next `mcpAdd` answers; null: the server is added at once (see [TestData.mcpServer]). */
    @Volatile var mcpNextAdd: McpAddStep? = null
    /** What the next `mcpRefresh` answers; null: the server as it is. */
    @Volatile var mcpNextRefresh: McpAddStep? = null
    /** Thrown by every MCP call that talks to a server. */
    @Volatile var mcpFailure: CoreException? = null
    val mcpAdds = CopyOnWriteArrayList<Pair<String, String?>>()
    val mcpTokenAdds = CopyOnWriteArrayList<List<String?>>()
    val mcpSignIns = CopyOnWriteArrayList<Pair<String, String>>()
    val mcpRefreshes = CopyOnWriteArrayList<String>()
    val mcpRemoved = CopyOnWriteArrayList<String>()
    val mcpHeavy = CopyOnWriteArrayList<Triple<String, String, Boolean>>()

    override suspend fun mcpServers(): List<McpServerView> = mcp

    override suspend fun mcpAdd(url: String, name: String?): McpAddStep {
        mcpFailure?.let { throw it }
        mcpAdds += url to name
        val step = mcpNextAdd ?: McpAddStep.Added(TestData.mcpServer("added", name ?: "Added", url))
        when (step) {
            is McpAddStep.Added -> mcp = mcp.filterNot { it.id == step.server.id } + step.server
            is McpAddStep.NeedsSignIn -> if (mcp.none { it.id == step.serverId }) {
                mcp = mcp + TestData.mcpServer(step.serverId, name ?: "New server", url, status = "needs_sign_in", tools = emptyList())
            }
        }
        return step
    }

    override suspend fun mcpAddWithToken(url: String, token: String, name: String?): McpServerView {
        mcpFailure?.let { throw it }
        mcpTokenAdds += listOf(url, token, name)
        val server = TestData.mcpServer("tokened", name ?: "Tokened", url)
        mcp = mcp + server
        return server
    }

    override suspend fun mcpFinishSignIn(serverId: String, redirectUrl: String): McpServerView {
        mcpSignIns += serverId to redirectUrl
        mcpFailure?.let { throw it }
        val server = (mcp.firstOrNull { it.id == serverId } ?: throw CoreException.NotFound())
            .copy(status = "ok", error = null, tools = TestData.mcpTools())
        mcp = mcp.map { if (it.id == serverId) server else it }
        return server
    }

    override suspend fun mcpRefresh(id: String): McpAddStep {
        mcpRefreshes += id
        mcpFailure?.let { throw it }
        return mcpNextRefresh ?: McpAddStep.Added(mcp.firstOrNull { it.id == id } ?: throw CoreException.NotFound())
    }

    override suspend fun mcpRemove(id: String) {
        mcpRemoved += id
        mcp = mcp.filterNot { it.id == id }
    }

    override suspend fun mcpSetHeavy(id: String, tool: String, heavy: Boolean) {
        mcpHeavy += Triple(id, tool, heavy)
        mcp = mcp.map { s -> if (s.id != id) s else s.copy(tools = s.tools.map { if (it.name == tool) it.copy(heavy = heavy) else it }) }
    }

    // ---- Autopilot ----------------------------------------------------------------------------------------------

    /** A connection's own Autopilot row, as the core stores it. */
    data class ApRow(val mode: AutopilotMode? = null, val bypassUntil: Long? = null, val profileId: String? = null)

    /** The global row: `mode` null = the default (Assisted with a model, else Manual). */
    @Volatile var apGlobal = ApRow()
    val apConnections = java.util.concurrent.ConcurrentHashMap<String, ApRow>()
    @Volatile var model: ModelStatus = TestData.modelStatus()
    @Volatile var profiles: List<ProfileView> = TestData.profiles()
    @Volatile var wifiOnly = true
    val suggestions = java.util.concurrent.ConcurrentHashMap<String, SuggestionView>()
    /** What "Try it" answers; null: a suggestion built from the text (deny when it mentions "password"). */
    @Volatile var evaluation: SuggestionView? = null
    /** Thrown by `setAutopilotMode` (e.g. a connection paired a minute ago cannot be bypassed). */
    @Volatile var modeFailure: CoreException? = null
    /** Thrown by `downloadModel` after some progress; the model then reads as failed with [downloadError]. */
    @Volatile var downloadFailure: CoreException? = null
    @Volatile var downloadError: String? = null
    @Volatile var runtime: ModelRuntime? = null

    val modeCalls = CopyOnWriteArrayList<Triple<String?, AutopilotMode?, UInt?>>()
    val classLocks = CopyOnWriteArrayList<Triple<String, String, Boolean?>>()
    val presets = CopyOnWriteArrayList<Pair<String, Preset>>()
    val corrections = CopyOnWriteArrayList<Pair<Long, Verdict>>()
    val evaluations = CopyOnWriteArrayList<Pair<String?, String>>()
    val assigned = CopyOnWriteArrayList<Pair<String, String?>>()
    val profileCalls = CopyOnWriteArrayList<String>()
    val downloads = java.util.concurrent.atomic.AtomicInteger()

    /** The clock the screens use: frozen in tests. */
    private fun now(): Long = (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000

    fun resetAutopilot() {
        apGlobal = ApRow()
        apConnections.clear()
        model = TestData.modelStatus()
        profiles = TestData.profiles()
        wifiOnly = true
        suggestions.clear()
        evaluation = null
        modeFailure = null
        downloadFailure = null
        downloadError = null
        listOf(modeCalls, classLocks, presets, corrections, evaluations, assigned, profileCalls).forEach { it.clear() }
        downloads.set(0)
    }

    private fun defaultProfile() = profiles.firstOrNull { it.isDefault } ?: profiles.first()

    private fun installed() = model.state == ModelState.INSTALLED

    private fun globalBase() = apGlobal.mode ?: if (installed()) AutopilotMode.ASSISTED else AutopilotMode.MANUAL

    private fun globalMode(): AutopilotMode = when {
        apGlobal.mode == AutopilotMode.LOCKDOWN -> AutopilotMode.LOCKDOWN
        (apGlobal.bypassUntil ?: 0) > now() -> AutopilotMode.BYPASS
        else -> globalBase()
    }

    override fun setModelRuntime(runtime: ModelRuntime) {
        this.runtime = runtime
    }

    override suspend fun autopilotSettings(): AutopilotSettings {
        val now = now()
        return AutopilotSettings(
            mode = globalMode(),
            baseMode = globalBase(),
            bypassUntil = apGlobal.bypassUntil?.takeIf { it > now },
            defaultProfileId = defaultProfile().id,
            wifiOnly = wifiOnly,
            model = model,
            connections = apConnections.map { (id, row) ->
                val mode = when {
                    globalMode() == AutopilotMode.LOCKDOWN || row.mode == AutopilotMode.LOCKDOWN -> AutopilotMode.LOCKDOWN
                    (row.bypassUntil ?: 0) > now -> AutopilotMode.BYPASS
                    else -> row.mode ?: globalMode()
                }
                ConnectionAutopilot(id, row.mode, row.bypassUntil?.takeIf { it > now }, mode, row.profileId ?: defaultProfile().id)
            },
        )
    }

    override suspend fun setAutopilotMode(connectionId: String?, mode: AutopilotMode?, minutes: UInt?) {
        modeFailure?.let { throw it }
        modeCalls += Triple(connectionId, mode, minutes)
        val row = if (connectionId == null) apGlobal else apConnections[connectionId] ?: ApRow()
        val next = if (mode == AutopilotMode.BYPASS) {
            row.copy(mode = row.mode.takeIf { it != AutopilotMode.LOCKDOWN }, bypassUntil = now() + (minutes ?: 15u).toLong() * 60)
        } else {
            row.copy(mode = mode, bypassUntil = null)
        }
        if (connectionId == null) apGlobal = next else apConnections[connectionId] = next
    }

    override suspend fun setAutopilotWifiOnly(wifiOnly: Boolean) {
        this.wifiOnly = wifiOnly
    }

    override suspend fun autopilotProfiles() = profiles

    override suspend fun createProfile(name: String, icon: String?): ProfileView {
        profileCalls += "create:$name"
        val profile = TestData.profile("p${profiles.size + 1}", name, icon, isDefault = false, memory = 0u, classes = emptyList(), trainedAt = null)
        profiles = profiles + profile
        return profile
    }

    override suspend fun renameProfile(profileId: String, name: String, icon: String?) {
        profileCalls += "rename:$profileId:$name:$icon"
        profiles = profiles.map { if (it.id == profileId) it.copy(name = name, icon = icon) else it }
    }

    override suspend fun deleteProfile(profileId: String) {
        if (profiles.size <= 1) throw CoreException.Invalid("the last profile cannot be deleted")
        profileCalls += "delete:$profileId"
        val wasDefault = profiles.first { it.id == profileId }.isDefault
        profiles = profiles.filterNot { it.id == profileId }
        if (wasDefault) profiles = profiles.mapIndexed { i, p -> p.copy(isDefault = i == 0) }
    }

    override suspend fun resetProfile(profileId: String) {
        profileCalls += "reset:$profileId"
        profiles = profiles.map { if (it.id == profileId) it.copy(memoryCount = 0u, classes = emptyList(), trainedAt = null) else it }
    }

    override suspend fun setDefaultProfile(profileId: String) {
        profileCalls += "default:$profileId"
        profiles = profiles.map { it.copy(isDefault = it.id == profileId) }
    }

    override suspend fun assignProfile(connectionId: String, profileId: String?) {
        assigned += connectionId to profileId
        apConnections[connectionId] = (apConnections[connectionId] ?: ApRow()).copy(profileId = profileId)
    }

    override suspend fun setClassLock(profileId: String, classKey: String, locked: Boolean?) {
        classLocks += Triple(profileId, classKey, locked)
        profiles = profiles.map { p ->
            if (p.id != profileId) {
                p
            } else {
                p.copy(classes = p.classes.map { c -> if (c.classKey == classKey) c.copy(manual = locked?.not(), autoApprove = locked == false || (locked == null && c.decisionsToUnlock == 0u && (c.shadowAccuracy ?: 0f) >= 0.95f)) else c })
            }
        }
    }

    override suspend fun setPreset(profileId: String, preset: Preset) {
        presets += profileId to preset
        profiles = profiles.map { if (it.id == profileId) it.copy(preset = preset) else it }
    }

    override suspend fun autopilotSuggestion(requestId: String): SuggestionView? = suggestions[requestId]

    override suspend fun correctDecision(activityId: Long, shouldHave: Verdict) {
        corrections += activityId to shouldHave
        activity = activity.map { e -> if (e.id == activityId) e.copy(autopilot = e.autopilot?.copy(correctable = false)) else e }
    }

    override suspend fun modelStatus(): ModelStatus = model

    override suspend fun downloadModel(progress: DownloadProgress): ModelStatus {
        downloads.incrementAndGet()
        model = model.copy(state = ModelState.DOWNLOADING, downloadedBytes = 0u, error = null)
        progress.progress(120_000_000u, 0u)
        model = model.copy(downloadedBytes = 120_000_000u)
        downloadFailure?.let {
            model = model.copy(state = ModelState.FAILED, downloadedBytes = 0u, error = downloadError)
            throw it
        }
        progress.progress(412_000_000u, 0u)
        model = model.copy(state = ModelState.INSTALLED, downloadedBytes = 0u, sizeBytes = 412_000_000u)
        return model
    }

    override suspend fun deleteModel() {
        model = TestData.modelStatus()
    }

    override suspend fun autopilotEvaluate(profileId: String?, situation: String): SuggestionView {
        evaluations += profileId to situation
        evaluation?.let { return it }
        val risky = situation.contains("password", ignoreCase = true)
        return if (risky) {
            TestData.suggestion(
                "", Verdict.DENY, 0.03f, 0.95f, 0.88f, novel = true, reason = "Like 2 times you denied: Send an email · unknown recipient",
                neighbours = listOf(
                    dev.reins.core.NeighbourView("Send an email · unknown recipient", Verdict.DENY, 0.89f, 1_699_990_000),
                    dev.reins.core.NeighbourView("Forward emails · outside address", Verdict.DENY, 0.81f, 1_699_900_000),
                ),
            )
        } else {
            TestData.suggestion("")
        }
    }

    companion object {
        fun defaultServices(): List<ServiceView> = listOf(
            ServiceView("gmail", "Gmail", "google", true, null, emptyList()),
            ServiceView("gcalendar", "Google Calendar", "google", true, null, emptyList()),
            ServiceView("gcontacts", "Google Contacts", "google", true, null, emptyList()),
            ServiceView("telegram", "Telegram", "telegram", true, null, emptyList()),
            ServiceView("github", "GitHub", "token", true, null, emptyList()),
            ServiceView("gitlab", "GitLab", "token", true, null, emptyList()),
            ServiceView("codeberg", "Codeberg", "token", true, null, emptyList()),
            ServiceView("bitbucket", "Bitbucket", "token", true, null, emptyList()),
            ServiceView("device_calendar", "Phone calendar", "device", true, null, emptyList()),
            ServiceView("device_contacts", "Phone contacts", "device", true, null, emptyList()),
            ServiceView("sms", "Text messages", "device", true, null, emptyList()),
            ServiceView("vault", "Password vault", "vault", true, null, emptyList()),
        )

        /** The PRF output of every passkey of the fake account (its vault's copy opens with it). */
        val PASSKEY_PRF = ByteArray(32) { 42 }
        val EXISTING_PASSKEY_ID = byteArrayOf(1, 2, 3, 4)

        fun existingPasskey() = VaultPasskeyView(EXISTING_PASSKEY_ID, "Pixel 8", 1_699_000_000)

        /** One instance per test process; tests reset it. Built after the values above, which it uses. */
        val shared = FakeCore()

        const val SSO_STATE = "st4te"
        const val SSO_VERIFIER = "v3rifier"
        const val RECOVERY_CODE = "ABCD-EFGH-IJKL-MNOP-QRST-UVWX-YZ23-4567-ABCD-EFGH-IJKL-MNOP-QRST"
        /** The code a reset vault gets. */
        const val RESET_RECOVERY_CODE = "HV3N-Q8RT-ZL2K-M7WD-PX4C-BJ9F-E6YS-NA5G-UT3R-KC8M-WQ2H-FD7L-YP4X"
    }
}
