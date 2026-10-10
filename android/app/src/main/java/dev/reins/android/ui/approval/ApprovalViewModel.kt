package dev.reins.android.ui.approval

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.SuggestionView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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
    /** What Autopilot made of the request (Assisted and Auto), if it looked at it. */
    val suggestion: SuggestionView? = null,
)

class ApprovalViewModel(private val container: AppContainer, private val requestId: String) : ViewModel() {
    private val _ui = MutableStateFlow(ApprovalUi())
    val ui: StateFlow<ApprovalUi> = _ui.asStateFlow()

    /** The user changed the draft (a tick, a period): a fresh read of the request keeps it. */
    @Volatile private var touched = false

    init {
        // Read ahead when the request arrived: the sheet opens with it, no spinner.
        container.approvalViews[requestId]?.let { _ui.value = opened(it) }
        viewModelScope.launch {
            try {
                val view = container.core.approvalView(requestId)
                // The fresh copy wins (repeats may have changed), without undoing what the user already changed.
                _ui.update { if (touched || it.busy) it.copy(view = view) else opened(view).copy(moreOpen = it.moreOpen) }
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
        return ApprovalUi(loading = false, view = view, draft = ApprovalDraft(selected = preselected, lifetime = lifetime, resources = defaultResources(view), classes = defaultClasses(view)))
    }

    /** Autopilot's suggestion is a hint: without one (Manual, no model, a failure) the screen is simply as before. */
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

    fun edit(change: (ApprovalDraft) -> ApprovalDraft) {
        touched = true
        _ui.update { it.copy(draft = change(it.draft), error = null) }
    }

    fun toggleMore() {
        _ui.update { it.copy(moreOpen = !it.moreOpen) }
    }

    /**
     * Approving needs the phone's own screen lock or biometrics first; anything but success does nothing. With [allow],
     * the core's "allow for a while" permission (same AI, same kind of request, same target) is made alongside, in
     * place of whatever "More options" says.
     */
    fun approve(authenticator: Authenticator, allow: Boolean = false) {
        val current = _ui.value
        val view = current.view ?: return
        if (current.busy) return
        val quickAllow = view.quick?.allow
        if (allow && quickAllow == null) return
        val choice = when (val built = buildChoice(view, current.draft)) {
            is BuildResult.Invalid -> {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(error = built.message) }
                return
            }
            is BuildResult.Ok -> if (allow) built.choice.copy(standing = quickAllow) else built.choice
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
        if (_ui.value.busy) return
        container.feedback.play(Event.Denied)
        container.answerInBackground(requestId, "Not denied") { container.core.deny(requestId) }
        _ui.update { it.copy(finished = true) }
    }
}
