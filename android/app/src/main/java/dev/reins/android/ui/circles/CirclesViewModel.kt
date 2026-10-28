package dev.reins.android.ui.circles

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.CircleDraft
import dev.reins.core.CircleInviteProgress
import dev.reins.core.CircleInviteStart
import dev.reins.core.CircleInviteView
import dev.reins.core.CircleJoinStage
import dev.reins.core.CirclePerson
import dev.reins.core.CircleRole
import dev.reins.core.GuardianNoticeView
import dev.reins.core.WardView
import dev.reins.android.platform.toInput
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlin.coroutines.cancellation.CancellationException

/** Settings › Circles: whether this phone can take part, the people, and what is being done to one of them. */
data class CirclesUi(
    /** Null while it is checked. */
    val available: Boolean? = null,
    /** Null until first read. */
    val people: List<CirclePerson>? = null,
    /** The arrangement a Pause, Resume or End is running for. */
    val busy: String? = null,
    val error: String? = null,
)

/** Where adding someone stands (the Add sheet). */
data class AddFlow(
    val role: CircleRole? = null,
    /** A key of [PRESETS]. */
    val preset: String? = null,
    /** One AI: the connection picked, and its name. */
    val agentId: String? = null,
    val agentLabel: String = "",
    val deferred: Boolean = true,
    /** A guardianship's notice (0, 3 or 7 days), picked by this account, the ward, before the code. */
    val noticeDays: Int? = null,
    /** The invitation, once made: its link (shown as text and a QR code). */
    val invite: CircleInviteStart? = null,
    val progress: CircleInviteProgress = CircleInviteProgress.Waiting,
    val busy: Boolean = false,
    val error: String? = null,
    /** The person it ended with, once confirmed. */
    val added: CirclePerson? = null,
) {
    /** Which page shows: the role, the scope, Instant or Deferred, then the invitation. */
    val page: AddPage get() = when {
        invite != null -> AddPage.Invite
        role == null -> AddPage.Role
        preset == null || (preset == "agent" && agentId == null) -> AddPage.Scope
        else -> AddPage.Class
    }
}

enum class AddPage { Role, Scope, Class, Invite }

/** The ward's own guardianship, on its screen (04 §4: the scope, every decision and note, the way out). */
data class WardUi(
    val person: CirclePerson,
    /** Null until read. */
    val view: WardView? = null,
    val busy: Boolean = false,
    val error: String? = null,
    /** The guardian's minor ask, set aside for now. */
    val minorIgnored: Boolean = false,
)

/** Someone this account guards: what it was told about them, and the consent a minor's leaving waits for. */
data class GuardingUi(
    val person: CirclePerson,
    /** Null until read; newest first. */
    val notices: List<GuardianNoticeView>? = null,
    /** The notice an Allow runs for, or [CirclesViewModel.MINOR]. */
    val busy: String? = null,
    val error: String? = null,
    /** The minor mark was asked for here. */
    val minorAsked: Boolean = false,
)

/** Where joining someone's Circle stands (the Join sheet). */
data class JoinFlow(
    val link: String = "",
    val view: CircleInviteView? = null,
    val busy: Boolean = false,
    val error: String? = null,
)

class CirclesViewModel(private val container: AppContainer) : ViewModel() {
    private val epoch = container.state.accountEpoch.value
    private val _ui = MutableStateFlow(CirclesUi())
    val ui: StateFlow<CirclesUi> = _ui.asStateFlow()

    private val _add = MutableStateFlow<AddFlow?>(null)
    /** Null when the Add sheet is closed. */
    val add: StateFlow<AddFlow?> = _add.asStateFlow()

    private val _join = MutableStateFlow<JoinFlow?>(null)
    /** Null when the Join sheet is closed. */
    val join: StateFlow<JoinFlow?> = _join.asStateFlow()

    private val _ward = MutableStateFlow<WardUi?>(null)
    /** The ward's own guardianship, open on its screen; null when closed. */
    val ward: StateFlow<WardUi?> = _ward.asStateFlow()

    private val _guarding = MutableStateFlow<GuardingUi?>(null)
    /** Someone this account guards, open on the notices screen; null when closed. */
    val guarding: StateFlow<GuardingUi?> = _guarding.asStateFlow()

    private var addPoll: Job? = null
    private var joinPoll: Job? = null

    private fun current() = container.state.isCurrent(epoch)

    /** The primary-phone setup offered when this phone can't take part yet. */
    fun primaryPhoneSetup() = dev.reins.android.ui.permissions.PrimaryPhoneSetupModel(container, viewModelScope, ::current)

    /** Reads whether this phone can take part, then the people ([fromServer]: ask the server first). */
    fun load(fromServer: Boolean = true) {
        viewModelScope.launch {
            val available = try {
                container.core.circlesAvailable()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                false
            }
            if (!current()) return@launch
            _ui.update { it.copy(available = available) }
            if (!available) return@launch
            try {
                val people = container.core.circlePeople(fromServer)
                if (current()) _ui.update { it.copy(people = people, error = null) }
                // What is open follows: a ward's countdown, a guardian's new notice.
                _ward.value?.let { open ->
                    people.firstOrNull { it.linkId == open.person.linkId && it.state != dev.reins.core.CircleLinkState.ENDED }?.let { readWard(it) }
                        ?: closeWard()
                }
                _guarding.value?.let { open -> people.firstOrNull { it.linkId == open.person.linkId }?.let { readNotices(it, refresh = false) } }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (current()) _ui.update { it.copy(error = e.userMessage(), people = it.people ?: emptyList()) }
            }
        }
    }

    /** Pauses (or resumes) an arrangement this account made. */
    fun pause(person: CirclePerson, paused: Boolean) = act(person) { container.core.circlePause(person.linkId, paused) }

    /** Ends an arrangement (the owner) or leaves it (a helper). */
    fun end(person: CirclePerson) = act(person) {
        container.core.circleEnd(person.linkId)
        container.feedback.play(Event.Revoked)
    }

    private fun act(person: CirclePerson, block: suspend () -> Unit) {
        if (_ui.value.busy != null) return
        _ui.update { it.copy(busy = person.linkId, error = null) }
        viewModelScope.launch {
            try {
                block()
                val people = container.core.circlePeople(false)
                if (current()) _ui.update { it.copy(people = people) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                if (current()) _ui.update { it.copy(error = e.userMessage()) }
            } finally {
                _ui.update { it.copy(busy = null) }
            }
        }
    }

    // ---- Add someone -----------------------------------------------------------------------------------------------

    fun openAdd() {
        stopAddPoll()
        _add.value = AddFlow()
    }

    fun pickRole(role: CircleRole) = _add.update { it?.copy(role = role, error = null) }

    fun pickPreset(key: String) = _add.update { it?.copy(preset = key, agentId = null, agentLabel = "", error = null) }

    fun pickAgent(id: String, label: String) = _add.update { it?.copy(agentId = id, agentLabel = label, error = null) }

    fun setDeferred(deferred: Boolean) = _add.update { it?.copy(deferred = deferred) }

    fun pickNotice(days: Int) {
        if (days in NOTICE_DAYS) _add.update { it?.copy(noticeDays = days, error = null) }
    }

    /** One step back inside the sheet; false when there is none (the sheet closes). */
    fun addBack(): Boolean {
        val flow = _add.value ?: return false
        _add.value = when (flow.page) {
            AddPage.Role -> return false
            AddPage.Scope -> if (flow.preset == "agent") flow.copy(preset = null, agentId = null) else flow.copy(role = null)
            AddPage.Class -> if (flow.preset == "agent") flow.copy(agentId = null) else flow.copy(preset = null)
            AddPage.Invite -> return false
        }
        return true
    }

    /** Makes the invitation from what was picked, then follows it every few seconds. */
    fun createInvite() {
        val flow = _add.value ?: return
        val role = flow.role ?: return
        val scope = preset(flow.preset ?: return, flow.agentId, flow.agentLabel) ?: return
        // A guardianship's notice is the ward's to pick, up front: no default.
        val notice = if (role == CircleRole.GUARDIAN) (flow.noticeDays ?: return) else null
        if (flow.busy) return
        _add.update { it?.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val draft = CircleDraft(
                    role = role,
                    preset = scope,
                    deferred = if (decides(role)) flow.deferred else true,
                    windowSecs = null,
                    fallBackToMe = role == CircleRole.DELEGATE,
                    fullCards = false,
                    noticeDays = notice?.toUByte(),
                )
                val start = container.core.circleInviteCreate(draft)
                if (!current()) return@launch
                _add.update { it?.copy(invite = start, busy = false, progress = CircleInviteProgress.Waiting) }
                pollInvite(start.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _add.update { it?.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    private fun pollInvite(id: String) {
        stopAddPoll()
        addPoll = viewModelScope.launch {
            while (isActive) {
                val progress = try {
                    container.core.circleInvitePoll(id)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    null // A missed poll: the next one tries again.
                }
                if (progress != null) {
                    val before = _add.value?.progress
                    _add.update { it?.copy(progress = progress) }
                    // The code just arrived: a tap says so.
                    if (inviteCode(progress) != null && before?.let(::inviteCode) == null) container.feedback.play(Event.Detent)
                    if (inviteFinished(progress)) {
                        if (progress is CircleInviteProgress.Active) load(fromServer = false)
                        return@launch
                    }
                }
                delay(POLL_MS)
            }
        }
    }

    private fun stopAddPoll() {
        addPoll?.cancel()
        addPoll = null
    }

    /**
     * The helper accepted and the code matches: the owner's check (biometrics or the screen lock, after the hold), then
     * the arrangement starts. The core does not ask by itself here, so this asks before it is called.
     */
    fun confirmInvite(authenticator: Authenticator) {
        val flow = _add.value ?: return
        val id = flow.invite?.id ?: return
        val accepted = flow.progress as? CircleInviteProgress.Accepted ?: return
        if (flow.busy) return
        _add.update { it?.copy(busy = true, error = null) }
        viewModelScope.launch {
            val owner = OwnerPrompt(authenticator, "Add ${shortName(untrusted(accepted.helper))}")
            try {
                if (!owner.confirm("${roleLabel(flow.role ?: CircleRole.WATCHER)} · ${formatCode(accepted.code)}")) {
                    _add.update {
                        it?.copy(
                            busy = false,
                            error = if (owner.last == AuthResult.Unavailable) "Set a screen lock or fingerprint on this phone first." else null,
                        )
                    }
                    return@launch
                }
                stopAddPoll()
                val person = container.core.circleInviteConfirm(id)
                container.feedback.play(Event.Connected)
                if (!current()) return@launch
                _add.update { it?.copy(busy = false, added = person, progress = CircleInviteProgress.Active(person.linkId)) }
                load(fromServer = false)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _add.update { it?.copy(busy = false, error = e.userMessage()) }
                pollInvite(id)
            }
        }
    }

    /** Closes the Add sheet; an invitation still open is withdrawn. */
    fun closeAdd() {
        val flow = _add.value
        stopAddPoll()
        _add.value = null
        val id = flow?.invite?.id ?: return
        if (inviteFinished(flow.progress)) return
        // The app's scope: closing the screen (which clears this model) still withdraws it.
        container.appScope.launch {
            try {
                container.core.circleInviteCancel(id)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // It runs out by itself.
            }
        }
    }

    // ---- Join someone's Circle -------------------------------------------------------------------------------------

    fun openJoin(link: String = "") {
        stopJoinPoll()
        _join.value = JoinFlow(link = link)
        if (circleLink(link) != null) openLink()
    }

    fun setJoinLink(text: String) = _join.update { it?.copy(link = text, error = null) }

    /** Answers the invitation in the link, then follows it every few seconds. */
    fun openLink() {
        val flow = _join.value ?: return
        val link = circleLink(flow.link)
        if (link == null) {
            _join.update { it?.copy(error = "Not a Circle link.") }
            return
        }
        if (flow.busy) return
        _join.update { it?.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val view = container.core.circleJoinOpen(link)
                if (!current()) return@launch
                _join.update { it?.copy(view = view, busy = false) }
                pollJoin(view.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _join.update { it?.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    private fun pollJoin(id: String) {
        stopJoinPoll()
        joinPoll = viewModelScope.launch {
            while (isActive) {
                delay(POLL_MS)
                val view = try {
                    container.core.circleJoinPoll(id)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    null
                }
                if (view != null) {
                    val before = _join.value?.view?.stage
                    _join.update { it?.copy(view = view) }
                    if (view.stage == CircleJoinStage.COMPARE && before != CircleJoinStage.COMPARE) container.feedback.play(Event.Detent)
                    if (joinFinished(view.stage)) {
                        if (view.stage == CircleJoinStage.ACTIVE) load(fromServer = false)
                        return@launch
                    }
                }
            }
        }
    }

    private fun stopJoinPoll() {
        joinPoll?.cancel()
        joinPoll = null
    }

    /** Accepts (after comparing the code) or declines the invitation. */
    fun answerJoin(accept: Boolean) {
        val flow = _join.value ?: return
        val view = flow.view ?: return
        if (flow.busy) return
        _join.update { it?.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                if (accept) container.core.circleJoinAccept(view.id) else container.core.circleJoinDecline(view.id)
                container.feedback.play(if (accept) Event.Approved else Event.Denied)
                val stage = if (accept) CircleJoinStage.ACCEPTED else CircleJoinStage.DECLINED
                _join.update { it?.copy(busy = false, view = it.view?.copy(stage = stage)) }
                if (!accept) stopJoinPoll() else if (joinPoll?.isActive != true) pollJoin(view.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _join.update { it?.copy(busy = false, error = e.userMessage()) }
            }
        }
    }

    fun closeJoin() {
        stopJoinPoll()
        _join.value = null
    }

    // ---- Guardianship (Decisions v2 P7, 04 §4) ---------------------------------------------------------------------

    /** A Guardian row: the ward's view of its guardianship, or the guardian's notices about the ward. */
    fun openPerson(person: CirclePerson) {
        when {
            wardsGuardianship(person) -> {
                _ward.value = WardUi(person)
                readWard(person)
            }
            guardsThem(person) -> {
                _guarding.value = GuardingUi(person)
                readNotices(person, refresh = true)
            }
        }
    }

    fun closeWard() {
        _ward.value = null
    }

    fun closeGuarding() {
        _guarding.value = null
    }

    private fun readWard(person: CirclePerson) {
        viewModelScope.launch {
            try {
                val view = container.core.guardianWardView(person.linkId)
                if (current()) _ward.update { it?.takeIf { w -> w.person.linkId == person.linkId }?.copy(person = person, view = view) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (current()) _ward.update { it?.takeIf { w -> w.person.linkId == person.linkId }?.copy(error = e.userMessage()) }
            }
        }
    }

    private fun wardAct(block: suspend (WardUi) -> Unit) {
        val open = _ward.value ?: return
        if (open.busy) return
        _ward.update { it?.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                block(open)
                val people = container.core.circlePeople(false)
                if (!current()) return@launch
                _ui.update { it.copy(people = people) }
                val person = people.firstOrNull { it.linkId == open.person.linkId && it.state != dev.reins.core.CircleLinkState.ENDED }
                if (person == null) {
                    // It ended right away (no notice): back to the list.
                    closeWard()
                    return@launch
                }
                val view = container.core.guardianWardView(person.linkId)
                _ward.update { it?.copy(person = person, view = view) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                if (current()) _ward.update { it?.copy(error = e.userMessage()) }
            } finally {
                _ward.update { it?.copy(busy = false) }
            }
        }
    }

    /** The ward leaves: the notice it chose starts (a minor's waits for the guardian), and the guardian is told. */
    fun leaveWard() = wardAct { open ->
        container.core.circleEnd(open.person.linkId)
        container.feedback.play(Event.Revoked)
    }

    /**
     * The ward accepts the guardian's minor mark, with the phone's age signal. No signal never blocks it: the ward's own
     * acceptance counts. An adult's signal is refused by the core.
     */
    fun acceptMinor(authenticator: Authenticator) = wardAct { open ->
        val signal = container.ageSignals().check()
        // The core asks the owner (biometrics or the screen lock) before it signs: a cancel marks nothing.
        val owner = OwnerPrompt(authenticator, "Mark as a minor's")
        container.core.guardianAcceptMinor(open.person.linkId, signal.toInput(), owner)
        container.feedback.play(Event.Approved)
    }

    /** Leaves the guardian's ask unanswered (it shows again next time). */
    fun ignoreMinor() = _ward.update { it?.copy(minorIgnored = true) }

    /** The ward's weekly summary of automatic decisions in scope, for the guardian (opt-in). */
    fun setDigest(on: Boolean) {
        val open = _ward.value ?: return
        val before = open.view ?: return
        if (open.busy) return
        _ward.update { it?.copy(view = before.copy(digest = on)) }
        wardAct { w ->
            try {
                container.core.guardianSetDigest(w.person.linkId, on)
            } catch (e: Exception) {
                _ward.update { it?.copy(view = it.view?.copy(digest = before.digest)) }
                throw e
            }
        }
    }

    private fun readNotices(person: CirclePerson, refresh: Boolean) {
        viewModelScope.launch {
            try {
                val notices = container.core.guardianNotices(refresh).filter { it.linkId == person.linkId }
                if (current()) _guarding.update { it?.takeIf { g -> g.person.linkId == person.linkId }?.copy(person = person, notices = notices) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (current()) {
                    _guarding.update { it?.takeIf { g -> g.person.linkId == person.linkId }?.copy(error = e.userMessage(), notices = it.notices ?: emptyList()) }
                }
            }
        }
    }

    private fun guardAct(key: String, block: suspend (GuardingUi) -> Unit) {
        val open = _guarding.value ?: return
        if (open.busy != null) return
        _guarding.update { it?.copy(busy = key, error = null) }
        viewModelScope.launch {
            try {
                block(open)
                val people = container.core.circlePeople(false)
                val notices = container.core.guardianNotices(false).filter { it.linkId == open.person.linkId }
                if (!current()) return@launch
                _ui.update { it.copy(people = people) }
                val person = people.firstOrNull { it.linkId == open.person.linkId } ?: open.person
                _guarding.update { it?.copy(person = person, notices = notices) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                if (current()) _guarding.update { it?.copy(error = e.userMessage()) }
            } finally {
                _guarding.update { it?.copy(busy = null) }
            }
        }
    }

    /**
     * The guardian allows a minor's request to leave: it ends now. The core asks the owner (biometrics or the screen
     * lock) before it signs; a cancel or a phone without a screen lock consents to nothing.
     */
    fun consentEnd(notice: GuardianNoticeView, authenticator: Authenticator) = guardAct(notice.id) {
        val owner = OwnerPrompt(authenticator, "Let ${shortName(untrusted(notice.ward))} leave")
        container.core.guardianConsentEnd(notice.linkId, owner)
        container.feedback.play(Event.Approved)
    }

    /** The guardian asks to mark the ward's account a minor's; the ward accepts on their phone, or not. */
    fun proposeMinor() = guardAct(MINOR) { open ->
        container.core.guardianProposeMinor(open.person.linkId)
        _guarding.update { it?.copy(minorAsked = true) }
    }

    override fun onCleared() {
        closeAdd()
        stopJoinPoll()
    }

    companion object {
        /** How often an invitation is asked about while its sheet is open. */
        const val POLL_MS = 2_500L

        /** [GuardingUi.busy] while the minor mark is asked for. */
        const val MINOR = "minor"
    }
}

/** The helper's side will not move any more. */
fun joinFinished(stage: CircleJoinStage): Boolean =
    stage == CircleJoinStage.ACTIVE || stage == CircleJoinStage.DECLINED || stage == CircleJoinStage.EXPIRED
