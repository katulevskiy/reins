package dev.reins.android.ui.autopilot

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.autopilot.AutopilotDraft
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.AutopilotStats
import dev.reins.core.ThresholdReplay
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.OvernightRisky
import dev.reins.core.Preset
import dev.reins.core.ProfileView
import dev.reins.core.SuggestionView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class AutopilotUi(
    val profiles: List<ProfileView> = emptyList(),
    /** [profiles] were read at least once (until then an empty list says nothing). */
    val profilesLoaded: Boolean = false,
    /** The model as last read; while it downloads this is polled. */
    val model: ModelStatus? = null,
    val job: DownloadJob = DownloadJob.Idle,
    val busy: Boolean = false,
    val error: String? = null,
    /** A short confirmation ("Work is now the default profile."). */
    val notice: String? = null,
    /** "Try it": the last answer, and whether one is on its way. */
    val evaluation: SuggestionView? = null,
    val evaluating: Boolean = false,
    /** "This week": requests scored, how often Autopilot matched the user, kinds on Auto. */
    val stats: AutopilotStats? = null,
    /** A profile's last 30 days at the thresholds being set (profile id to the answer). */
    val replay: Pair<String, ThresholdReplay>? = null,
)

/** Settings > Autopilot, its profiles and "Try it", and the Autopilot part of a connection's page. */
class AutopilotViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(AutopilotUi())
    val ui: StateFlow<AutopilotUi> = _ui.asStateFlow()

    /** Modes, bypasses and the model, shared with the header pill. */
    val settings: StateFlow<AutopilotSettings?> get() = container.state.autopilot

    private var polling: Job? = null

    init {
        refresh()
        // Whatever re-reads Autopilot (a push, the download job finishing) brings the model's state along.
        viewModelScope.launch {
            settings.collect { s ->
                if (s != null) {
                    _ui.update { it.copy(model = s.model) }
                    if (s.model.state == ModelState.DOWNLOADING) pollModel()
                }
            }
        }
        viewModelScope.launch {
            container.modelDownloads.job.collect { job ->
                _ui.update { it.copy(job = job) }
                if (job != DownloadJob.Idle) pollModel()
            }
        }
    }

    fun refresh() {
        viewModelScope.launch {
            val settings = container.refreshAutopilot()
            settings?.let { s -> _ui.update { it.copy(model = s.model) } }
            if (settings?.model?.state == ModelState.DOWNLOADING) pollModel()
        }
    }

    /**
     * A page that shows profiles opened: they are read now, each time (they change as Autopilot learns), not at
     * sign-in, where reading every remembered decision would compete with the first refresh for someone who may never
     * open Autopilot.
     */
    fun showingProfiles() {
        viewModelScope.launch { loadProfiles() }
    }

    /** The Autopilot page opened: its profiles, and this week's numbers. */
    fun showingAutopilot() {
        viewModelScope.launch {
            loadProfiles()
            try {
                val stats = container.core.autopilotStats(7u)
                _ui.update { it.copy(stats = stats) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // The strip shows dashes; nothing else depends on it.
            }
        }
    }

    private suspend fun loadProfiles() {
        try {
            val profiles = container.core.autopilotProfiles()
            _ui.update { it.copy(profiles = profiles, profilesLoaded = true) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _ui.update { it.copy(error = e.userMessage()) }
        }
    }

    fun dismissMessages() {
        _ui.update { it.copy(error = null, notice = null) }
    }

    // ---- modes --------------------------------------------------------------------------------------------------

    /** The mode in force for [connectionId] (null = the global one). */
    fun modeOf(connectionId: String?): AutopilotMode? {
        val s = settings.value ?: return null
        if (connectionId == null) return s.mode
        return s.connections.firstOrNull { it.connectionId == connectionId }?.mode ?: s.mode
    }

    /**
     * Sets the global mode or a connection's ([mode] null: back to following the global mode); never Bypass or
     * Overnight, which [startBypass] and [startOvernight] turn on.
     * The change is felt at once; a refusal from the core (a just-paired connection cannot be bypassed) says why.
     */
    fun setMode(mode: AutopilotMode?, connectionId: String? = null) {
        // A bypass or an Overnight starts only through its own start, after the owner's check.
        if (AutopilotText.isTimed(mode)) return
        val old = modeOf(connectionId)
        val new = mode ?: settings.value?.mode ?: AutopilotMode.MANUAL
        AutopilotText.modeChangeEvent(old, new)?.let { container.feedback.play(it) }
        draft { AutopilotDraft.withMode(it, connectionId, mode, null, System.currentTimeMillis() / 1000) }
        perform {
            container.core.setAutopilotMode(connectionId, mode, null)
            container.refreshAutopilot()
        }
    }

    /**
     * Starts a bypass of every AI ([connectionId] null) or of one ([who] names it), for [minutes] or, with null, until
     * the user stops it. Holding the button proves intent, not identity: the phone's owner is checked first
     * ([authenticator]); without a pass nothing changes, not even for a moment. A refusal from the core (a just-paired
     * AI) says why.
     */
    fun startBypass(minutes: UInt?, connectionId: String? = null, who: String? = null, authenticator: Authenticator) {
        viewModelScope.launch {
            if (!ownerChecked(authenticator, AutopilotText.bypassAuthTitle(who), minutes)) return@launch
            container.feedback.play(Event.BypassOn)
            draft { AutopilotDraft.withBypass(it, connectionId, minutes, System.currentTimeMillis() / 1000) }
            perform {
                container.core.startBypass(connectionId, minutes)
                container.refreshAutopilot()
            }
        }
    }

    /**
     * Starts Overnight for every AI ([connectionId] null) or one ([who] names it), for [minutes] or, with null, until
     * the user stops it. The core itself asks the owner ([OwnerPrompt] over [authenticator], with its own reason) and
     * changes nothing without a pass; so it shows once the core said yes. The mode it interrupts is remembered for Stop.
     */
    fun startOvernight(minutes: UInt?, connectionId: String? = null, who: String? = null, authenticator: Authenticator) {
        val owner = OwnerPrompt(authenticator, AutopilotText.overnightAuthTitle(who))
        val before = settings.value
        val old = modeOf(connectionId)
        ownerAsked(owner) {
            container.core.startOvernight(connectionId, minutes, owner)
            AutopilotText.modeChangeEvent(old, AutopilotMode.OVERNIGHT)?.let { container.feedback.play(it) }
            if (before != null) container.overnightReturns.started(connectionId, AutopilotDraft.interrupted(before, connectionId))
            draft { AutopilotDraft.withOvernight(it, connectionId, minutes, System.currentTimeMillis() / 1000) }
            container.refreshAutopilot()
        }
    }

    /** Ends an Overnight: back to the mode it interrupted (the global one) or the AI's own setting before it. */
    fun stopOvernight(connectionId: String? = null) {
        val back = container.overnightReturns.stopMode(connectionId)
        val new = back ?: if (connectionId == null) AutopilotMode.MANUAL else settings.value?.mode ?: AutopilotMode.MANUAL
        AutopilotText.modeChangeEvent(AutopilotMode.OVERNIGHT, new)?.let { container.feedback.play(it) }
        draft { AutopilotDraft.withMode(it, connectionId, back, null, System.currentTimeMillis() / 1000) }
        perform {
            container.stopOvernight(connectionId)
            container.refreshAutopilot()
        }
    }

    /**
     * What Overnight does with a request too risky to decide: wait for the user, or deny it. Either way the core asks
     * the owner first; the choice shows once it is made.
     */
    fun setOvernightWhenRisky(risky: OvernightRisky, authenticator: Authenticator) {
        if (settings.value?.overnightWhenRisky == risky) return
        val owner = OwnerPrompt(authenticator, "Change ${AutopilotText.OVERNIGHT}")
        ownerAsked(owner) {
            container.core.setOvernightWhenRisky(risky, owner)
            draft { it.copy(overnightWhenRisky = risky) }
            container.refreshAutopilot()
        }
    }

    /**
     * Runs [block], a call the core answers only after asking [owner]. A cancel says nothing; a phone without a
     * screen lock says how to get one; any other refusal says why. What shows is re-read either way.
     */
    private fun ownerAsked(owner: OwnerPrompt, block: suspend () -> Unit) {
        _ui.update { it.copy(busy = true, error = null, notice = null) }
        viewModelScope.launch {
            try {
                block()
                _ui.update { it.copy(busy = false) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val error = when (owner.last) {
                    AuthResult.Cancelled -> null
                    AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone to turn on ${AutopilotText.OVERNIGHT}."
                    else -> e.userMessage()
                }
                if (error != null && owner.last != AuthResult.Unavailable) container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = error) }
                container.refreshAutopilot()
            }
        }
    }

    /**
     * Holding the button proves intent, not identity: the phone's owner is checked before a bypass turns on. Without a
     * pass nothing changes; without a screen lock at all it says so. (Overnight's check is asked by the core itself.)
     */
    private suspend fun ownerChecked(authenticator: Authenticator, title: String, minutes: UInt?): Boolean {
        val result = try {
            authenticator.authenticate(title, AutopilotText.bypassAuthDetail(minutes))
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            AuthResult.Cancelled
        }
        return when (result) {
            AuthResult.Success -> true
            AuthResult.Cancelled -> false
            AuthResult.Unavailable -> {
                _ui.update { it.copy(error = "Set a screen lock or fingerprint on this phone to turn on Bypass.") }
                false
            }
        }
    }

    /** Ends a bypass: the global one goes back to the mode it interrupted, a connection's to its own setting. */
    fun stopBypass(connectionId: String? = null) {
        val s = settings.value ?: return
        container.feedback.play(Event.BypassOff)
        draft { AutopilotDraft.withoutBypass(it, connectionId) }
        perform {
            if (connectionId == null) {
                container.core.setAutopilotMode(null, s.baseMode, null)
            } else {
                val own = s.connections.firstOrNull { it.connectionId == connectionId }?.baseMode
                container.core.setAutopilotMode(connectionId, own, null)
            }
            container.refreshAutopilot()
        }
    }

    /** Which profile a connection's decisions train (null = the default profile). */
    fun assignProfile(connectionId: String, profileId: String?) {
        draft { AutopilotDraft.withProfile(it, connectionId, profileId) }
        perform {
            container.core.assignProfile(connectionId, profileId)
            container.refreshAutopilot()
            loadProfiles()
        }
    }

    // ---- the model ----------------------------------------------------------------------------------------------

    fun download() {
        val wifiOnly = settings.value?.wifiOnly ?: true
        _ui.update { it.copy(error = null, model = it.model?.copy(state = if (it.model.state == ModelState.FAILED) ModelState.NOT_INSTALLED else it.model.state, error = null)) }
        container.modelDownloads.start(wifiOnly)
        pollModel()
    }

    /** Only a download still waiting for its network can be called off. */
    fun cancelDownload() {
        container.modelDownloads.cancel()
    }

    fun deleteModel() {
        container.feedback.play(Event.Revoked)
        perform {
            container.core.deleteModel()
            refreshModel()
            container.refreshAutopilot()
        }
    }

    fun setWifiOnly(on: Boolean) {
        draft { it.copy(wifiOnly = on) }
        perform {
            container.core.setAutopilotWifiOnly(on)
            container.refreshAutopilot()
        }
    }

    private suspend fun refreshModel() {
        val model = container.core.modelStatus()
        _ui.update { it.copy(model = model) }
    }

    /** While the job waits or runs, the bytes are read a couple of times a second. */
    private fun pollModel() {
        if (polling?.isActive == true) return
        polling = viewModelScope.launch {
            do {
                try {
                    refreshModel()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                }
                delay(POLL_MS)
            } while (_ui.value.job != DownloadJob.Idle || _ui.value.model?.state == ModelState.DOWNLOADING)
            val finished = _ui.value.model?.state
            if (finished == ModelState.INSTALLED) container.feedback.play(Event.GrantCreated)
            if (finished == ModelState.FAILED) container.feedback.play(Event.Error)
            container.refreshAutopilot()
        }
    }

    // ---- profiles -----------------------------------------------------------------------------------------------

    fun createProfile(name: String, icon: String?, onCreated: (String) -> Unit = {}) {
        val clean = name.trim()
        if (clean.isEmpty()) return
        container.feedback.play(Event.GrantCreated)
        perform {
            val profile = container.core.createProfile(clean, icon)
            loadProfiles()
            onCreated(profile.id)
        }
    }

    fun renameProfile(id: String, name: String, icon: String?) {
        val clean = name.trim()
        if (clean.isEmpty()) return
        perform {
            container.core.renameProfile(id, clean, icon)
            loadProfiles()
        }
    }

    fun makeDefault(id: String) {
        container.feedback.play(Event.Selection)
        draft { AutopilotDraft.withDefaultProfile(it, id) }
        _ui.update { ui -> ui.copy(profiles = ui.profiles.map { it.copy(isDefault = it.id == id) }) }
        perform {
            container.core.setDefaultProfile(id)
            loadProfiles()
            container.refreshAutopilot()
            val name = _ui.value.profiles.firstOrNull { it.id == id }?.name
            _ui.update { it.copy(notice = name?.let { n -> "$n is now the default profile." }) }
        }
    }

    fun setPreset(id: String, preset: Preset) {
        // Shown at once; the reload after the core's answer settles it (a refusal reloads the old one).
        editProfile(id) { it.copy(preset = preset) }
        perform {
            container.core.setPreset(id, preset)
            loadProfiles()
        }
    }

    /** The profile's own thresholds (whole percent, 80 to 99), saved when the user lets go of a slider. */
    fun setThresholds(id: String, approveAt: Float, denyAt: Float) {
        editProfile(id) { it.copy(preset = Preset.CUSTOM, approveAt = approveAt, denyAt = denyAt) }
        perform {
            container.core.setThresholds(id, approveAt, denyAt)
            loadProfiles()
        }
    }

    private var replaying: Job? = null

    /** What the profile's last 30 days would have been at these thresholds; a newer ask replaces one on its way. */
    fun replay(id: String, approveAt: Float, denyAt: Float) {
        replaying?.cancel()
        replaying = viewModelScope.launch {
            delay(REPLAY_DEBOUNCE_MS)
            try {
                val answer = container.core.thresholdReplay(id, approveAt, denyAt)
                _ui.update { it.copy(replay = id to answer) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
            }
        }
    }

    /** [locked]: true always asks, false lets Auto approve now, null lets the numbers decide. */
    fun setClassLock(id: String, classKey: String, locked: Boolean?) {
        // The chips sound their own choice; unlocking (after its warning) is felt as more autonomy.
        if (locked == false) container.feedback.play(Event.AutopilotOn)
        editProfile(id) { p -> p.copy(classes = p.classes.map { if (it.classKey == classKey) it.copy(manual = locked?.not(), autoApprove = locked?.not() ?: it.autoApprove) else it }) }
        perform {
            container.core.setClassLock(id, classKey, locked)
            loadProfiles()
        }
    }

    fun resetProfile(id: String) {
        container.feedback.play(Event.Revoked)
        perform {
            container.core.resetProfile(id)
            loadProfiles()
            _ui.update { it.copy(notice = "Forgotten. This profile starts learning again from your next answer.") }
        }
    }

    fun deleteProfile(id: String, onDone: () -> Unit) {
        container.feedback.play(Event.Revoked)
        perform {
            container.core.deleteProfile(id)
            loadProfiles()
            container.refreshAutopilot()
            onDone()
        }
    }

    // ---- Try it -------------------------------------------------------------------------------------------------

    fun evaluate(profileId: String?, situation: String) {
        if (situation.isBlank() || _ui.value.evaluating) return
        _ui.update { it.copy(evaluating = true, error = null) }
        viewModelScope.launch {
            try {
                val answer = container.core.autopilotEvaluate(profileId, situation.trim())
                container.feedback.play(Event.Refresh)
                _ui.update { it.copy(evaluating = false, evaluation = answer) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(evaluating = false, error = e.userMessage()) }
            }
        }
    }

    fun clearEvaluation() {
        _ui.update { it.copy(evaluation = null) }
    }

    /** Shows a change to the shared settings at once; the re-read after the core's answer (or its refusal) settles it. */
    private fun draft(edit: (AutopilotSettings) -> AutopilotSettings) {
        settings.value?.let { container.state.setAutopilot(edit(it)) }
    }

    private fun editProfile(id: String, edit: (ProfileView) -> ProfileView) {
        _ui.update { ui -> ui.copy(profiles = ui.profiles.map { if (it.id == id) edit(it) else it }) }
    }

    /**
     * Runs [block] off the caller, showing a failure (named so that inside a coroutine it is never taken for the
     * standard library's `run`, which would let a failure escape unseen).
     */
    private fun perform(block: suspend () -> Unit) {
        _ui.update { it.copy(busy = true, error = null, notice = null) }
        viewModelScope.launch {
            try {
                block()
                _ui.update { it.copy(busy = false) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
                container.refreshAutopilot()
                loadProfiles()
            }
        }
    }

    private companion object {
        const val POLL_MS = 500L
        const val REPLAY_DEBOUNCE_MS = 120L
    }
}
