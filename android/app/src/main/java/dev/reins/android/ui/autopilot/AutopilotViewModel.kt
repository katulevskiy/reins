package dev.reins.android.ui.autopilot

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
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
            loadProfiles()
            if (settings?.model?.state == ModelState.DOWNLOADING) pollModel()
        }
    }

    private suspend fun loadProfiles() {
        try {
            val profiles = container.core.autopilotProfiles()
            _ui.update { it.copy(profiles = profiles) }
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
     * Sets the global mode or a connection's ([mode] null: back to following the global mode). Bypass takes [minutes].
     * The change is felt at once; a refusal from the core (a just-paired connection cannot be bypassed) says why.
     */
    fun setMode(mode: AutopilotMode?, minutes: UInt? = null, connectionId: String? = null) {
        val old = modeOf(connectionId)
        val new = mode ?: settings.value?.mode ?: AutopilotMode.MANUAL
        val restart = mode == AutopilotMode.BYPASS && old == AutopilotMode.BYPASS
        (if (restart) Event.BypassOn else AutopilotText.modeChangeEvent(old, new))?.let { container.feedback.play(it) }
        run {
            container.core.setAutopilotMode(connectionId, mode, if (mode == AutopilotMode.BYPASS) minutes ?: AutopilotText.bypassMinutes.first() else null)
            container.refreshAutopilot()
        }
    }

    /** Ends a bypass: the global one goes back to the mode it interrupted, a connection's to its own setting. */
    fun stopBypass(connectionId: String? = null) {
        val s = settings.value ?: return
        container.feedback.play(Event.BypassOff)
        run {
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
        run {
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
        run {
            container.core.deleteModel()
            refreshModel()
            container.refreshAutopilot()
        }
    }

    fun setWifiOnly(on: Boolean) {
        run {
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
        run {
            val profile = container.core.createProfile(clean, icon)
            loadProfiles()
            onCreated(profile.id)
        }
    }

    fun renameProfile(id: String, name: String, icon: String?) {
        val clean = name.trim()
        if (clean.isEmpty()) return
        run {
            container.core.renameProfile(id, clean, icon)
            loadProfiles()
        }
    }

    fun makeDefault(id: String) {
        container.feedback.play(Event.Selection)
        run {
            container.core.setDefaultProfile(id)
            loadProfiles()
            container.refreshAutopilot()
            val name = _ui.value.profiles.firstOrNull { it.id == id }?.name
            _ui.update { it.copy(notice = name?.let { n -> "$n is now the default profile." }) }
        }
    }

    fun setPreset(id: String, preset: Preset) {
        run {
            container.core.setPreset(id, preset)
            loadProfiles()
        }
    }

    /** [locked]: true always asks, false lets Auto approve now, null lets the numbers decide. */
    fun setClassLock(id: String, classKey: String, locked: Boolean?) {
        // The chips sound their own choice; unlocking (after its warning) is felt as more autonomy.
        if (locked == false) container.feedback.play(Event.AutopilotOn)
        run {
            container.core.setClassLock(id, classKey, locked)
            loadProfiles()
        }
    }

    fun resetProfile(id: String) {
        container.feedback.play(Event.Revoked)
        run {
            container.core.resetProfile(id)
            loadProfiles()
            _ui.update { it.copy(notice = "Forgotten. This profile starts learning again from your next answer.") }
        }
    }

    fun deleteProfile(id: String, onDone: () -> Unit) {
        container.feedback.play(Event.Revoked)
        run {
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

    private fun run(block: suspend () -> Unit) {
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
            }
        }
    }

    private companion object {
        const val POLL_MS = 500L
    }
}
