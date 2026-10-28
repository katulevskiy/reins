package dev.reins.android.ui.autopilot

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.autopilot.AutopilotDraft
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.CheckNow
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.autopilot.ModelUpdateGate
import dev.reins.android.autopilot.NoProgress
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.AutopilotStats
import dev.reins.core.AutoSetup
import dev.reins.core.CellChoice
import dev.reins.core.WorldItem
import dev.reins.core.PolicyChange
import dev.reins.core.ZoneChoice
import dev.reins.core.ThresholdReplay
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.Preset
import dev.reins.core.ProfileView
import dev.reins.core.SuggestionView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
    /** "Check now" is asking the core (and maybe downloading the newer model). */
    val checking: Boolean = false,
    /** What the last "Check now" found ("Up to date", "Waiting for Wi-Fi", an error). */
    val checkResult: String? = null,
    /** Auto's setup as last read (Decisions v2): the owner's world and the matrix. */
    val setup: AutoSetup? = null,
    /** Picking Auto before the setup was finished: the setup should open (the page that shows it clears this). */
    val setupRequested: Boolean = false,
    /** A cell about to be made looser, with what it would have done on this phone's last 30 days (02 §4). */
    val loosen: LoosenCell? = null,
    /** How long an approved email waits with Undo before it goes, in seconds (0: off; 08 R10); null until read. */
    val sendHoldSecs: UInt? = null,
)

/** A looser matrix cell waiting for the owner to look at its preview. */
data class LoosenCell(val zone: ZoneChoice, val level: Int, val cell: CellChoice, val preview: String)

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
        viewModelScope.launch { loadSendHold() }
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

    /** The send hold as the core has it; a failed read leaves the setting out (it shows once it is known). */
    private suspend fun loadSendHold() {
        try {
            val secs = container.core.sendHoldSecs()
            _ui.update { it.copy(sendHoldSecs = secs) }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
        }
    }

    /**
     * How long an approved email waits with Undo before it goes (08 R10): 0 (off) to 120 seconds. Shown at once; a
     * refusal says why and shows the setting as the core has it.
     */
    fun setSendHold(secs: UInt) {
        if (_ui.value.sendHoldSecs == secs) return
        _ui.update { it.copy(sendHoldSecs = secs) }
        perform {
            try {
                container.core.setSendHoldSecs(secs)
            } finally {
                withContext(kotlinx.coroutines.NonCancellable) { loadSendHold() }
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
    fun setMode(mode: AutopilotMode?, connectionId: String? = null, authenticator: Authenticator? = null) {
        // A bypass or an Overnight starts only through its own start, after the owner's check.
        if (AutopilotText.isTimed(mode)) return
        // Leaving Lockdown loosens: it goes only through the owner's check ([endLockdown]), from any control. Without
        // an authenticator to ask, nothing changes.
        if (leavesLockdown(mode, connectionId)) {
            if (authenticator != null) endLockdown(mode ?: AutopilotMode.MANUAL, authenticator, connectionId)
            return
        }
        // Turning on Auto for every AI guides first, until its setup is finished (Decisions v2, 01 §9.1).
        if (mode == AutopilotMode.AUTO && connectionId == null && modeOf(null) != AutopilotMode.AUTO) {
            guideOrApply()
            return
        }
        applyMode(mode, connectionId)
    }

    /** Auto with its setup finished is set at once; before that, the setup opens instead and nothing changes yet. */
    private fun guideOrApply() {
        viewModelScope.launch {
            val setup = try {
                container.core.autoSetup()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Nothing to guide with: Auto is set as before.
                null
            }
            if (setup != null && !setup.done) {
                container.feedback.play(Event.Open)
                _ui.update { it.copy(setup = setup, setupRequested = true) }
            } else {
                applyMode(AutopilotMode.AUTO, null)
            }
        }
    }

    /** Whether setting [mode] would take [connectionId] (or every AI) out of a Lockdown in force. */
    fun leavesLockdown(mode: AutopilotMode?, connectionId: String? = null): Boolean =
        modeOf(connectionId) == AutopilotMode.LOCKDOWN && mode != AutopilotMode.LOCKDOWN

    /** Sets the mode with no check: only for what [setMode] allows and for [endLockdown] after the owner's pass. */
    private fun applyMode(mode: AutopilotMode?, connectionId: String?) {
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
     * How long what Overnight parks waits past the morning before it expires, not approved. It never loosens
     * anything, so nothing is asked; the choice shows at once.
     */
    fun setOvernightExpiry(hours: UInt) {
        if (settings.value?.overnightExpiryHours == hours) return
        draft { it.copy(overnightExpiryHours = hours) }
        perform {
            container.core.setOvernightExpiry(hours)
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
     * Ends Lockdown for [mode] after the owner's check: it loosens, so it asks first, wherever it is ended from (the
     * Trust field, a Quick Settings tile opening Trust). Without a pass Lockdown stays.
     */
    fun endLockdown(mode: AutopilotMode, authenticator: Authenticator, connectionId: String? = null) {
        viewModelScope.launch {
            val result = try {
                authenticator.authenticate("End lockdown", "Requests can be answered again")
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                AuthResult.Cancelled
            }
            when (result) {
                AuthResult.Success -> applyMode(mode, connectionId)
                AuthResult.Cancelled -> Unit
                AuthResult.Unavailable -> _ui.update { it.copy(error = "Set a screen lock or fingerprint on this phone to end Lockdown.") }
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

    // ---- Auto's setup (Decisions v2) ----------------------------------------------------------------------------

    /** The setup page is up: the request to open it is answered, and what it shows is read again. */
    fun showingSetup() {
        _ui.update { it.copy(setupRequested = false) }
        viewModelScope.launch { loadSetup() }
    }

    private suspend fun loadSetup() {
        try {
            val setup = container.core.autoSetup()
            _ui.update { it.copy(setup = setup) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _ui.update { it.copy(error = e.userMessage()) }
        }
    }

    /**
     * Whose [item] is. Shown at once; toward Mine the core asks the owner first ([authenticator]) and a refusal puts
     * the old zone back.
     */
    fun setWorldZone(item: WorldItem, zone: ZoneChoice, authenticator: Authenticator) {
        if (item.zone == zone && item.confirmed) return
        container.feedback.play(Event.Selection)
        _ui.update { ui ->
            ui.copy(setup = ui.setup?.let { s -> s.copy(world = s.world.map { if (it.kind == item.kind && it.key == item.key) it.copy(zone = zone, confirmed = true) else it }) })
        }
        val owner = OwnerPrompt(authenticator, "Auto")
        setupChange(owner) { container.core.setWorldZone(item.kind, item.key, zone, owner) }
    }

    /**
     * One cell of the matrix ([level] 0 to 3). Looser shows its preview first ([LoosenCell]: what it would have done on
     * this phone's own history), then asks the owner; stricter applies at once.
     */
    fun setMatrixCell(zone: ZoneChoice, level: Int, cell: CellChoice, authenticator: Authenticator) {
        val old = _ui.value.setup?.matrix?.firstOrNull { it.zone == zone }?.cells?.getOrNull(level)
        if (old != null && strictness(cell) < strictness(old)) {
            viewModelScope.launch {
                val preview = try {
                    container.core.previewPolicyChange(PolicyChange.SetCell(zone, level.toUByte(), cell)).line
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    ""
                }
                _ui.update { it.copy(loosen = LoosenCell(zone, level, cell, preview)) }
            }
            return
        }
        applyMatrixCell(zone, level, cell, authenticator)
    }

    /** The owner looked at the preview: go on (the core still asks for the owner's check). */
    fun confirmLoosen(authenticator: Authenticator) {
        val l = _ui.value.loosen ?: return
        _ui.update { it.copy(loosen = null) }
        applyMatrixCell(l.zone, l.level, l.cell, authenticator)
    }

    fun cancelLoosen() {
        _ui.update { it.copy(loosen = null) }
    }

    private fun strictness(c: CellChoice) = when (c) {
        CellChoice.AUTO -> 0
        CellChoice.ASK_FIRST -> 1
        CellChoice.ASK -> 2
    }

    private fun applyMatrixCell(zone: ZoneChoice, level: Int, cell: CellChoice, authenticator: Authenticator) {
        container.feedback.play(Event.Selection)
        _ui.update { ui ->
            ui.copy(
                setup = ui.setup?.let { s ->
                    s.copy(matrix = s.matrix.map { row -> if (row.zone != zone) row else row.copy(cells = row.cells.mapIndexed { l, c -> if (l == level) cell else c }) })
                },
            )
        }
        val owner = OwnerPrompt(authenticator, "Auto")
        setupChange(owner) { container.core.setMatrixCell(zone, level.toUByte(), cell, owner) }
    }

    /** "Turn on": confirms what the setup shows and turns Auto on, after the owner's check; then [onDone]. */
    fun finishSetup(authenticator: Authenticator, onDone: () -> Unit) {
        val owner = OwnerPrompt(authenticator, "Turn on Auto")
        val old = modeOf(null)
        setupChange(owner) {
            container.core.finishAutoSetup(owner)
            AutopilotText.modeChangeEvent(old, AutopilotMode.AUTO)?.let { container.feedback.play(it) }
            container.refreshAutopilot()
            onDone()
        }
    }

    /** Runs a setup change the core may ask the owner about; whatever happens, the setup is read again. */
    private fun setupChange(owner: OwnerPrompt, block: suspend () -> Unit) {
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
                    AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone first."
                    else -> e.userMessage()
                }
                if (error != null && owner.last != AuthResult.Unavailable) container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = error) }
            }
            loadSetup()
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

    /** "Update automatically" (on Wi-Fi, quietly). */
    fun setAutoUpdate(on: Boolean) {
        draft { it.copy(autoUpdate = on) }
        perform {
            container.core.setAutopilotAutoUpdate(on)
            container.refreshAutopilot()
        }
    }

    private var checkJob: Job? = null

    /**
     * The user's "Check now": looks for a newer model and, when there is one, downloads, checks and puts it in use.
     * Battery Saver and Data Saver do not stop it; without the network "Download on Wi-Fi only" allows, it waits for
     * Wi-Fi in the background instead.
     */
    fun checkNow() {
        if (checkJob?.isActive == true) return
        _ui.update { it.copy(checking = true, checkResult = null, error = null) }
        checkJob = viewModelScope.launch {
            val wifiOnly = settings.value?.wifiOnly ?: true
            val conditions = withContext(Dispatchers.IO) { container.networkConditions() }
            if (!ModelUpdateGate.checkNowAllows(conditions, wifiOnly)) {
                withContext(Dispatchers.IO) { container.modelUpdates.checkOnWifi(wifiOnly) }
                _ui.update { it.copy(checking = false, checkResult = CheckNow.WAITING) }
                return@launch
            }
            // The bytes show on the model's card while the newer model comes.
            val watch = launch {
                while (true) {
                    delay(POLL_MS)
                    try {
                        refreshModel()
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                    }
                }
            }
            val result = try {
                CheckNow.outcome(container.core.updateModel(automatic = false, progress = NoProgress))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // A newer model that failed its check, or one already on its way: the core's words, as a sentence.
                val refused = e is dev.reins.core.CoreException.Service || e is dev.reins.core.CoreException.Invalid
                if (refused) AutopilotText.modelError(e.userMessage()) else e.userMessage()
            } finally {
                watch.cancel()
            }
            try {
                refreshModel()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
            }
            container.refreshAutopilot()
            _ui.update { it.copy(checking = false, checkResult = result) }
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
            // Only a download the user started sounds when it ends; an automatic update stays quiet.
            var started = false
            do {
                if (_ui.value.job != DownloadJob.Idle) started = true
                try {
                    refreshModel()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                }
                delay(POLL_MS)
                if (_ui.value.job != DownloadJob.Idle) started = true
            } while (_ui.value.job != DownloadJob.Idle || _ui.value.model?.state == ModelState.DOWNLOADING)
            val finished = _ui.value.model?.state
            if (started && finished == ModelState.INSTALLED) container.feedback.play(Event.GrantCreated)
            if (started && finished == ModelState.FAILED) container.feedback.play(Event.Error)
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

    /** [locked]: true always asks, false trusts the kind now (no cautious start, no pause), null lets Autopilot decide. */
    fun setClassLock(id: String, classKey: String, locked: Boolean?) {
        // The chips sound their own choice; trusting (after its warning) is felt as more autonomy.
        if (locked == false) container.feedback.play(Event.AutopilotOn)
        editProfile(id) { p ->
            p.copy(
                classes = p.classes.map {
                    if (it.classKey != classKey) it
                    else it.copy(manual = locked?.not(), autoApprove = locked?.not() ?: it.autoApprove, paused = locked == null && it.paused)
                },
            )
        }
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
