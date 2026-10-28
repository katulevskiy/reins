package dev.reins.android

import dev.reins.core.*
import dev.reins.core.AccountKeys
import dev.reins.core.BudgetView
import dev.reins.core.PaymentsOverview
import dev.reins.core.PurchaseChoice
import dev.reins.core.SecretsRevoked
import dev.reins.core.SpendLimitInput
import dev.reins.core.SpendLimitView
import dev.reins.core.SpendingView
import dev.reins.core.AccountView
import dev.reins.core.StartingPolicy
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
import dev.reins.core.ActivityPush
import dev.reins.core.ApprovalChoice
import dev.reins.core.ApprovalKind
import dev.reins.core.StandingGrant
import dev.reins.core.ApprovalView
import dev.reins.core.BlobView
import dev.reins.core.Blobatar
import dev.reins.core.ConnectionView
import dev.reins.core.DeviceView
import dev.reins.core.CoreException
import dev.reins.core.EmailContent
import dev.reins.core.GitCommitView
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
import dev.reins.core.VaultField
import dev.reins.core.VaultItemDetail
import dev.reins.core.VaultItemInput
import dev.reins.core.VaultItemKind
import dev.reins.core.VaultItemSummary
import dev.reins.core.VaultPasskeyOptions
import dev.reins.core.VaultPasskeyView
import dev.reins.core.VaultSshKey
import dev.reins.core.VaultUse
import java.util.concurrent.CopyOnWriteArrayList

/** In-memory core for UI tests: holds state, records the calls that matter. */
class FakeCore : ReinsCoreInterface {
    val permissionViews = java.util.concurrent.ConcurrentHashMap<String, AccountPermissionsView>()
    val savedPermissionDrafts = CopyOnWriteArrayList<AccountPermissionsDraft>()
    val delegationRecords = CopyOnWriteArrayList<DelegationView>()
    @Volatile var permissionFailure: Exception? = null
    val permissionScopeGates = java.util.concurrent.ConcurrentHashMap<String, kotlinx.coroutines.CompletableDeferred<Unit>>()
    @Volatile var permissionScopeOverride: PermissionScopeOptions? = null
    val permissionSaveCalls = java.util.concurrent.atomic.AtomicInteger()
    @Volatile var permissionSaveGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    override suspend fun accountPermissions(service: String, account: String): AccountPermissionsView = permissionViews["$service/$account"] ?: PermissionFixtures.view(service, account)
    override suspend fun accountPermissionDraft(service: String, account: String, preset: AccountPermissionPreset) = PermissionFixtures.draft(accountPermissions(service, account), preset)
    override suspend fun permissionScopeOptions(service: String, account: String, capability: PermissionCapability, query: String, limit: UInt): PermissionScopeOptions {
        permissionScopeGates[query]?.await()
        permissionScopeOverride?.let { return it }
        val recipient = capability == PermissionCapability.MAIL_SEND
        val kind = if (recipient) PermissionScopeKind.SELECTED_RECIPIENTS else PermissionScopeKind.SELECTED_RESOURCES
        return PermissionScopeOptions(if (recipient) listOf(kind) else listOf(PermissionScopeKind.ALL_IN_ACCOUNT, kind),
            if (query.contains('@')) listOf(PermissionScopeOption("recipient:" + query.lowercase(), query.lowercase(), "Exact address", service, account, kind)) else emptyList(), recipient, null)
    }
    override suspend fun previewAccountPermissions(draft: AccountPermissionsDraft): PermissionPreview {
        permissionFailure?.let { throw it }
        if (draft.expectedRevision != accountPermissions(draft.service, draft.account).revision) throw CoreException.Invalid("Permissions changed. Reload current settings.")
        return PermissionFixtures.preview(draft.expectedRevision)
    }
    override suspend fun saveAccountPermissions(draft: AccountPermissionsDraft, owner: OwnerCheck): AccountPermissionsView {
        permissionSaveCalls.incrementAndGet()
        permissionSaveGate?.await()
        permissionFailure?.let { throw it }
        if (!owner.confirm("Save permissions for ${draft.account}")) throw CoreException.Invalid("Owner did not confirm")
        previewAccountPermissions(draft)
        savedPermissionDrafts += draft
        val result = accountPermissions(draft.service, draft.account).copy(revision = draft.expectedRevision + 1u, capabilities = accountPermissions(draft.service, draft.account).capabilities.map { cap -> cap.copy(configuredMode = draft.rules.first { it.capability == cap.capability }.mode) }, expiresAt = draft.expiresAt, remainingUses = draft.maxUses)
        permissionViews["${draft.service}/${draft.account}"] = result
        return result
    }
    override suspend fun delegations() = delegationRecords.toList()
    override suspend fun delegationDraft(service: String, account: String): DelegationDraft {
        val base = accountPermissionDraft(service, account, AccountPermissionPreset.READ_ASSIST)
        return DelegationDraft(service, account, base.expectedRevision, base.agentIds, false, base.rules.filter { it.mode == PermissionMode.AUTO }, now()+3600, 20u)
    }
    override suspend fun previewDelegation(draft: DelegationDraft) = PermissionFixtures.preview(draft.expectedRevision)
    override suspend fun issueDelegation(draft: DelegationDraft, owner: OwnerCheck): DelegationView {
        permissionFailure?.let { throw it }
        if (!owner.confirm("Create temporary permission for ${draft.account}")) throw CoreException.Invalid("Owner did not confirm")
        val result = DelegationView("temporary-${delegationRecords.size}", draft.expectedRevision + 1u, "Temporary account permission", draft.service, draft.account, DelegationState.ACTIVE, draft.expiresAt, draft.agentIds, draft.rules, 0u, true, 0u, draft.maxUses, "Selected agents only", "Explicit temporary permission", emptyList())
        delegationRecords += result
        return result
    }
    override suspend fun changeDelegation(id: String, expectedRevision: ULong, change: DelegationChange, owner: OwnerCheck): DelegationView {
        if (!owner.confirm("Change temporary permission")) throw CoreException.Invalid("Owner did not confirm")
        val record = delegationRecords.first { it.id == id }
        if (record.revision != expectedRevision) throw CoreException.Invalid("Permissions changed. Reload current settings.")
        val result = record.copy(revision = record.revision + 1u, state = when(change) { DelegationChange.RESUME -> DelegationState.ACTIVE; DelegationChange.PAUSE -> DelegationState.PAUSED; DelegationChange.REVOKE -> DelegationState.REVOKED })
        delegationRecords.remove(record); delegationRecords += result
        return result
    }
    @Volatile var session: SessionInfo? = null
    @Volatile var loginError: CoreException? = null
    @Volatile var pending: List<PendingItem> = emptyList()
    @Volatile var approval: ApprovalView? = null
    @Volatile var pairing: PairingView? = null
    @Volatile var grants: List<GrantView> = emptyList()
    @Volatile var connections: List<ConnectionView> = emptyList()
    /** Thrown by [connections] instead of the list (a phone the server does not show them to). */
    @Volatile var connectionsError: CoreException? = null
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

    /** The newest [limit] of [activity] (seeded newest first), as the core reads the history at once. */
    override suspend fun activity(limit: UInt) = activity.take(limit.toInt())

    /** The ids `activityEntry` was asked for. */
    val entryLookups = CopyOnWriteArrayList<Long>()

    /** Any entry of [activity] by its id, however far back, as the core reads one. */
    override suspend fun activityEntry(entryId: Long): ActivityEntry? {
        entryLookups += entryId
        return activity.firstOrNull { it.id == entryId }
    }

    /** Whether the user edited the name field, for each answer in [pairingAnswers]. */
    val pairingNameEdited = CopyOnWriteArrayList<Boolean>()

    override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: UByte?, label: String?, nameEdited: Boolean) {
        pairingAnswers += listOf(pairingId, approve, chosenCode, label)
        pairingNameEdited += nameEdited
        pending = pending.filterNot { it.id == pairingId }
    }

    /** Approval views by request id (the Activity cards read them); [approval] answers for any other id. */
    val views = java.util.concurrent.ConcurrentHashMap<String, ApprovalView>()

    override suspend fun approvalView(requestId: String) = views[requestId] ?: approval ?: throw CoreException.NotFound()

    /** What each Activity entry kept of its approval, by entry id (none: an older entry). */
    val entryApprovals = java.util.concurrent.ConcurrentHashMap<Long, ApprovalView>()

    override suspend fun activityApproval(entryId: Long): ApprovalView? = entryApprovals[entryId]

    /** Pushes of entries, as `activityPush` returns them. */
    val entryPushes = mutableMapOf<Long, ActivityPush>()

    override suspend fun activityPush(entryId: Long): ActivityPush? = entryPushes[entryId]

    // ---- Activity's search: a plain case-insensitive match over the entries in memory ---------------------------------

    /** Every query `searchActivity` and `activityStamps` were asked, in order. */
    val activityQueries = CopyOnWriteArrayList<dev.reins.core.ActivityQuery>()

    private fun matching(q: dev.reins.core.ActivityQuery): List<ActivityEntry> {
        activityQueries += q
        val words = searchTerms(q.text)
        return activity.filter { e ->
            (q.outcomes.isEmpty() || dev.reins.android.ui.activity.outcomeKind(e.outcome) in q.outcomes) &&
                (q.deciders.isEmpty() || dev.reins.android.ui.activity.deciderKind(e.decidedBy) in q.deciders) &&
                (q.services.isEmpty() || e.service in q.services) &&
                (q.actions.isEmpty() || e.action in q.actions) &&
                (q.connections.isEmpty() || e.connectionId in q.connections) &&
                (q.since == null || e.at >= q.since!!) &&
                (q.until == null || e.at < q.until!!) &&
                words.all { w -> searchable(e).any { it.contains(w, ignoreCase = true) } }
        }.sortedWith(compareByDescending<ActivityEntry> { it.at }.thenByDescending { it.id })
    }

    private fun searchTerms(text: String): List<String> =
        text.lowercase().split(Regex("[^\\p{L}\\p{N}]+")).filter { it.isNotEmpty() }.distinct()

    private fun searchable(e: ActivityEntry): List<String> = listOfNotNull(
        dev.reins.android.ui.common.askSentence(e).text, e.connectionLabel, e.detail, e.opTitle, e.info.query,
        e.info.email?.subject, e.info.email?.body, e.label?.subject, e.request?.full,
    ) + e.info.messages.map { it.subject } + e.info.email?.to.orEmpty()

    override suspend fun searchActivity(query: dev.reins.core.ActivityQuery, after: dev.reins.core.ActivityCursor?, limit: UInt): dev.reins.core.ActivityPage {
        val all = matching(query)
        val from = if (after == null) all else all.filter { it.at < after.at || (it.at == after.at && it.id < after.id) }
        val page = from.take(limit.toInt().coerceIn(1, 200))
        val next = if (from.size > page.size) page.lastOrNull()?.let { dev.reins.core.ActivityCursor(it.at, it.id) } else null
        val terms = searchTerms(query.text)
        val hits = page.map { e -> dev.reins.core.ActivityHit(e, snippetOf(e, terms)) }
        return dev.reins.core.ActivityPage(hits, next, all.size.toUInt(), terms)
    }

    /** A snippet when a word is found in the email's body and not in the row's title: the words around it. */
    private fun snippetOf(e: ActivityEntry, terms: List<String>): dev.reins.core.ActivitySnippet? {
        val body = e.info.email?.body ?: return null
        val title = dev.reins.android.ui.activity.matchRanges(dev.reins.android.ui.common.askSentence(e).text, terms)
        if (terms.isEmpty() || title.size >= terms.size) return null
        val ranges = dev.reins.android.ui.activity.matchRanges(body, terms)
        if (ranges.isEmpty()) return null
        val spans = mutableListOf<dev.reins.core.ActivityHighlight>()
        var at = maxOf(0, ranges.first().first - 30)
        for (r in ranges) {
            if (r.first > at) spans += dev.reins.core.ActivityHighlight(body.substring(at, r.first), false)
            spans += dev.reins.core.ActivityHighlight(body.substring(r.first, r.last + 1), true)
            at = r.last + 1
        }
        if (at < body.length) spans += dev.reins.core.ActivityHighlight(body.substring(at, minOf(body.length, at + 80)), false)
        return dev.reins.core.ActivitySnippet(dev.reins.core.ActivityField.BODY, spans)
    }

    override suspend fun activityStamps(query: dev.reins.core.ActivityQuery): List<dev.reins.core.ActivityStamp> =
        matching(query).map {
            dev.reins.core.ActivityStamp(it.id, it.at, dev.reins.android.ui.activity.outcomeKind(it.outcome), dev.reins.android.ui.activity.deciderKind(it.decidedBy))
        }

    override suspend fun activityFacets(): dev.reins.core.ActivityFacets {
        val newest = activity.sortedByDescending { it.at }
        return dev.reins.core.ActivityFacets(
            newest.groupingBy { it.service }.eachCount().entries.sortedByDescending { it.value }.map { it.key },
            newest.groupingBy { it.action }.eachCount().entries.sortedByDescending { it.value }.map { it.key },
            newest.distinctBy { it.connectionId }.map { dev.reins.core.ActivityConnection(it.connectionId, it.connectionLabel) },
        )
    }

    /** Commits `activityCommitChange` loads, by entry and short id; anything else fails as with no connection. */
    val commitChanges = java.util.concurrent.ConcurrentHashMap<Pair<Long, String>, GitCommitView>()

    /** The calls `activityCommitChange` got: (entry, short id). */
    val commitChangeCalls = CopyOnWriteArrayList<Pair<Long, String>>()

    /** Holds `activityCommitChange` until completed (to see it loading); null: answers at once. */
    @Volatile var commitChangeGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun activityCommitChange(entryId: Long, shortSha: String): GitCommitView {
        commitChangeCalls += entryId to shortSha
        commitChangeGate?.await()
        return commitChanges[entryId to shortSha] ?: throw CoreException.Service("GitHub is not connected")
    }

    /** Makes approving fail (after the sheet has already closed: the app answers in the background). */
    @Volatile var approveFailure: CoreException? = null

    override suspend fun approve(requestId: String, choice: ApprovalChoice) {
        approveFailure?.let { throw it }
        approvals += requestId to choice
        pending = pending.filterNot { it.id == requestId }
    }

    // ---- Payments: what the screens show, and what they asked for ------------------------------------------------

    @Volatile var payments: PaymentsOverview? = null
    @Volatile var spending: SpendingView = SpendingView(0, emptyList(), emptyList(), emptyList())
    /** Thrown by `paymentsConnectProvider` (a key Privacy.com refuses). */
    @Volatile var connectProviderError: CoreException? = null
    val purchaseApprovals = CopyOnWriteArrayList<Pair<String, PurchaseChoice>>()
    /** One line per settings call ("method card:c1 true", "provider privacy sandbox=false"). */
    val paymentsCalls = CopyOnWriteArrayList<String>()
    val addedLimits = CopyOnWriteArrayList<SpendLimitInput>()

    override suspend fun approvePurchase(requestId: String, choice: PurchaseChoice) {
        purchaseApprovals += requestId to choice
        pending = pending.filterNot { it.id == requestId }
    }

    override suspend fun paymentsOverview() = payments ?: throw CoreException.NotFound()

    override suspend fun paymentsSetMethod(methodId: String, enabled: Boolean) {
        paymentsCalls += "method $methodId $enabled"
        payments = payments?.let { o -> o.copy(methods = o.methods.map { if (it.id == methodId) it.copy(enabled = enabled) else it }) }
    }

    override suspend fun paymentsSetNickname(methodId: String, nickname: String?) {
        paymentsCalls += "nickname $methodId $nickname"
    }

    override suspend fun paymentsSetDefaults(methodId: String?, addressId: String?) {
        paymentsCalls += "defaults $methodId $addressId"
        payments = payments?.copy(defaultMethod = methodId, defaultAddress = addressId)
    }

    override suspend fun paymentsConnectProvider(kind: String, apiKey: String, sandbox: Boolean, singleUse: Boolean) {
        connectProviderError?.let { throw it }
        paymentsCalls += "provider $kind sandbox=$sandbox singleUse=$singleUse"
    }

    override suspend fun paymentsDisconnectProvider() {
        paymentsCalls += "disconnect provider"
        payments = payments?.copy(provider = null)
    }

    override suspend fun paymentsSetCardOptions(tolerancePct: UInt, singleUse: Boolean) {
        paymentsCalls += "card options $tolerancePct $singleUse"
        payments = payments?.copy(tolerancePct = tolerancePct)
    }

    override suspend fun paymentsSetBudget(budget: BudgetView) {
        paymentsCalls += "budget ${budget.connectionId} ${budget.perPurchase} ${budget.perMonth} ${budget.merchants}"
    }

    override suspend fun paymentsAddLimit(limit: SpendLimitInput): SpendLimitView {
        addedLimits += limit
        return SpendLimitView(
            "limit-${addedLimits.size}", limit.connectionId, "Claude", limit.merchants, limit.method, "Privacy.com card",
            limit.currency, limit.perPurchase, limit.perPeriod, limit.period, "Up to a limit", 0, 0, limit.durationSecs, true,
        )
    }

    override suspend fun paymentsRemoveLimit(limitId: String) {
        paymentsCalls += "remove limit $limitId"
        payments = payments?.let { o -> o.copy(limits = o.limits.filterNot { it.id == limitId }) }
    }

    override suspend fun paymentsSpending(since: Long) = spending

    override suspend fun paymentsAcknowledgeCharge(purchaseId: String) {
        paymentsCalls += "acknowledge $purchaseId"
    }

    override suspend fun paymentsClearPurchase(purchaseId: String) {
        paymentsCalls += "clear $purchaseId"
    }

    override suspend fun paymentsCloseCard(purchaseId: String) {
        paymentsCalls += "close $purchaseId"
    }

    /** Requests approved without being opened (a notification's Approve, "Approve all"). */
    val quickApprovals = CopyOnWriteArrayList<String>()

    override suspend fun approveQuick(requestId: String) {
        val item = pending.firstOrNull { it.id == requestId } ?: throw CoreException.NotFound()
        if (!item.quick) throw CoreException.Invalid("Open this request to decide.")
        quickApprovals += requestId
        pending = pending.filterNot { it.id == requestId }
    }

    /** The starting rule; null until chosen. */
    @Volatile var startingPolicy: StartingPolicy? = null

    override suspend fun startingPolicy() = startingPolicy

    override suspend fun setStartingPolicy(policy: StartingPolicy) {
        startingPolicy = policy
    }

    override suspend fun connections(): List<ConnectionView> {
        connectionsError?.let { throw it }
        return connections
    }

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

    @Volatile var removeAccountGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun removeAccount(account: String) {
        removedAccounts += account
        removeAccountGate?.await()
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

    @Volatile var tokenAccountGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun addTokenAccount(service: String, token: String): AccountView {
        serviceFailure?.let { throw it }
        tokensAdded += service to token
        tokenAccountGate?.await()
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

    val extended = CopyOnWriteArrayList<Pair<String, ULong>>()

    override suspend fun extendGrant(grantId: String, durationSecs: ULong) {
        extended += grantId to durationSecs
        grants = grants.map { if (it.id == grantId) it.copy(expiresAt = 1_700_000_000L + durationSecs.toLong()) else it }
    }

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

    /** The renames, in order: "c1=Claude (work)", "c1=" (cleared). */
    val renames = CopyOnWriteArrayList<String>()

    /** As the core: one clean line of at most 64 characters; blank takes the name away (its own name again). */
    override suspend fun renameConnection(connectionId: String, name: String?): String? {
        val clean = name?.let { dev.reins.android.ui.common.untrustedName(it, 64) }?.ifEmpty { null }
        renames += "$connectionId=${clean.orEmpty()}"
        connections = connections.map {
            if (it.id != connectionId) return@map it
            val own = it.ownLabel.ifEmpty { if (it.name == null) it.label else it.clientName }
            it.copy(name = clean, label = clean ?: own, ownLabel = own, clientName = it.clientName.ifEmpty { own })
        }
        return clean
    }

    override suspend fun setConnectionIcon(connectionId: String, icon: String?) {
        icons[connectionId] = icon ?: "auto"
        connections = connections.map { if (it.id == connectionId) it.copy(icon = icon) else it }
    }

    /** The blobatar calls, in order: "shape 3 colour 7 c1", "shuffle c1". */
    val blobatarCalls = CopyOnWriteArrayList<String>()
    private var shuffles = 0

    /** Keeps the seed, like the core; a connection without one gets its id. */
    override suspend fun setConnectionBlobatar(connectionId: String, shape: UByte, colour: UByte): Blobatar {
        blobatarCalls += "shape $shape colour $colour $connectionId"
        val seed = connections.firstOrNull { it.id == connectionId }?.blobatar?.seed ?: connectionId
        return Blobatar(seed, shape, colour).also { b -> keepBlobatar(connectionId, b) }
    }

    /** A new seed, and the next shape and colour (always others). */
    override suspend fun shuffleConnectionBlobatar(connectionId: String): Blobatar {
        blobatarCalls += "shuffle $connectionId"
        val current = connections.firstOrNull { it.id == connectionId }?.blobatar ?: Blobatar(connectionId, 0u, 0u)
        shuffles++
        val next = Blobatar(
            "shuffled-$shuffles",
            ((current.shape.toInt() + 1) % 10).toUByte(),
            ((current.colour.toInt() + 5) % 12).toUByte(),
        )
        return next.also { keepBlobatar(connectionId, it) }
    }

    private fun keepBlobatar(connectionId: String, blobatar: Blobatar) {
        connections = connections.map { if (it.id == connectionId) it.copy(blobatar = blobatar) else it }
    }

    override suspend fun deny(requestId: String) {
        denials += requestId
        pending = pending.filterNot { it.id == requestId }
    }

    // ---- tickets (Decisions v2): Later, a denial's note, Why? ------------------------------------------------------

    /** `decideLater` calls, by request id. */
    val laterCalls = CopyOnWriteArrayList<String>()
    /** `denyWithNote` calls: (request id, note). */
    val notedDenials = CopyOnWriteArrayList<Pair<String, String?>>()
    /** `askWhy` calls: (request id, question). */
    val whyQuestions = CopyOnWriteArrayList<Pair<String, String>>()

    /** The window Later gives (the server's default Deferred window). */
    @Volatile var deferredWindowSecs: Long = 14_400

    override suspend fun decideLater(requestId: String) {
        val item = pending.firstOrNull { it.id == requestId } ?: throw CoreException.NotFound()
        if (!item.canDefer || item.ticket != null) throw CoreException.Invalid("This request can't wait.")
        laterCalls += requestId
        val at = now()
        val ticket = dev.reins.core.TicketView("rt_$requestId", "deferred", at, at + deferredWindowSecs, null, null, null)
        pending = pending.map { if (it.id == requestId) it.copy(ticket = ticket, canDefer = false, waitUntil = ticket.deadline) else it }
    }

    override suspend fun denyWithNote(requestId: String, note: String?) {
        notedDenials += requestId to note
        pending = pending.filterNot { it.id == requestId }
    }

    override suspend fun askWhy(requestId: String, question: String) {
        val item = pending.firstOrNull { it.id == requestId } ?: throw CoreException.NotFound()
        whyQuestions += requestId to question
        val at = now()
        val ticket = item.ticket?.copy(state = "needs_info", question = question, agentReply = null)
            ?: dev.reins.core.TicketView("rt_$requestId", "needs_info", at, at + deferredWindowSecs, question, null, null)
        pending = pending.map { if (it.id == requestId) it.copy(ticket = ticket, canDefer = false, waitUntil = ticket.deadline) else it }
    }

    /** This phone's signing key, as agents and the desktop app pin it; [signingKeyCalls] counts the reads. */
    @Volatile var signingKey: String = "4821 9930 1274"
    val signingKeyCalls = java.util.concurrent.atomic.AtomicInteger()

    override suspend fun signingKeyFingerprint(): String {
        signingKeyCalls.incrementAndGet()
        return signingKey
    }

    fun resetTickets() {
        laterCalls.clear()
        notedDenials.clear()
        whyQuestions.clear()
        deferredWindowSecs = 14_400
        signingKey = "4821 9930 1274"
        signingKeyCalls.set(0)
    }

    // ---- Circles (Decisions v2, 04): people who share control ------------------------------------------------------

    /** Whether this phone can take part in Circles (the account's primary phone). */
    @Volatile var circlesOn: Boolean = true
    /** The arrangements, either side. */
    @Volatile var circlePeopleList: List<CirclePerson> = emptyList()
    /** Requests of other accounts for this phone (For others), waiting and settled. */
    @Volatile var circleReviewList: List<CircleReviewView> = emptyList()
    /** What a watcher was told. */
    @Volatile var circleNoticeList: List<CircleNoticeView> = emptyList()
    /** `circleAnswer` calls: (review id, approve, note). */
    val circleAnswers = CopyOnWriteArrayList<Triple<String, Boolean, String?>>()
    /** Thrown by `circleAnswer` when set. */
    @Volatile var circleAnswerFailure: Exception? = null
    /** The drafts `circleInviteCreate` was given. */
    val circleDrafts = CopyOnWriteArrayList<CircleDraft>()
    /** What `circleInvitePoll` answers next, one per poll (the last one stays). */
    val circleInviteSteps = java.util.concurrent.ConcurrentLinkedQueue<CircleInviteProgress>()
    @Volatile private var circleInviteLast: CircleInviteProgress = CircleInviteProgress.Waiting
    val circleInviteConfirms = CopyOnWriteArrayList<String>()
    val circleInviteCancels = CopyOnWriteArrayList<String>()
    /** The links `circleJoinOpen` was given. */
    val circleJoinLinks = CopyOnWriteArrayList<String>()
    /** What `circleJoinOpen` and `circleJoinPoll` answer. */
    @Volatile var circleJoinView: CircleInviteView? = null
    val circleJoinAccepts = CopyOnWriteArrayList<String>()
    val circleJoinDeclines = CopyOnWriteArrayList<String>()
    /** `circlePause` calls: (link id, paused). */
    val circlePauses = CopyOnWriteArrayList<Pair<String, Boolean>>()
    val circleEnds = CopyOnWriteArrayList<String>()
    val circleReviewReads = java.util.concurrent.atomic.AtomicInteger()

    override suspend fun circlesAvailable(): Boolean = circlesOn

    override suspend fun circleInviteCreate(draft: CircleDraft): CircleInviteStart {
        circleDrafts += draft
        return CircleInviteStart("inv1", "reins-circle:127.0.0.1:8000/secret123", now() + 900)
    }

    override suspend fun circleInvitePoll(id: String): CircleInviteProgress {
        circleInviteSteps.poll()?.let { circleInviteLast = it }
        return circleInviteLast
    }

    override suspend fun circleInviteConfirm(id: String): CirclePerson {
        circleInviteConfirms += id
        val helper = (circleInviteLast as? CircleInviteProgress.Accepted)?.helper ?: "alex@example.com"
        val draft = circleDrafts.lastOrNull()
        val person = CirclePerson(
            "link-$id", helper, draft?.role ?: CircleRole.COSIGN, true, "Money", draft?.deferred ?: true, 14_400uL,
            CircleLinkState.ACTIVE, now(), null, false,
        )
        circlePeopleList = circlePeopleList + person
        circleInviteLast = CircleInviteProgress.Active(person.linkId)
        return person
    }

    override suspend fun circleInviteCancel(id: String) {
        circleInviteCancels += id
        circleInviteLast = CircleInviteProgress.Cancelled
    }

    override suspend fun circleJoinOpen(link: String): CircleInviteView {
        circleJoinLinks += link
        if (!link.startsWith("reins-circle:")) throw CoreException.Invalid("This is not a Circle invitation.")
        return circleJoinView ?: throw CoreException.NotFound()
    }

    override suspend fun circleJoinPoll(id: String): CircleInviteView = circleJoinView ?: throw CoreException.NotFound()

    override suspend fun circleJoinAccept(id: String) {
        circleJoinAccepts += id
        circleJoinView = circleJoinView?.copy(stage = CircleJoinStage.ACCEPTED)
    }

    override suspend fun circleJoinDecline(id: String) {
        circleJoinDeclines += id
        circleJoinView = circleJoinView?.copy(stage = CircleJoinStage.DECLINED)
    }

    override suspend fun circlePeople(refresh: Boolean): List<CirclePerson> = circlePeopleList

    override suspend fun circlePause(linkId: String, paused: Boolean) {
        circlePauses += linkId to paused
        circlePeopleList = circlePeopleList.map {
            if (it.linkId == linkId) it.copy(state = if (paused) CircleLinkState.PAUSED else CircleLinkState.ACTIVE) else it
        }
    }

    override suspend fun circleEnd(linkId: String) {
        circleEnds += linkId
        val ward = guardianWards[linkId]
        if (ward == null || ward.noticeDays.toInt() == 0 && !ward.minor) {
            guardianWards.remove(linkId)
            circlePeopleList = circlePeopleList.filterNot { it.linkId == linkId }
            return
        }
        // The ward asks: a minor's waits for the guardian; else it ends after the notice the ward chose.
        if (ward.ending != null || ward.needsConsent) throw CoreException.Invalid("You asked to leave already.")
        val ending = if (ward.minor) null else now() + ward.noticeDays.toLong() * 86_400
        guardianWards[linkId] = ward.copy(ending = ending, needsConsent = ward.minor)
        circlePeopleList = circlePeopleList.map { if (it.linkId == linkId) it.copy(ending = ending, needsConsent = ward.minor) else it }
    }

    // ---- Guardianship (Decisions v2 P7) -----------------------------------------------------------------------------

    /** The ward's views of its guardianships, by link. */
    val guardianWards = java.util.concurrent.ConcurrentHashMap<String, WardView>()
    /** What a guardian was told, newest first. */
    @Volatile var guardianNoticeList: List<GuardianNoticeView> = emptyList()
    /** `guardianAcceptMinor` calls: (link id, signal). */
    val guardianMinorAccepts = CopyOnWriteArrayList<Pair<String, AgeSignalInput>>()
    val guardianMinorProposals = CopyOnWriteArrayList<String>()
    val guardianConsents = CopyOnWriteArrayList<String>()
    /** `guardianSetDigest` calls: (link id, on). */
    val guardianDigests = CopyOnWriteArrayList<Pair<String, Boolean>>()
    val guardianNoticeReads = java.util.concurrent.atomic.AtomicInteger()

    override suspend fun guardianProposeMinor(linkId: String) {
        guardianMinorProposals += linkId
    }

    override suspend fun guardianAcceptMinor(linkId: String, signal: AgeSignalInput, owner: OwnerCheck) {
        val ward = guardianWards[linkId] ?: throw CoreException.NotFound()
        // As the core: the owner confirms first, or nothing is marked.
        if (!owner.confirm("Mark this account a minor's")) throw CoreException.Invalid("Not confirmed.")
        if (signal == AgeSignalInput.Adult) throw CoreException.Invalid("This account isn't a minor's.")
        if (!ward.minorProposed) throw CoreException.Invalid("Your guardian hasn't asked for this.")
        guardianMinorAccepts += linkId to signal
        guardianWards[linkId] = ward.copy(minor = true, minorProposed = false)
    }

    override suspend fun guardianConsentEnd(linkId: String, owner: OwnerCheck) {
        // As the core: the owner confirms first, or nothing is consented.
        if (!owner.confirm("Let them leave")) throw CoreException.Invalid("Not confirmed.")
        val notice = guardianNoticeList.firstOrNull { it.linkId == linkId && it.needsConsent }
            ?: throw CoreException.Invalid("Nothing waits for your consent.")
        guardianConsents += linkId
        guardianNoticeList = guardianNoticeList.map { if (it.id == notice.id) it.copy(needsConsent = false) else it }
        circlePeopleList = circlePeopleList.filterNot { it.linkId == linkId }
    }

    override suspend fun guardianSetDigest(linkId: String, on: Boolean) {
        val ward = guardianWards[linkId] ?: throw CoreException.NotFound()
        guardianDigests += linkId to on
        guardianWards[linkId] = ward.copy(digest = on)
    }

    override suspend fun guardianNotices(refresh: Boolean): List<GuardianNoticeView> {
        guardianNoticeReads.incrementAndGet()
        return guardianNoticeList
    }

    override suspend fun guardianWardView(linkId: String): WardView = guardianWards[linkId] ?: throw CoreException.NotFound()

    override suspend fun circleReviews(refresh: Boolean): List<CircleReviewView> {
        circleReviewReads.incrementAndGet()
        return circleReviewList
    }

    override suspend fun circleNotices(): List<CircleNoticeView> = circleNoticeList

    override suspend fun circleAnswer(reviewId: String, approve: Boolean, note: String?) {
        circleAnswerFailure?.let { throw it }
        circleAnswers += Triple(reviewId, approve, note)
        circleReviewList = circleReviewList.map {
            if (it.id == reviewId) it.copy(state = if (approve) CircleReviewState.APPROVED else CircleReviewState.DENIED) else it
        }
    }

    fun resetCircles() {
        circlesOn = true
        circlePeopleList = emptyList()
        circleReviewList = emptyList()
        circleNoticeList = emptyList()
        circleAnswers.clear()
        circleAnswerFailure = null
        circleDrafts.clear()
        circleInviteSteps.clear()
        circleInviteLast = CircleInviteProgress.Waiting
        circleInviteConfirms.clear()
        circleInviteCancels.clear()
        circleJoinLinks.clear()
        circleJoinView = null
        circleJoinAccepts.clear()
        circleJoinDeclines.clear()
        circlePauses.clear()
        circleEnds.clear()
        circleReviewReads.set(0)
        guardianWards.clear()
        guardianNoticeList = emptyList()
        guardianMinorAccepts.clear()
        guardianMinorProposals.clear()
        guardianConsents.clear()
        guardianDigests.clear()
        guardianNoticeReads.set(0)
    }

    override suspend fun gmailStatus() = gmail

    override suspend fun grants() = grants

    /** Pushes handled, as `kind:id`; [pushFailure] makes the next ones fail. */
    val pushes = CopyOnWriteArrayList<String>()
    @Volatile var pushFailure: CoreException? = null

    override suspend fun handlePush(kind: String, id: String) {
        pushFailure?.let { throw it }
        pushes += "$kind:$id"
    }

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
        ssoFinishErrorAfterSignIn = null
        syncNotice = null
        syncNoticeError = null
        syncNoticeDismissals.set(0)
        sessionError = null
        sessionThrows = 0
        ssoBegins.clear()
        org = null
        family = null
        familyJoins.clear()
        familyConsents.clear()
        familyMinorAnswers.clear()
        orgsOffered = false
        pushTag = null
        orgReviews = emptyList()
        orgDecisions.clear()
        orgDecideError = null
        orgPasskeys = emptyList()
        passkeyConfirmations.clear()
        memberPolicies.clear()
        ssoBeginWorks.clear()
        enrollments.clear()
        postures.clear()
        serverInfos.clear()
        ssoFinishes.clear()
        resetError = null
        resets.clear()
        keys = AccountKeys.UNLOCKED
        recoveryCode = null
        unlockAttempts.clear()
        recoveryCodeReads.set(0)
        syncStarts.set(0)
        syncRefusals.set(0)
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

    /** What `serverInfo` answers per server; unknown servers say nothing (an older server). */
    val serverInfos = java.util.concurrent.ConcurrentHashMap<String, dev.reins.core.ServerInfo>()

    override suspend fun serverInfo(serverUrl: String) = serverInfos[serverUrl] ?: dev.reins.core.ServerInfo(null, null, null)

    /** The work account's org (`orgMembership`); null: a Personal account. */
    @Volatile var org: dev.reins.core.OrgView? = null
    val ssoBeginWorks = CopyOnWriteArrayList<List<String>>()
    val enrollments = CopyOnWriteArrayList<Pair<String, dev.reins.core.EnrollInput>>()
    val postures = CopyOnWriteArrayList<dev.reins.core.PostureInput>()

    override suspend fun ssoBeginWork(serverUrl: String, domain: String): SsoStart {
        ssoBeginWorks += listOf(serverUrl, domain)
        ssoBeginError?.let { throw it }
        return SsoStart("$serverUrl/identity/connect/authorize?domain_hint=$domain&state=$SSO_STATE", "com.reins2fa.app", SSO_STATE, SSO_VERIFIER)
    }

    override suspend fun orgMembership(): dev.reins.core.OrgView? = org

    /** The family this account is in (`familyView`); null: none. */
    @Volatile var family: dev.reins.core.FamilyView? = null
    val familyJoins = CopyOnWriteArrayList<String>()
    val familyConsents = CopyOnWriteArrayList<String>()
    val familyMinorAnswers = CopyOnWriteArrayList<Boolean>()

    override suspend fun familyJoin(link: String): dev.reins.core.FamilyView {
        familyJoins += link
        return family ?: throw CoreException.Invalid("Use a new address for your family account.")
    }

    override suspend fun familyView(): dev.reins.core.FamilyView? = family

    override suspend fun familyRequestLeave(): dev.reins.core.FamilyLeaveInfo {
        val f = family ?: throw CoreException.NotFound()
        val now = System.currentTimeMillis() / 1000
        val leave = dev.reins.core.FamilyLeaveInfo(now, if (f.minor) null else now + f.noticeDays.toLong() * 86_400, f.minor)
        family = f.copy(leave = leave)
        return leave
    }

    override suspend fun familyCancelLeave() {
        family = family?.copy(leave = null)
    }

    override suspend fun familyConsentLeave(member: String) {
        familyConsents += member
        family = family?.let { f -> f.copy(leaves = f.leaves.filterNot { it.member == member }) }
    }

    override suspend fun familyAnswerMinor(accept: Boolean) {
        familyMinorAnswers += accept
        family = family?.let { it.copy(minor = it.minor || accept, minorProposed = false) }
    }

    /** Whether the server offers organizations ("Add work account"). */
    @Volatile var orgsOffered = false

    override suspend fun orgsOffered(serverUrl: String): Boolean = orgsOffered

    /** The tag this account's pushes carry. */
    @Volatile var pushTag: String? = null

    override suspend fun pushTag(): String? = pushTag

    override suspend fun orgChallenge(): String = org?.let { "challenge-1" } ?: throw CoreException.NotFound()

    override suspend fun orgEnroll(challenge: String, input: dev.reins.core.EnrollInput) {
        enrollments += challenge to input
    }

    override suspend fun orgReportPosture(posture: dev.reins.core.PostureInput) {
        postures += posture
    }

    /** The org reviews waiting for this member (`orgReviews`); a Personal account (no [org]) has none. */
    @Volatile var orgReviews: List<dev.reins.core.OrgReviewView> = emptyList()

    /** `orgDecide` calls: (id, approve, at, signature). */
    val orgDecisions = CopyOnWriteArrayList<List<Any>>()
    @Volatile var orgDecideError: CoreException? = null
    /** `orgSetMemberPolicy` calls: (body, version, at, signature). */
    val memberPolicies = CopyOnWriteArrayList<List<Any>>()

    override suspend fun orgReviews(): List<dev.reins.core.OrgReviewView> = if (org == null) emptyList() else orgReviews

    override suspend fun orgDecide(id: String, approve: Boolean, at: Long, signature: String) {
        orgDecideError?.let { throw it }
        if (orgReviews.none { it.id == id }) throw CoreException.NotFound()
        orgDecisions += listOf(id, approve, at, signature)
        orgReviews = orgReviews.filterNot { it.id == id }
    }

    /** The member's web passkeys (`orgPasskeys`); a Personal account (no [org]) has none. */
    @Volatile var orgPasskeys: List<dev.reins.core.OrgPasskeyView> = emptyList()

    /** `orgConfirmPasskey` calls: (id, approve, at, signature). */
    val passkeyConfirmations = CopyOnWriteArrayList<List<Any>>()

    override suspend fun orgPasskeys(): List<dev.reins.core.OrgPasskeyView> = if (org == null) emptyList() else orgPasskeys

    override suspend fun orgConfirmPasskey(id: String, approve: Boolean, at: Long, signature: String) {
        if (orgPasskeys.none { it.credentialId == id }) throw CoreException.NotFound()
        passkeyConfirmations += listOf(id, approve, at, signature)
        orgPasskeys = if (approve) orgPasskeys.map { if (it.credentialId == id) it.copy(confirmed = true) else it } else orgPasskeys.filterNot { it.credentialId == id }
    }

    override suspend fun orgSetMemberPolicy(body: String, version: ULong, at: Long, signature: String) {
        memberPolicies += listOf(body, version, at, signature)
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
        ssoFinishErrorAfterSignIn?.let { throw it }
        return SsoOutcome(info, ssoKeys)
    }

    /** Thrown by `ssoFinish` once the account is open, as a core before the sync merge did on a sync conflict. */
    @Volatile var ssoFinishErrorAfterSignIn: CoreException? = null

    /** The core's sync notice, and how often it was dismissed. */
    @Volatile var syncNotice: String? = null
    val syncNoticeDismissals = java.util.concurrent.atomic.AtomicInteger()
    /** Thrown by [accountSyncNotice] while set. A failed read is not "no notice". */
    @Volatile var syncNoticeError: CoreException? = null

    override suspend fun accountSyncNotice(): String? {
        syncNoticeError?.let { throw it }
        return syncNotice
    }

    override suspend fun restoreAccountSyncBackup(): Boolean = false

    override suspend fun dismissAccountSyncNotice() {
        syncNoticeDismissals.incrementAndGet()
        syncNotice = null
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

    // Synthetic role setup fixtures; no real keys, authority or recovery proof are persisted here.
    @Volatile var setupRole = PhoneRole.LEGACY
    @Volatile var setupRoster: RosterSummary? = null
    @Volatile var setupRequiresProof = false
    @Volatile var setupLostReply = false
    @Volatile var setupAuthorizedGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    val setupCalls = java.util.concurrent.atomic.AtomicInteger()
    val setupCreated = java.util.concurrent.atomic.AtomicInteger()
    val setupFinished = java.util.concurrent.atomic.AtomicInteger()
    val setupProofUsed = java.util.concurrent.atomic.AtomicBoolean()
    fun resetPrimarySetup() {
        setupRole = PhoneRole.LEGACY; setupRoster = null; setupRequiresProof = false; setupLostReply = false; setupAuthorizedGate = null
        setupCalls.set(0); setupCreated.set(0); setupFinished.set(0); setupProofUsed.set(false)
    }
    override suspend fun phoneRole(): PhoneRole = setupRole
    override suspend fun rosterSummary(refresh: Boolean): RosterSummary? = setupRoster

    private val legacyLive = dev.reins.core.LiveSyncState(
        dev.reins.core.PhoneRole.LEGACY, 0u, 0u, 0u, "", null, null, false, null, false, false, 0u, 0u, null,
    )

    override suspend fun liveSync(waitSecs: UInt): dev.reins.core.LiveSyncState = legacyLive

    override suspend fun liveSyncState(): dev.reins.core.LiveSyncState = legacyLive

    override suspend fun proposalCreate(kind: String, title: String, rowsJson: String): String =
        throw CoreException.NotFound()

    override suspend fun proposalWithdraw(id: String): Unit = throw CoreException.NotFound()

    override suspend fun proposals(): List<dev.reins.core.ProposalView> = emptyList()

    override suspend fun proposalPreview(id: String): dev.reins.core.ProposalPreview = throw CoreException.NotFound()

    override suspend fun proposalKeep(id: String, digest: String): Unit = throw CoreException.NotFound()

    override suspend fun proposalDiscard(id: String): Unit = throw CoreException.NotFound()

    override suspend fun rosterCreate(deviceName: String, codeOrPassword: String?): dev.reins.core.RosterSummary =
        throw CoreException.NotFound()

    override suspend fun rosterCreateAuthorized(
        deviceName: String,
        codeOrPassword: String?,
        owner: OwnerCheck,
    ): RosterSummary {
        setupCalls.incrementAndGet()
        try {
            setupAuthorizedGate?.await()
            if (!owner.confirm("Make this the approval phone for your whole Reins account")) throw CoreException.Invalid("Owner did not confirm")
            if (setupRole != PhoneRole.LEGACY || setupRoster != null) throw CoreException.Invalid("This account has phone roles already.")
            setupProofUsed.set(codeOrPassword != null)
            if (codeOrPassword?.isBlank() == true || (setupRequiresProof && codeOrPassword == null)) throw CoreException.Invalid("Enter the recovery code or master password.")
            val result = RosterSummary(1u, PhoneRole.PRIMARY, listOf(RosterMemberView("this-phone", deviceName, true, true)), "fixture-owner", emptyList())
            setupRole = PhoneRole.PRIMARY; setupRoster = result; setupCreated.incrementAndGet()
            permissionViews.replaceAll { _, view ->
                if (view.service == "gmail" && !view.conflicted && view.usageVerified) view.copy(availablePresets = AccountPermissionPreset.entries,
                    capabilities = view.capabilities.map { cap -> cap.copy(supportedModes = PermissionMode.entries) }) else view
            }
            if (setupLostReply) { setupLostReply = false; throw CoreException.Network("The setup response was lost. Check the phone status before retrying.") }
            return result
        } finally { setupFinished.incrementAndGet() }
    }

    // Viewer enrollment. The joining phone follows [enrollScript] (one step per poll, the last one repeats); the
    // approving phone shows [enrollViews] (an id there is an enrollment) and records its answers.
    val enrollBegins = CopyOnWriteArrayList<String>()
    @Volatile var enrollBeginError: CoreException? = null
    val enrollScript = java.util.concurrent.ConcurrentLinkedQueue<dev.reins.core.EnrollProgress>()
    @Volatile var enrollLast: dev.reins.core.EnrollProgress = dev.reins.core.EnrollProgress.Waiting
    val enrollCancels = java.util.concurrent.atomic.AtomicInteger()
    val enrollViews = java.util.concurrent.ConcurrentHashMap<String, dev.reins.core.EnrollView>()
    val enrollAnswers = CopyOnWriteArrayList<Pair<String, Boolean>>()
    val removedMembers = CopyOnWriteArrayList<String>()
    fun resetEnroll() {
        enrollBegins.clear(); enrollBeginError = null; enrollScript.clear(); enrollLast = dev.reins.core.EnrollProgress.Waiting
        enrollCancels.set(0); enrollViews.clear(); enrollAnswers.clear(); removedMembers.clear()
    }

    override suspend fun enrollBegin(deviceName: String): dev.reins.core.EnrollStart {
        enrollBeginError?.let { throw it }
        enrollBegins += deviceName
        enrollLast = dev.reins.core.EnrollProgress.Waiting
        return dev.reins.core.EnrollStart("enroll-1", 0)
    }

    override suspend fun enrollPoll(): dev.reins.core.EnrollProgress {
        enrollScript.poll()?.let { enrollLast = it }
        // Enrolled: the server shows this phone the connections from now on.
        if (enrollLast == dev.reins.core.EnrollProgress.Enrolled) connectionsError = null
        return enrollLast
    }

    override suspend fun enrollCancel() {
        enrollCancels.incrementAndGet()
    }

    override suspend fun enrollView(id: String): dev.reins.core.EnrollView = enrollViews[id] ?: throw CoreException.NotFound()

    override suspend fun enrollConfirm(id: String, approve: Boolean) {
        val view = enrollViews[id] ?: throw CoreException.NotFound()
        if (approve && view.code == null) throw CoreException.Invalid("The other phone has not shown its code yet.")
        enrollAnswers += id to approve
        enrollViews.remove(id)
        pending = pending.filterNot { it.id == id }
    }

    override suspend fun joinIsEnrollment(id: String): Boolean = enrollViews.containsKey(id)

    override suspend fun handoverRequest(): String = throw CoreException.NotFound()

    override suspend fun handoverPoll(): dev.reins.core.HandoverProgress = throw CoreException.NotFound()

    override suspend fun handoverRequests(): List<dev.reins.core.HandoverView> = emptyList()

    override suspend fun handoverApprove(id: String, deviceId: String) = throw CoreException.NotFound()

    override suspend fun handoverDeny(id: String) = throw CoreException.NotFound()

    override suspend fun rosterRecoverPreview(codeOrPassword: String): dev.reins.core.RecoveryPreview =
        throw CoreException.NotFound()

    override suspend fun rosterRecover(
        codeOrPassword: String,
        deviceName: String,
        tip: String,
        confirmation: String?,
    ): dev.reins.core.RosterSummary = throw CoreException.NotFound()

    override suspend fun rosterRemoveMember(deviceId: String) {
        removedMembers += deviceId
        setupRoster = setupRoster?.let { r -> r.copy(members = r.members.filterNot { it.deviceId == deviceId }) }
    }

    override suspend fun rosterRewrap(newPassword: String) = throw CoreException.NotFound()

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
        val view = pairingByCodeResult ?: TestData.pairingView("pair-code", "M2A", "laptop", keyFingerprint = "4821 9930")
        pairing = view
        pending = pending.filterNot { it.id == view.id } + TestData.pairingItem(view.id)
        return view
    }

    override suspend fun pending() = pending

    /** What a mode has still to look at (never shown to the owner). */
    @Volatile var deciding: List<PendingItem> = emptyList()

    override suspend fun deciding() = deciding

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

    val secretsRevoked = mutableListOf<Pair<String?, String?>>()

    /** What "Revoke on all computers" answers. */
    @Volatile var secretsRevokeAnswer = SecretsRevoked(ended = 0u, told = 0u, notDelivered = emptyList())

    override suspend fun revokeSecretsEverywhere(project: String?, env: String?): SecretsRevoked {
        secretsRevoked += project to env
        return secretsRevokeAnswer
    }

    val alwaysAsk = mutableMapOf<Pair<String, String>, Boolean?>()

    override suspend fun setSecretsAlwaysAsk(project: String, env: String, always: Boolean?) {
        alwaysAsk[project to env] = always
    }

    /**
     * Thrown by [session] while set. [sessionThrows] is how many of the next calls throw, after which the read
     * succeeds; 0 throws every time. A throw is a busy or failed store, not "signed out".
     */
    @Volatile var sessionError: CoreException? = null
    @Volatile var sessionThrows: Int = 0

    override suspend fun session(): SessionInfo? {
        val error = sessionError ?: return session
        if (sessionThrows > 0) {
            sessionThrows -= 1
            if (sessionThrows == 0) sessionError = null
        }
        throw error
    }

    /** The foreground poll: returns quickly with what is pending. */
    override suspend fun sync(waitSecs: UInt): List<PendingItem> {
        syncStarts.incrementAndGet()
        kotlinx.coroutines.delay(250)
        // The server refuses phones that are not the approval device (yet).
        if (syncRefusals.getAndUpdate { (it - 1).coerceAtLeast(0) } > 0) throw CoreException.Server(403u, "not the approval device")
        return pending
    }

    /** How many of the next polls the server refuses with 403. */
    val syncRefusals = java.util.concurrent.atomic.AtomicInteger()

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
    @Volatile var mcpAddGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    val mcpAdds = CopyOnWriteArrayList<Pair<String, String?>>()
    val mcpTokenAdds = CopyOnWriteArrayList<List<String?>>()
    val mcpSignIns = CopyOnWriteArrayList<Pair<String, String>>()
    val mcpRefreshes = CopyOnWriteArrayList<String>()
    val mcpRemoved = CopyOnWriteArrayList<String>()
    val mcpHeavy = CopyOnWriteArrayList<Triple<String, String, Boolean>>()

    @Volatile var mcpListFailure: CoreException? = null

    override suspend fun mcpServers(): List<McpServerView> {
        mcpListFailure?.let { throw it }
        return mcp
    }

    override suspend fun mcpAdd(url: String, name: String?): McpAddStep {
        mcpFailure?.let { throw it }
        mcpAdds += url to name
        mcpAddGate?.await()
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
    data class ApRow(
        val mode: AutopilotMode? = null,
        val bypassUntil: Long? = null,
        val profileId: String? = null,
        val bypassSince: Long? = null,
        /** With `mode` Overnight: its end ([Long.MAX_VALUE] without one), its start, and the mode it returns to. */
        val overnightUntil: Long? = null,
        val overnightSince: Long? = null,
        val overnightReturn: AutopilotMode? = null,
    ) {
        fun overnighting(now: Long) = mode == AutopilotMode.OVERNIGHT && (overnightUntil ?: Long.MAX_VALUE) > now

        /** The mode when no bypass runs: an Overnight that ended reads as the mode it interrupted. */
        fun base(now: Long): AutopilotMode? = if (mode == AutopilotMode.OVERNIGHT && !overnighting(now)) overnightReturn else mode
    }

    /** The global row: `mode` null = the default (Manual). */
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
    /** "Update automatically". */
    @Volatile var autoUpdate = true
    /** What `modelUpdatePending` answers; null: as the core (on, an older model installed, its update not rejected). */
    @Volatile var updatePending: Boolean? = null
    /** What `updateModel` answers; null: as the core (the newer model when there is one, else up to date). */
    @Volatile var updateOutcome: dev.reins.core.ModelUpdateOutcome? = null
    /** Thrown by `updateModel` (a dropped connection, a model that failed its check). */
    @Volatile var updateFailure: CoreException? = null
    /** `updateModel` calls: whether each was automatic. */
    val updateCalls = CopyOnWriteArrayList<Boolean>()
    /** `updateModel` downloads until its caller is cancelled (WorkManager stopping the worker). */
    @Volatile var updateHangs = false
    /** `pauseModelDownload` calls. */
    @Volatile var pauses = 0
    /** `setAutopilotAutoUpdate` calls. */
    val autoUpdateCalls = CopyOnWriteArrayList<Boolean>()

    val modeCalls = CopyOnWriteArrayList<Triple<String?, AutopilotMode?, UInt?>>()
    val classLocks = CopyOnWriteArrayList<Triple<String, String, Boolean?>>()
    val presets = CopyOnWriteArrayList<Pair<String, Preset>>()
    val thresholdCalls = CopyOnWriteArrayList<Triple<String, Float, Float>>()
    /** `startBypass` calls: the connection (null: every AI) and the minutes (null: until turned off). */
    val bypassCalls = CopyOnWriteArrayList<Pair<String?, UInt?>>()
    /** `startOvernight` calls, like [bypassCalls]. */
    val overnightCalls = CopyOnWriteArrayList<Pair<String?, UInt?>>()
    /** The owner checks handed to `startOvernight`, refused or not. */
    val overnightOwners = CopyOnWriteArrayList<dev.reins.core.OwnerCheck>()
    /** How long what Overnight parks waits past the morning, and the `setOvernightExpiry` calls. */
    @Volatile var overnightExpiry: UInt = 12u
    val expiryCalls = CopyOnWriteArrayList<UInt>()
    /** What `overnightDigest` answers; reading it throws [digestFailure] instead when set. */
    @Volatile var digest: dev.reins.core.OvernightDigest? = null
    @Volatile var digestFailure: CoreException? = null
    /** What `tasks` lists (live first, then those ended in the last day). */
    @Volatile var tasks: List<dev.reins.core.TaskView> = emptyList()
    /** `approveAll` calls: (connection, task). */
    val approveAllCalls = CopyOnWriteArrayList<Pair<String, String?>>()
    /** `startTask`, `endTask` and `approveTask` calls. */
    val startedTasks = CopyOnWriteArrayList<dev.reins.core.TaskView>()
    val endedTasks = CopyOnWriteArrayList<String>()
    val approvedTasks = CopyOnWriteArrayList<Pair<String, dev.reins.core.TaskView>>()
    /** The owner checks handed to `startTask` and `approveTask`. */
    val taskOwners = CopyOnWriteArrayList<dev.reins.core.OwnerCheck>()
    @Volatile var stats = dev.reins.core.AutopilotStats(7u, 142u, 0.94f, 120u, 2u, 9u)
    /** What `thresholdReplay` answers; null: numbers that move with the thresholds. */
    @Volatile var replay: dev.reins.core.ThresholdReplay? = null
    val corrections = CopyOnWriteArrayList<Pair<Long, Verdict>>()
    val evaluations = CopyOnWriteArrayList<Pair<String?, String>>()
    val assigned = CopyOnWriteArrayList<Pair<String, String?>>()
    val profileCalls = CopyOnWriteArrayList<String>()
    val downloads = java.util.concurrent.atomic.AtomicInteger()

    /** The clock the screens use: frozen in tests. */
    private fun now(): Long = (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000

    fun resetAutopilot() {
        profilesGate = null
        apGlobal = ApRow()
        apConnections.clear()
        model = TestData.modelStatus()
        profiles = TestData.profiles()
        wifiOnly = true
        suggestions.clear()
        views.clear()
        extended.clear()
        evaluation = null
        modeFailure = null
        downloadFailure = null
        downloadError = null
        autoUpdate = true
        updatePending = null
        updateOutcome = null
        updateFailure = null
        updateCalls.clear()
        autoUpdateCalls.clear()
        listOf(modeCalls, classLocks, presets, corrections, evaluations, assigned, profileCalls, thresholdCalls, bypassCalls, overnightCalls, expiryCalls, overnightOwners, approveAllCalls, startedTasks, endedTasks, approvedTasks, taskOwners).forEach { it.clear() }
        overnightExpiry = 12u
        digest = null
        digestFailure = null
        tasks = emptyList()
        stats = dev.reins.core.AutopilotStats(7u, 142u, 0.94f, 120u, 2u, 9u)
        replay = null
        downloads.set(0)
        resetDecisions()
    }

    private fun defaultProfile() = profiles.firstOrNull { it.isDefault } ?: profiles.first()

    private fun installed() = model.state == ModelState.INSTALLED

    private fun globalBase() = apGlobal.base(now()) ?: AutopilotMode.MANUAL

    private fun globalMode(): AutopilotMode = when {
        apGlobal.mode == AutopilotMode.LOCKDOWN -> AutopilotMode.LOCKDOWN
        (apGlobal.bypassUntil ?: 0) > now() -> AutopilotMode.BYPASS
        else -> globalBase()
    }

    override fun setModelRuntime(runtime: ModelRuntime) {
        this.runtime = runtime
    }

    /** The fake does not load a model either way. */
    override fun setBackground(background: Boolean) = Unit

    override suspend fun autopilotSettings(): AutopilotSettings {
        val now = now()
        return AutopilotSettings(
            mode = globalMode(),
            baseMode = globalBase(),
            bypassUntil = apGlobal.bypassUntil?.takeIf { it > now },
            bypassSince = apGlobal.bypassSince.takeIf { (apGlobal.bypassUntil ?: 0) > now },
            overnightUntil = (apGlobal.overnightUntil ?: Long.MAX_VALUE).takeIf { apGlobal.overnighting(now) },
            overnightSince = apGlobal.overnightSince.takeIf { apGlobal.overnighting(now) },
            overnightExpiryHours = overnightExpiry,
            defaultProfileId = defaultProfile().id,
            wifiOnly = wifiOnly,
            autoUpdate = autoUpdate,
            model = model,
            connections = apConnections.map { (id, row) ->
                // `modes::effective`: lockdowns, bypasses, its Overnight, a global one unless kept on Manual, its mode.
                val mode = when {
                    globalMode() == AutopilotMode.LOCKDOWN || row.mode == AutopilotMode.LOCKDOWN -> AutopilotMode.LOCKDOWN
                    (row.bypassUntil ?: 0) > now || (apGlobal.bypassUntil ?: 0) > now -> AutopilotMode.BYPASS
                    row.overnighting(now) -> AutopilotMode.OVERNIGHT
                    apGlobal.overnighting(now) && row.base(now) != AutopilotMode.MANUAL -> AutopilotMode.OVERNIGHT
                    else -> row.base(now) ?: globalMode()
                }
                ConnectionAutopilot(
                    connectionId = id,
                    baseMode = row.base(now),
                    bypassUntil = row.bypassUntil?.takeIf { it > now },
                    bypassSince = row.bypassSince.takeIf { (row.bypassUntil ?: 0) > now },
                    overnightUntil = (row.overnightUntil ?: Long.MAX_VALUE).takeIf { row.overnighting(now) },
                    overnightSince = row.overnightSince.takeIf { row.overnighting(now) },
                    mode = mode,
                    profileId = row.profileId ?: defaultProfile().id,
                )
            },
        )
    }

    override suspend fun setAutopilotMode(connectionId: String?, mode: AutopilotMode?, minutes: UInt?) {
        modeFailure?.let { throw it }
        // As the core: Overnight starts only through startOvernight, after the owner check.
        if (mode == AutopilotMode.OVERNIGHT) throw CoreException.Invalid("Overnight starts with start_overnight, after the owner check")
        modeCalls += Triple(connectionId, mode, minutes)
        val row = if (connectionId == null) apGlobal else apConnections[connectionId] ?: ApRow()
        val next = if (mode == AutopilotMode.BYPASS) {
            row.copy(mode = row.mode.takeIf { it != AutopilotMode.LOCKDOWN }, bypassUntil = now() + (minutes ?: 15u).toLong() * 60, bypassSince = now())
        } else {
            row.copy(mode = mode, bypassUntil = null, bypassSince = null, overnightUntil = null, overnightSince = null, overnightReturn = null)
        }
        if (connectionId == null) apGlobal = next else apConnections[connectionId] = next
    }

    override suspend fun startOvernight(connectionId: String?, minutes: UInt?, owner: dev.reins.core.OwnerCheck) {
        overnightOwners += owner
        modeFailure?.let { throw it }
        if (minutes != null && (minutes == 0u || minutes > 525_600u)) throw CoreException.Invalid("Overnight lasts between 1 minute and 365 days")
        // The core's own words for the prompt, then nothing changes without a pass.
        val reason = when {
            minutes == null -> "Start Overnight until you turn it off"
            minutes % 60u == 0u -> "Start Overnight for ${minutes / 60u} h"
            else -> "Start Overnight for $minutes min"
        }
        if (!owner.confirm(reason)) throw CoreException.Invalid("the owner did not confirm")
        overnightCalls += connectionId to minutes
        modeCalls += Triple(connectionId, AutopilotMode.OVERNIGHT, minutes)
        val row = if (connectionId == null) apGlobal else apConnections[connectionId] ?: ApRow()
        val back = when (row.mode) {
            AutopilotMode.LOCKDOWN -> null
            AutopilotMode.OVERNIGHT -> row.overnightReturn
            else -> row.mode
        }
        val until = if (minutes == null) Long.MAX_VALUE else now() + minutes.toLong() * 60
        val next = row.copy(mode = AutopilotMode.OVERNIGHT, overnightUntil = until, overnightSince = now(), overnightReturn = back)
        if (connectionId == null) apGlobal = next else apConnections[connectionId] = next
    }

    override suspend fun setOvernightExpiry(hours: UInt) {
        // As the core: 1 to 48 hours, and no owner check (it never loosens).
        if (hours == 0u || hours > 48u) throw CoreException.Invalid("between 1 and 48 hours")
        expiryCalls += hours
        overnightExpiry = hours
    }

    override suspend fun overnightDigest(): dev.reins.core.OvernightDigest? {
        digestFailure?.let { throw it }
        return digest
    }

    /** As the core: only the batchable ones of that agent's task (or none); the rest stay. */
    override suspend fun approveAll(connectionId: String, taskId: String?): UInt {
        approveAllCalls += connectionId to taskId
        val taken = pending.filter { it.connectionId == connectionId && it.taskId == taskId && it.batchable }
        pending = pending - taken.toSet()
        return taken.size.toUInt()
    }

    override suspend fun tasks(): List<dev.reins.core.TaskView> = tasks

    override suspend fun startTask(draft: dev.reins.core.TaskView, owner: dev.reins.core.OwnerCheck): dev.reins.core.TaskView {
        taskOwners += owner
        if (!owner.confirm("Start a task for ${draft.connectionLabel}")) throw CoreException.Invalid("the owner did not confirm")
        val task = draft.copy(id = "task${startedTasks.size + 1}", startedBy = "you")
        startedTasks += task
        tasks = listOf(task) + tasks.map { if (it.connectionId == draft.connectionId && it.endedAt == null) it.copy(endedAt = now()) else it }
        return task
    }

    override suspend fun endTask(id: String) {
        endedTasks += id
        tasks = tasks.map { if (it.id == id && it.endedAt == null) it.copy(endedAt = now()) else it }
    }

    override suspend fun approveTask(requestId: String, draft: dev.reins.core.TaskView, owner: dev.reins.core.OwnerCheck) {
        taskOwners += owner
        if (!owner.confirm("Approve the task \"${draft.title}\"")) throw CoreException.Invalid("the owner did not confirm")
        approvedTasks += requestId to draft
        pending = pending.filterNot { it.id == requestId }
        tasks = listOf(draft.copy(id = "task-$requestId")) + tasks
    }

    override suspend fun startBypass(connectionId: String?, minutes: UInt?) {
        modeFailure?.let { throw it }
        if (minutes != null && (minutes == 0u || minutes > 525_600u)) throw CoreException.Invalid("a bypass lasts between 1 minute and 365 days")
        bypassCalls += connectionId to minutes
        modeCalls += Triple(connectionId, AutopilotMode.BYPASS, minutes)
        val row = if (connectionId == null) apGlobal else apConnections[connectionId] ?: ApRow()
        val until = if (minutes == null) Long.MAX_VALUE else now() + minutes.toLong() * 60
        // As the core: a bypass ends the Overnight of its scope, so it never returns to it without the owner.
        val base = if (row.mode == AutopilotMode.OVERNIGHT) row.overnightReturn else row.mode.takeIf { it != AutopilotMode.LOCKDOWN }
        val next = row.copy(mode = base, bypassUntil = until, bypassSince = now(), overnightUntil = null, overnightSince = null, overnightReturn = null)
        if (connectionId == null) apGlobal = next else apConnections[connectionId] = next
    }

    override suspend fun autopilotStats(days: UInt) = stats.copy(days = days)

    override suspend fun setThresholds(profileId: String, approveAt: Float, denyAt: Float) {
        val clamp = { x: Float -> Math.round(x.coerceIn(0.80f, 0.99f) * 100) / 100f }
        thresholdCalls += Triple(profileId, clamp(approveAt), clamp(denyAt))
        profiles = profiles.map { if (it.id == profileId) it.copy(preset = Preset.CUSTOM, approveAt = clamp(approveAt), denyAt = clamp(denyAt)) else it }
    }

    override suspend fun thresholdReplay(profileId: String, approveAt: Float, denyAt: Float): dev.reins.core.ThresholdReplay {
        replay?.let { return it }
        // Lower thresholds decide more of the same 53 requests.
        val decided = (41 + ((0.93f - approveAt) * 100).toInt() * 2 + ((0.88f - denyAt) * 100).toInt()).coerceIn(0, 53)
        return dev.reins.core.ThresholdReplay(30u, decided.toUInt(), (53 - decided).toUInt(), if (approveAt < 0.86f) 1u else 0u)
    }

    override suspend fun setAutopilotWifiOnly(wifiOnly: Boolean) {
        this.wifiOnly = wifiOnly
    }

    @Volatile var profilesGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    override suspend fun autopilotProfiles(): List<ProfileView> {
        profilesGate?.await()
        return profiles
    }

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
                // as the core: a lock wins, a trust by hand clears a pause, and otherwise Auto runs unless paused
                p.copy(classes = p.classes.map { c -> if (c.classKey == classKey) c.copy(manual = locked?.not(), autoApprove = locked == false || (locked == null && !c.paused), paused = locked == null && c.paused) else c })
            }
        }
    }

    override suspend fun setPreset(profileId: String, preset: Preset) {
        presets += profileId to preset
        profiles = profiles.map {
            when {
                it.id != profileId -> it
                preset == Preset.CUSTOM -> it.copy(preset = preset)
                else -> TestData.thresholds(preset).let { (a, d) -> it.copy(preset = preset, approveAt = a, denyAt = d) }
            }
        }
    }

    override suspend fun autopilotSuggestion(requestId: String): SuggestionView? = suggestions[requestId]

    override suspend fun correctDecision(activityId: Long, shouldHave: Verdict) {
        corrections += activityId to shouldHave
        activity = activity.map { e -> if (e.id == activityId) e.copy(autopilot = e.autopilot?.copy(correctable = false)) else e }
    }

    // ---- Decisions v2: Auto's setup, questions, Ask me next time --------------------------------------------------

    /** The setup was finished: picking Auto sets the mode at once. Tests of the setup turn it off. */
    @Volatile var autoSetupDone = true
    @Volatile var matrixDecides = false
    @Volatile var world: List<WorldItem> = emptyList()
    /** Rows Mine, Friends, Work, Outside; cells L0..L3 (01 §4's defaults). */
    @Volatile var matrix: List<List<CellChoice>> = defaultMatrix()
    @Volatile var autoQuestionList: List<AutoQuestion> = emptyList()
    /** `setWorldZone` calls that changed something: kind, key, zone. */
    val worldZoneCalls = CopyOnWriteArrayList<Triple<FactChoice, String, ZoneChoice>>()
    val matrixCellCalls = CopyOnWriteArrayList<Triple<ZoneChoice, UByte, CellChoice>>()
    val finishSetupCalls = java.util.concurrent.atomic.AtomicInteger()
    val answeredQuestions = CopyOnWriteArrayList<Pair<String, ZoneChoice>>()
    val dismissedQuestions = CopyOnWriteArrayList<String>()
    val askedNextTime = CopyOnWriteArrayList<String>()
    /** What a preview says (Decisions v2 O4), and the changes previewed. */
    @Volatile var previewLine: String = "Last 30 days: 0 Auto → Ask, 4 Ask → Auto"
    val previewCalls = CopyOnWriteArrayList<dev.reins.core.PolicyChange>()
    /** Every owner check the setup and the questions asked with, refused or not (their reasons). */
    val autoOwnerReasons = CopyOnWriteArrayList<String>()

    fun resetDecisions() {
        autoSetupDone = true
        matrixDecides = false
        world = emptyList()
        matrix = defaultMatrix()
        autoQuestionList = emptyList()
        listOf(worldZoneCalls, matrixCellCalls, answeredQuestions, dismissedQuestions, askedNextTime, autoOwnerReasons).forEach { it.clear() }
        previewCalls.clear()
        finishSetupCalls.set(0)
        resetFolders()
    }

    // ---- A computer's folders: proposed by it, signed by this phone -------------------------------------------------

    /** A folder a computer proposes: which computer (its id and name), the folder, and when (Unix seconds). */
    data class FolderProposal(val connectionId: String, val computer: String, val folder: FolderView, val at: Long)

    /** What the computers propose now; confirmed (signed) folders per computer; and those dismissed with "Not now". */
    @Volatile var folderProposals: List<FolderProposal> = emptyList()
    @Volatile var confirmedFolders: Map<String, List<FolderView>> = emptyMap()
    @Volatile var dismissedFolders: Set<Pair<String, FolderView>> = emptySet()
    /** The calls that changed something: connection id and folder. */
    val folderConfirmCalls = CopyOnWriteArrayList<Pair<String, FolderView>>()
    val folderDismissCalls = CopyOnWriteArrayList<Pair<String, FolderView>>()
    val folderRemoveCalls = CopyOnWriteArrayList<Pair<String, FolderView>>()
    val computerFoldersReads = java.util.concurrent.atomic.AtomicInteger()

    fun resetFolders() {
        folderProposals = emptyList()
        confirmedFolders = emptyMap()
        dismissedFolders = emptySet()
        listOf(folderConfirmCalls, folderDismissCalls, folderRemoveCalls).forEach { it.clear() }
        computerFoldersReads.set(0)
    }

    private fun confirmedOf(connectionId: String) = confirmedFolders[connectionId].orEmpty()

    private fun proposed(connectionId: String, folder: FolderView) =
        folderProposals.any { it.connectionId == connectionId && it.folder == folder } && folder !in confirmedOf(connectionId)

    override suspend fun folderQuestions(): List<FolderQuestion> = folderProposals
        .filter { proposed(it.connectionId, it.folder) && (it.connectionId to it.folder) !in dismissedFolders }
        .sortedByDescending { it.at }
        .map { FolderQuestion(it.connectionId, it.computer, it.folder.path, it.folder.kind, it.at) }

    override suspend fun computerFolders(connectionId: String): ComputerFolders {
        computerFoldersReads.incrementAndGet()
        val suggested = folderProposals.filter { it.connectionId == connectionId && proposed(connectionId, it.folder) }.map { it.folder }
        return ComputerFolders(connectionId, confirmedOf(connectionId), suggested)
    }

    override suspend fun confirmComputerFolder(connectionId: String, path: String, kind: FolderKindChoice, owner: OwnerCheck) {
        val folder = FolderView(path, kind)
        // As the core: only what the computer proposes now, and it loosens, so the owner is asked first.
        if (!proposed(connectionId, folder)) throw CoreException.NotFound()
        ownerConfirms(owner, "Trust $path as ${if (kind == FolderKindChoice.ROOT) "projects" else "scratch"}")
        folderConfirmCalls += connectionId to folder
        dismissedFolders = dismissedFolders - (connectionId to folder)
        confirmedFolders = confirmedFolders + (connectionId to (confirmedOf(connectionId) + folder))
    }

    override suspend fun dismissComputerFolder(connectionId: String, path: String, kind: FolderKindChoice) {
        val folder = FolderView(path, kind)
        if (!proposed(connectionId, folder)) throw CoreException.NotFound()
        folderDismissCalls += connectionId to folder
        dismissedFolders = dismissedFolders + (connectionId to folder)
    }

    override suspend fun removeComputerFolder(connectionId: String, path: String, kind: FolderKindChoice) {
        val folder = FolderView(path, kind)
        if (folder !in confirmedOf(connectionId)) throw CoreException.NotFound()
        folderRemoveCalls += connectionId to folder
        confirmedFolders = confirmedFolders + (connectionId to (confirmedOf(connectionId) - folder))
    }

    private fun defaultMatrix(): List<List<CellChoice>> = listOf(
        listOf(CellChoice.AUTO, CellChoice.AUTO, CellChoice.AUTO, CellChoice.ASK),
        listOf(CellChoice.AUTO, CellChoice.ASK_FIRST, CellChoice.AUTO, CellChoice.ASK),
        listOf(CellChoice.AUTO, CellChoice.ASK, CellChoice.ASK, CellChoice.ASK),
        listOf(CellChoice.ASK, CellChoice.ASK, CellChoice.ASK, CellChoice.ASK),
    )

    private suspend fun ownerConfirms(owner: OwnerCheck, reason: String) {
        autoOwnerReasons += reason
        if (!owner.confirm(reason)) throw CoreException.Invalid("the owner did not confirm")
    }

    private fun zoneLabel(z: ZoneChoice) = z.name.lowercase().replaceFirstChar { it.uppercase() }

    private fun strictness(c: CellChoice) = when (c) {
        CellChoice.AUTO -> 0
        CellChoice.ASK_FIRST -> 1
        CellChoice.ASK -> 2
    }

    override suspend fun autoSetup(): AutoSetup {
        val zones = listOf(ZoneChoice.MINE, ZoneChoice.FRIENDS, ZoneChoice.WORK, ZoneChoice.OUTSIDE)
        val rows = zones.mapIndexed { i, z ->
            val names = world.filter { it.zone == z }.sortedWith(compareByDescending<WorldItem> { it.count }.thenBy { it.label }).take(3).map { it.label }
            MatrixRowView(z, matrix[i], names)
        }
        return AutoSetup(autoSetupDone, matrixDecides, world, rows, autoQuestionList.size.toUInt())
    }

    override suspend fun setWorldZone(kind: FactChoice, key: String, zone: ZoneChoice, owner: OwnerCheck) {
        // As the core: toward Mine (looser than where it is; an unknown thing is Outside) asks the owner first.
        val current = world.firstOrNull { it.kind == kind && it.key == key && it.confirmed }?.zone ?: ZoneChoice.OUTSIDE
        if (zone.ordinal < current.ordinal) ownerConfirms(owner, "Treat $key as ${zoneLabel(zone)}")
        worldZoneCalls += Triple(kind, key, zone)
        world = if (world.any { it.kind == kind && it.key == key }) {
            world.map { if (it.kind == kind && it.key == key) it.copy(zone = zone, confirmed = true) else it }
        } else {
            world + WorldItem(kind, key, key, if (kind == FactChoice.OWNER || kind == FactChoice.REPO) "github" else "email", zone, true, 0u)
        }
        autoQuestionList = autoQuestionList.filterNot { it.kind == kind && it.key == key }
    }

    override suspend fun setMatrixCell(zone: ZoneChoice, level: UByte, cell: CellChoice, owner: OwnerCheck) {
        if (level > 3u) throw CoreException.Invalid("a level is 0 to 3")
        val row = zone.ordinal
        if (strictness(cell) < strictness(matrix[row][level.toInt()])) ownerConfirms(owner, "Let Auto do more in ${zoneLabel(zone)}")
        matrixCellCalls += Triple(zone, level, cell)
        matrix = matrix.mapIndexed { i, cells -> if (i == row) cells.mapIndexed { l, c -> if (l == level.toInt()) cell else c } else cells }
    }

    override suspend fun finishAutoSetup(owner: OwnerCheck) {
        ownerConfirms(owner, "Turn on Auto")
        finishSetupCalls.incrementAndGet()
        world = world.map { it.copy(confirmed = true) }
        autoSetupDone = true
        setAutopilotMode(null, AutopilotMode.AUTO, null)
    }

    override suspend fun setMatrixDecides(on: Boolean, owner: OwnerCheck) {
        if (on && !matrixDecides) ownerConfirms(owner, "Let the matrix decide")
        matrixDecides = on
    }

    override suspend fun autoQuestions(): List<AutoQuestion> = autoQuestionList.sortedByDescending { it.at }

    override suspend fun answerAutoQuestion(id: String, zone: ZoneChoice, owner: OwnerCheck) {
        val q = autoQuestionList.firstOrNull { it.id == id } ?: throw CoreException.NotFound()
        setWorldZone(q.kind, q.key, zone, owner)
        answeredQuestions += id to zone
        autoQuestionList = autoQuestionList.filterNot { it.id == id }
    }

    override suspend fun dismissAutoQuestion(id: String) {
        if (autoQuestionList.none { it.id == id }) throw CoreException.NotFound()
        dismissedQuestions += id
        autoQuestionList = autoQuestionList.filterNot { it.id == id }
    }

    override suspend fun askMeNextTime(requestId: String) {
        askedNextTime += requestId
    }

    override suspend fun previewPolicyChange(change: dev.reins.core.PolicyChange): dev.reins.core.PreviewView {
        previewCalls += change
        return dev.reins.core.PreviewView(previewLine, 4u, 30u)
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
        // An update replaces the older model with the newer one, which rates risk.
        model = model.copy(
            state = ModelState.INSTALLED,
            downloadedBytes = 0u,
            sizeBytes = 412_000_000u,
            risk = model.risk || model.update,
            update = false,
        )
        return model
    }

    override suspend fun updateModel(automatic: Boolean, progress: DownloadProgress): dev.reins.core.ModelUpdateOutcome {
        updateCalls += automatic
        if (updateHangs) kotlinx.coroutines.awaitCancellation()
        updateFailure?.let { throw it }
        updateOutcome?.let { return it }
        return when {
            automatic && !autoUpdate -> dev.reins.core.ModelUpdateOutcome.OFF
            !installed() -> dev.reins.core.ModelUpdateOutcome.NOT_INSTALLED
            !model.update -> dev.reins.core.ModelUpdateOutcome.UP_TO_DATE
            else -> {
                progress.progress(412_000_000u, 412_000_000u)
                model = model.copy(state = ModelState.INSTALLED, version = "2", risk = true, update = false, updatedAt = now())
                dev.reins.core.ModelUpdateOutcome.UPDATED
            }
        }
    }

    override suspend fun modelUpdatePending(): Boolean = updatePending ?: (autoUpdate && installed() && model.update)

    override suspend fun pauseModelDownload() {
        pauses++
    }

    override suspend fun setAutopilotAutoUpdate(on: Boolean) {
        autoUpdateCalls += on
        autoUpdate = on
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

    // ---- devices ----

    @Volatile var devices: List<DeviceView> = TestData.devices()
    @Volatile var devicesError: CoreException? = null
    val signedOutDevices = CopyOnWriteArrayList<String>()

    fun resetDevices() {
        devices = TestData.devices()
        devicesError = null
        signedOutDevices.clear()
    }

    override suspend fun devices(): List<DeviceView> {
        devicesError?.let { throw it }
        return devices
    }

    /** The proof `signOutDevice` takes (the recovery code or master password typed then). */
    @Volatile var signOutProof = RECOVERY_CODE

    override suspend fun signOutDevice(deviceId: String, codeOrPassword: String) {
        if (codeOrPassword != signOutProof) throw CoreException.Invalid("That is neither the recovery code nor the master password.")
        signedOutDevices += deviceId
        devices = devices.filterNot { it.id == deviceId }
    }

    // ---- the vault on the phone ----

    /** The vault's items and their details; secrets are in [vaultSecrets] by "id/key". */
    @Volatile var vaultDetails: List<VaultItemDetail> = TestData.vaultItems()
    @Volatile var vaultSecrets: Map<String, String> = TestData.vaultSecrets()
    @Volatile var vaultError: CoreException? = null
    val vaultCreated = CopyOnWriteArrayList<VaultItemInput>()
    val vaultUpdated = CopyOnWriteArrayList<Pair<String, VaultItemInput>>()
    val vaultDeleted = CopyOnWriteArrayList<String>()
    val vaultRevealed = CopyOnWriteArrayList<String>()

    fun resetVault() {
        vaultDetails = TestData.vaultItems()
        vaultSecrets = TestData.vaultSecrets()
        vaultError = null
        vaultCreated.clear()
        vaultUpdated.clear()
        vaultDeleted.clear()
        vaultRevealed.clear()
    }

    override suspend fun vaultItems(query: String): List<VaultItemSummary> {
        vaultError?.let { throw it }
        return vaultDetails
            .filter { query.isBlank() || it.name.contains(query, ignoreCase = true) }
            .sortedBy { it.name.lowercase() }
            .map { d -> VaultItemSummary(d.id, d.name, d.kind, d.fields.firstOrNull { !it.secret }?.value.orEmpty(), d.favorite) }
    }

    override suspend fun vaultItem(id: String): VaultItemDetail {
        vaultError?.let { throw it }
        return vaultDetails.firstOrNull { it.id == id } ?: throw CoreException.Service("That item is no longer in the vault.")
    }

    override suspend fun vaultReveal(id: String, key: String): String {
        vaultRevealed += "$id/$key"
        return vaultSecrets["$id/$key"] ?: throw CoreException.Service("That item has no such field.")
    }

    override suspend fun vaultCreate(input: VaultItemInput): String {
        vaultError?.let { throw it }
        vaultCreated += input
        val id = "new-${vaultCreated.size}"
        vaultDetails = vaultDetails + VaultItemDetail(
            id, input.name, input.kind,
            input.fields.map { VaultField(it.key, it.key, if (it.key == "password" || it.key == "notes") null else it.value, it.key == "password" || it.key == "notes", false) },
            listOf(VaultUse("vault:${input.name}/password", "reins run --env, and secret = in an [[api]] block")),
            null, false,
        )
        return id
    }

    override suspend fun vaultUpdate(id: String, input: VaultItemInput) {
        vaultError?.let { throw it }
        vaultUpdated += id to input
        vaultDetails = vaultDetails.map { if (it.id == id) it.copy(name = input.name) else it }
    }

    override suspend fun vaultDelete(id: String) {
        vaultDeleted += id
        vaultDetails = vaultDetails.filterNot { it.id == id }
    }

    override suspend fun vaultGenerateSshKey(name: String): VaultSshKey {
        val public = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForTestsOnly0000000000000000000000 $name"
        val fingerprint = "SHA256:fakefingerprint0000000000000000000000000000"
        val id = "ssh-${vaultDetails.size}"
        vaultDetails = vaultDetails + VaultItemDetail(
            id, name, VaultItemKind.SSH_KEY,
            listOf(
                VaultField("private_key", "Private key", null, true, true),
                VaultField("public_key", "Public key", public, false, true),
                VaultField("fingerprint", "Fingerprint", fingerprint, false, false),
            ),
            listOf(VaultUse(fingerprint, "Offered by the SSH agent of the desktop app (reins ssh setup); the private key stays on the phone")),
            null, false,
        )
        return VaultSshKey(id, public, fingerprint)
    }

    @Volatile var phoneKeyDigits = "4821-9930-1274"

    override suspend fun phoneKeyFingerprint() = phoneKeyDigits

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
            ServiceView("payments", "Payments", "payments", true, null, emptyList()),
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
