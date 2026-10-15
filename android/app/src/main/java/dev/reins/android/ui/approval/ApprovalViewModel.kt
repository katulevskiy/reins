package dev.reins.android.ui.approval

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import dev.reins.android.autopilot.DownloadJob
import dev.reins.core.ApprovalKind
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.ApprovalView
import dev.reins.core.LimitPeriod
import dev.reins.core.PurchaseChoice
import dev.reins.core.PurchaseView
import dev.reins.core.SpendLimitInput
import dev.reins.core.SuggestionView
import dev.reins.android.ui.payments.Money
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class ApprovalUi(
    val loading: Boolean = true,
    val view: ApprovalView? = null,
    val draft: ApprovalDraft = ApprovalDraft(),
    val busy: Boolean = false,
    val error: String? = null,
    val finished: Boolean = false,
    /** The "More options" section is open. */
    val moreOpen: Boolean = false,
    /** What Autopilot made of the request, if it looked at it. */
    val suggestion: SuggestionView? = null,
    /** Autopilot's on-device model: without one there is no score, and the screen offers the download. */
    val model: ModelStatus? = null,
    /** The download job (waiting for its network, running, or idle). */
    val downloadJob: DownloadJob = DownloadJob.Idle,
    /** A purchase: what pays and where it goes, as picked on the purchase screen. */
    val purchase: PurchaseDraft = PurchaseDraft(),
    /** An Activity entry's request shown again as it was decided: nothing can be changed or answered. */
    val readOnly: Boolean = false,
)

/** The purchase screen's picks, and a spend limit for more like it. */
data class PurchaseDraft(
    val methodId: String? = null,
    val addressId: String? = null,
    val limit: Boolean = false,
    /** Up to this a purchase, and this a day, for [limitDays] days (text as typed). */
    val limitPerPurchase: String = "",
    val limitPerDay: String = "",
    val limitDays: Int = 7,
) {
    companion object {
        /** What the view picked (the AI's choice or the user's defaults), else the first that can pay. */
        fun of(p: PurchaseView): PurchaseDraft {
            val usable = p.methods.filter { it.unavailable == null }
            val method = p.methodId?.takeIf { id -> usable.any { it.id == id } } ?: usable.firstOrNull()?.id
            val whole = Money.plain(p.totalMinor, p.currency).substringBefore('.')
            return PurchaseDraft(
                methodId = method,
                addressId = p.addressId ?: p.addresses.firstOrNull()?.id,
                limitPerPurchase = (whole.toLongOrNull()?.plus(1) ?: 1).toString(),
                limitPerDay = ((whole.toLongOrNull() ?: 0) * 2 + 2).toString(),
            )
        }
    }
}

/**
 * A request waiting for the user; or, with [recorded], an Activity entry's request as its approval showed it, read-only
 * with [released] (what was shared) ticked.
 */
class ApprovalViewModel(
    private val container: AppContainer,
    private val requestId: String,
    recorded: ApprovalView? = null,
    released: Set<String> = emptySet(),
) : ViewModel() {
    private val _ui = MutableStateFlow(ApprovalUi())
    val ui: StateFlow<ApprovalUi> = _ui.asStateFlow()

    /** The user changed the draft (a tick, a period): a fresh read of the request keeps it. */
    @Volatile private var touched = false

    private var polling: Job? = null

    init {
        if (recorded != null) {
            val shown = opened(recorded)
            _ui.value = shown.copy(readOnly = true, draft = shown.draft.copy(selected = released))
        } else {
            load()
        }
    }

    private fun load() {
        // Read ahead when the request arrived: the sheet opens with it, no spinner.
        container.approvalViews[requestId]?.let { _ui.value = opened(it) }
        // The model as the shared Autopilot settings know it; a download started here or elsewhere shows.
        viewModelScope.launch {
            container.state.autopilot.collect { s -> if (s != null && polling?.isActive != true) _ui.update { it.copy(model = s.model) } }
        }
        viewModelScope.launch {
            container.modelDownloads.job.collect { job ->
                _ui.update { it.copy(downloadJob = job) }
                if (job != DownloadJob.Idle) pollModel()
            }
        }
        // Until the shared settings are read, the model's own status says whether there is one.
        viewModelScope.launch {
            val model = try {
                container.core.modelStatus()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                null
            }
            if (model != null) _ui.update { if (it.model == null) it.copy(model = model) else it }
        }
        viewModelScope.launch {
            try {
                val view = container.core.approvalView(requestId)
                // The fresh copy wins (repeats may have changed), without undoing what the user already changed.
                _ui.update { if (touched || it.busy) it.copy(view = view) else opened(view).copy(moreOpen = it.moreOpen, model = it.model, downloadJob = it.downloadJob, suggestion = it.suggestion) }
                loadSuggestion()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // A copy read ahead stays on screen, with what is wrong under it.
                _ui.update { if (it.view == null) ApprovalUi(loading = false, error = e.userMessage()) else it.copy(error = e.userMessage()) }
            }
        }
    }

    private fun opened(view: ApprovalView): ApprovalUi {
        // Everything found starts ticked: approving is one tap, and unticking is how you hold something back.
        val preselected = when (view.kind) {
            ApprovalKind.GRANT, ApprovalKind.SEND, ApprovalKind.WRITE -> emptySet()
            ApprovalKind.ACCOUNTS -> shareableAccounts(view).toSet()
            // A code or a password is never ticked for the user.
            else -> view.messages.filter { !it.sensitive }.map { it.id }.toSet()
        }
        // Showing the accounts is remembered for a month unless the user says otherwise.
        val lifetime = if (view.kind == ApprovalKind.ACCOUNTS) LifetimeKind.MONTH else LifetimeKind.ONCE
        return ApprovalUi(
            loading = false,
            view = view,
            draft = ApprovalDraft(selected = preselected, lifetime = lifetime, resources = defaultResources(view), classes = defaultClasses(view)),
            purchase = view.purchase?.let(PurchaseDraft::of) ?: PurchaseDraft(),
        )
    }

    /** Autopilot's suggestion is a hint: without one (Lockdown, Bypass, a failure) the screen is simply as before. */
    private suspend fun loadSuggestion() {
        val suggestion = try {
            container.core.autopilotSuggestion(requestId)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            null
        }
        _ui.update { it.copy(suggestion = suggestion) }
    }

    /**
     * Starts the download of Autopilot's model from the approval screen (the same job as Settings > Autopilot starts);
     * once it is installed, the request is scored.
     */
    fun downloadModel() {
        val wifiOnly = container.state.autopilot.value?.wifiOnly ?: true
        container.feedback.play(Event.Selection)
        _ui.update { it.copy(model = it.model?.copy(state = if (it.model.state == ModelState.FAILED) ModelState.NOT_INSTALLED else it.model.state, error = null)) }
        container.modelDownloads.start(wifiOnly)
        pollModel()
    }

    /** While the job waits or runs, the bytes are read a couple of times a second; at the end the request is scored. */
    private fun pollModel() {
        if (polling?.isActive == true) return
        polling = viewModelScope.launch {
            do {
                try {
                    val model = container.core.modelStatus()
                    _ui.update { it.copy(model = model) }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                }
                delay(500)
            } while (_ui.value.downloadJob != DownloadJob.Idle || _ui.value.model?.state == ModelState.DOWNLOADING)
            container.refreshAutopilot()
            if (_ui.value.model?.state == ModelState.INSTALLED) loadSuggestion()
        }
    }

    fun edit(change: (ApprovalDraft) -> ApprovalDraft) {
        if (_ui.value.readOnly) return
        touched = true
        _ui.update { it.copy(draft = change(it.draft), error = null) }
    }

    fun editPurchase(change: (PurchaseDraft) -> PurchaseDraft) {
        if (_ui.value.readOnly) return
        touched = true
        _ui.update { it.copy(purchase = change(it.purchase), error = null) }
    }

    /**
     * Approves a purchase with what the purchase screen shows picked, after the screen lock or biometrics. Paying on
     * the phone then opens the store's checkout ([openCheckout]).
     */
    fun approvePurchase(authenticator: Authenticator, openCheckout: (String) -> Unit) {
        if (_ui.value.readOnly) return
        val current = _ui.value
        val view = current.view ?: return
        val purchase = view.purchase ?: return
        if (current.busy) return
        val draft = current.purchase
        val method = purchase.methods.firstOrNull { it.id == draft.methodId }
        val problem = when {
            method == null -> "Choose how to pay."
            method.unavailable != null -> method.unavailable
            purchase.ships && draft.addressId == null -> "Choose where it goes. Add an identity with an address to your vault."
            else -> null
        }
        val limit = if (draft.limit && method?.id in purchase.limitMethods) {
            val each = Money.parse(draft.limitPerPurchase, purchase.currency)
            val day = Money.parse(draft.limitPerDay, purchase.currency)
            if (each == null || day == null || day < each) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(error = "Enter the spend limit's amounts, the day's at least a purchase's.") }
                return
            }
            SpendLimitInput("", listOf(purchase.domain), method!!.id, purchase.currency, each, day, LimitPeriod.DAY, draft.limitDays * 86_400L)
        } else {
            null
        }
        if (problem != null) {
            container.feedback.play(Event.Error)
            _ui.update { it.copy(error = problem) }
            return
        }
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                when (authenticator.authenticate("Approve purchase", "${purchase.total} at ${purchase.domain}")) {
                    AuthResult.Success -> {
                        container.feedback.play(if (limit != null) Event.GrantCreated else Event.Approved)
                        container.core.approvePurchase(requestId, PurchaseChoice(draft.methodId, draft.addressId, limit))
                        container.refreshAfterAnswer(requestId)
                        if (method?.kind == "pay_on_phone") openCheckout(purchase.checkoutUrl)
                        _ui.update { it.copy(busy = false, finished = true) }
                    }
                    AuthResult.Cancelled -> _ui.update { it.copy(busy = false) }
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.update { it.copy(busy = false, error = "Set a screen lock or fingerprint on this phone to approve.") }
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    /**
     * "Ask every time" for the environment a release is from, saved on the phone at once. Switched on, nothing of
     * this request is remembered either.
     */
    fun setEnvAlwaysAsk(on: Boolean) {
        if (_ui.value.readOnly) return
        val release = _ui.value.view?.envRelease ?: return
        val before = _ui.value.draft.envAlwaysAsk
        edit { it.copy(envAlwaysAsk = on, lifetime = if (on) LifetimeKind.ONCE else it.lifetime) }
        viewModelScope.launch {
            try {
                container.core.setSecretsAlwaysAsk(release.project, release.env, on)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(draft = it.draft.copy(envAlwaysAsk = before), error = e.userMessage()) }
            }
        }
    }

    fun toggleMore() {
        _ui.update { it.copy(moreOpen = !it.moreOpen) }
    }

    /**
     * Approving needs the phone's own screen lock or biometrics first; anything but success does nothing. What is
     * remembered alongside is what the dock and its "Remember for" sheet say ([buildDecision]).
     */
    fun approve(authenticator: Authenticator) {
        if (_ui.value.readOnly) return
        val current = _ui.value
        val view = current.view ?: return
        if (current.busy) return
        val choice = when (val built = buildDecision(view, current.draft)) {
            is BuildResult.Invalid -> {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(error = built.message) }
                return
            }
            is BuildResult.Ok -> built.choice
        }
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val title = if (view.kind == ApprovalKind.GRANT || view.kind == ApprovalKind.ACCOUNTS) "Allow access" else "Approve request"
                when (authenticator.authenticate(title, view.connectionLabel)) {
                    AuthResult.Success -> {
                        // A permission that stays is a bigger step than one answer, and sounds like it.
                        val standing = choice.standing != null || view.kind == ApprovalKind.GRANT
                        container.feedback.play(if (standing) Event.GrantCreated else Event.Approved)
                        // The sheet closes now; the action (an email going out, a push) finishes in the background.
                        container.answerInBackground(requestId, "Not approved") { container.core.approve(requestId, choice) }
                        _ui.update { it.copy(busy = false, finished = true) }
                    }
                    AuthResult.Cancelled -> _ui.update { it.copy(busy = false) }
                    AuthResult.Unavailable -> {
                        container.feedback.play(Event.Error)
                        _ui.update { it.copy(busy = false, error = "Set a screen lock or fingerprint on this phone to approve.") }
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    /** Closes at once; the answer goes out in the background (and the card comes back if it could not). */
    fun deny() {
        if (_ui.value.readOnly) return
        if (_ui.value.busy) return
        container.feedback.play(Event.Denied)
        container.answerInBackground(requestId, "Not denied") { container.core.deny(requestId) }
        _ui.update { it.copy(finished = true) }
    }
}
