package dev.rewarden.android.ui.approval

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.rewarden.android.AppContainer
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.play
import dev.rewarden.android.platform.AuthResult
import dev.rewarden.android.platform.Authenticator
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.ApprovalKind
import dev.rewarden.core.ApprovalView
import dev.rewarden.core.SuggestionView
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

    init {
        viewModelScope.launch {
            try {
                val view = container.core.approvalView(requestId)
                // Everything found starts ticked: approving is one tap, and unticking is how you hold something back.
                val preselected = when (view.kind) {
                    ApprovalKind.GRANT, ApprovalKind.SEND, ApprovalKind.WRITE -> emptySet()
                    ApprovalKind.ACCOUNTS -> shareableAccounts(view).toSet()
                    // A code or a password is never ticked for the user.
                    ApprovalKind.FETCH -> view.messages.filter { !it.sensitive }.map { it.id }.toSet()
                    else -> view.messages.map { it.id }.toSet()
                }
                // Showing the accounts is remembered for a month unless the user says otherwise.
                val lifetime = if (view.kind == ApprovalKind.ACCOUNTS) LifetimeKind.MONTH else LifetimeKind.ONCE
                _ui.value = ApprovalUi(loading = false, view = view, draft = ApprovalDraft(selected = preselected, lifetime = lifetime, resources = defaultResources(view), classes = defaultClasses(view)))
                loadSuggestion()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.value = ApprovalUi(loading = false, error = e.userMessage())
            }
        }
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
        _ui.update { it.copy(draft = change(it.draft), error = null) }
    }

    fun toggleMore() {
        _ui.update { it.copy(moreOpen = !it.moreOpen) }
    }

    /** Approving needs the phone's own screen lock or biometrics first; anything but success does nothing. */
    fun approve(authenticator: Authenticator) {
        val current = _ui.value
        val view = current.view ?: return
        if (current.busy) return
        val choice = when (val built = buildChoice(view, current.draft)) {
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
                        container.core.approve(requestId, choice)
                        container.refreshPending()
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

    fun deny() {
        if (_ui.value.busy) return
        _ui.update { it.copy(busy = true, error = null) }
        container.feedback.play(Event.Denied)
        viewModelScope.launch {
            try {
                container.core.deny(requestId)
                container.refreshPending()
                _ui.update { it.copy(busy = false, finished = true) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.update { it.copy(busy = false, error = e.userMessage()) }
            }
        }
    }
}
